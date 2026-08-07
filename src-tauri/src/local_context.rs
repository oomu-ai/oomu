use crate::foundation::{
    clock::{unix_time_ms_i64 as unix_time_ms, unix_time_ns_from},
    digest::sha256_reader_bounded,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

mod native_drop;
#[cfg(test)]
mod native_drop_tests;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

const MAX_FILE_BYTES: u64 = 96 * 1024;
const MAX_CONTEXT_TEXT_BYTES: usize = 128 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 240;
const MAX_PICKER_FILES: usize = 5;
const MAX_PICKER_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PICKER_AGGREGATE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_LIVE_CONTEXT_GRANTS: usize = 32;
const MAX_LIVE_CONTEXT_FILE_GRANTS: usize = 24;
const MAX_LIVE_CONTEXT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PENDING_DROPS: usize = 8;
const MAX_IMAGE_DIMENSION: u32 = 8_192;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;
const IMAGE_HEADER_INSPECTION_BYTES: u64 = 1024 * 1024;
const GRANT_TTL_MS: i64 = 5 * 60 * 1000;
const DROP_TTL_MS: i64 = 30 * 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    length: u64,
    modified_ns: u128,
}

impl FileIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Result<Self, String> {
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(unix_time_ns_from)
            .unwrap_or_default();
        Ok(Self {
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            length: metadata.len(),
            modified_ns,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GrantOperation {
    Read,
    List,
}

struct LocalContextGrant {
    path: PathBuf,
    handle: fs::File,
    identity: FileIdentity,
    operation: GrantOperation,
    session_id: String,
    turn_id: String,
    expires_at_ms: i64,
    retained_bytes: u64,
    content_sha256: Option<[u8; 32]>,
}

struct ConsumedLocalContextGrant {
    path: PathBuf,
    handle: fs::File,
}

#[derive(Clone, Debug, Default)]
struct TurnGrantBudget {
    count: usize,
    decoded_bytes: u64,
    expires_at_ms: i64,
}

struct PendingLocalContextDrop {
    paths: Vec<PathBuf>,
    expires_at_ms: i64,
    sequence: u64,
}

#[derive(Default)]
struct LocalContextGrantState {
    grants: HashMap<String, LocalContextGrant>,
    budgets: HashMap<(String, String), TurnGrantBudget>,
    pending_drops: HashMap<String, PendingLocalContextDrop>,
    next_drop_sequence: u64,
}

#[derive(Clone, Default)]
pub struct LocalContextGrantStore {
    state: Arc<Mutex<LocalContextGrantState>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChooseLocalContextRequest {
    pub session_id: String,
    pub turn_id: String,
    #[serde(default = "default_picker_operation")]
    pub operation: String,
}

fn default_picker_operation() -> String {
    "read".to_string()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickerGrantResult {
    pub name: String,
    pub ok: bool,
    pub grant_id: Option<String>,
    pub mime_type: String,
    pub decoded_byte_count: u64,
    pub encoded_byte_count: u64,
    pub expires_at_ms: Option<i64>,
    pub error_code: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChooseLocalContextResponse {
    pub results: Vec<PickerGrantResult>,
    pub count_limit: usize,
    pub decoded_byte_limit: u64,
    pub encoded_byte_limit: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimDroppedLocalContextRequest {
    pub drop_id: String,
    pub session_id: String,
    pub turn_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimLatestDroppedLocalContextRequest {
    pub session_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalContextRequest {
    pub grant_id: String,
    pub session_id: String,
    pub turn_id: String,
}

pub type ListLocalDirectoryRequest = LocalContextRequest;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeLocalContextGrantsRequest {
    pub session_id: String,
    pub turn_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeLocalContextGrantsResponse {
    pub revoked_count: usize,
}

#[derive(Debug, Serialize)]
pub struct LocalContextResponse {
    pub name: String,
    pub mime_type: String,
    pub byte_count: u64,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct ListLocalDirectoryResponse {
    pub entries: Vec<String>,
    pub text: String,
    pub truncated: bool,
}

#[tauri::command]
pub async fn choose_local_context(
    request: ChooseLocalContextRequest,
    grants: tauri::State<'_, LocalContextGrantStore>,
) -> Result<ChooseLocalContextResponse, String> {
    let operation = parse_operation(&request.operation)?;
    validate_scope(&request.session_id, &request.turn_id)?;
    let paths = if operation == GrantOperation::List {
        rfd::AsyncFileDialog::new()
            .pick_folder()
            .await
            .map(|handle| vec![handle.path().to_path_buf()])
            .unwrap_or_default()
    } else {
        rfd::AsyncFileDialog::new()
            .pick_files()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|handle| handle.path().to_path_buf())
            .collect()
    };
    Ok(issue_grants_for_paths(
        grants.inner(),
        paths,
        operation,
        &request.session_id,
        &request.turn_id,
    ))
}

pub(crate) fn register_dropped_local_context(
    store: &LocalContextGrantStore,
    paths: &[PathBuf],
) -> Result<Option<String>, String> {
    if paths.is_empty() {
        return Ok(None);
    }
    let now = unix_time_ms();
    let mut state = store
        .state
        .lock()
        .map_err(|_| "local_context_grant_store_unavailable".to_string())?;
    state
        .pending_drops
        .retain(|_, pending| pending.expires_at_ms > now);
    while state.pending_drops.len() >= MAX_PENDING_DROPS {
        let Some(oldest) = state
            .pending_drops
            .iter()
            .min_by_key(|(_, pending)| pending.expires_at_ms)
            .map(|(drop_id, _)| drop_id.clone())
        else {
            break;
        };
        state.pending_drops.remove(&oldest);
    }
    let drop_id = random_grant_id();
    state.next_drop_sequence = state.next_drop_sequence.saturating_add(1);
    let sequence = state.next_drop_sequence;
    state.pending_drops.insert(
        drop_id.clone(),
        PendingLocalContextDrop {
            paths: paths.iter().take(MAX_PICKER_FILES + 1).cloned().collect(),
            expires_at_ms: now.saturating_add(DROP_TTL_MS),
            sequence,
        },
    );
    Ok(Some(drop_id))
}

#[tauri::command]
pub fn claim_latest_dropped_local_context(
    request: ClaimLatestDroppedLocalContextRequest,
    grants: tauri::State<'_, LocalContextGrantStore>,
) -> Result<ChooseLocalContextResponse, String> {
    native_drop::claim_latest_dropped_local_context_with_store(grants.inner(), request)
}

#[tauri::command]
pub fn claim_dropped_local_context(
    request: ClaimDroppedLocalContextRequest,
    grants: tauri::State<'_, LocalContextGrantStore>,
) -> Result<ChooseLocalContextResponse, String> {
    claim_dropped_local_context_with_store(grants.inner(), request)
}

fn claim_dropped_local_context_with_store(
    store: &LocalContextGrantStore,
    request: ClaimDroppedLocalContextRequest,
) -> Result<ChooseLocalContextResponse, String> {
    validate_scope(&request.session_id, &request.turn_id)?;
    if request.drop_id.len() != 64 || !request.drop_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("local_context_drop_invalid_or_expired".to_string());
    }
    let paths = {
        let now = unix_time_ms();
        let mut state = store
            .state
            .lock()
            .map_err(|_| "local_context_grant_store_unavailable".to_string())?;
        state
            .pending_drops
            .retain(|_, pending| pending.expires_at_ms > now);
        state
            .pending_drops
            .remove(&request.drop_id)
            .map(|pending| pending.paths)
            .ok_or_else(|| "local_context_drop_invalid_or_expired".to_string())?
    };
    Ok(issue_grants_for_paths(
        store,
        paths,
        GrantOperation::Read,
        &request.session_id,
        &request.turn_id,
    ))
}

#[tauri::command]
pub fn revoke_local_context_grants(
    request: RevokeLocalContextGrantsRequest,
    grants: tauri::State<'_, LocalContextGrantStore>,
) -> Result<RevokeLocalContextGrantsResponse, String> {
    validate_scope(
        &request.session_id,
        request.turn_id.as_deref().unwrap_or("revoke-all"),
    )?;
    let mut state = grants
        .state
        .lock()
        .map_err(|_| "local_context_grant_store_unavailable".to_string())?;
    let before = state.grants.len();
    state.grants.retain(|_, grant| {
        grant.session_id != request.session_id
            || request
                .turn_id
                .as_deref()
                .is_some_and(|turn_id| grant.turn_id != turn_id)
    });
    state.budgets.retain(|(session_id, turn_id), _| {
        session_id != &request.session_id
            || request
                .turn_id
                .as_deref()
                .is_some_and(|requested_turn| turn_id != requested_turn)
    });
    Ok(RevokeLocalContextGrantsResponse {
        revoked_count: before.saturating_sub(state.grants.len()),
    })
}

#[tauri::command]
pub fn read_local_context(
    request: LocalContextRequest,
    grants: tauri::State<'_, LocalContextGrantStore>,
) -> Result<LocalContextResponse, String> {
    read_local_context_with_store(grants.inner(), request)
}

fn read_local_context_with_store(
    grants: &LocalContextGrantStore,
    request: LocalContextRequest,
) -> Result<LocalContextResponse, String> {
    let consumed = consume_grant(grants, &request, GrantOperation::Read)?;
    let metadata = consumed
        .handle
        .metadata()
        .map_err(|_| "local_context_file_unavailable".to_string())?;
    let name = consumed
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("selected-context")
        .to_string();

    if !metadata.is_file() {
        return Err("local_context_grant_type_mismatch".to_string());
    }

    let parsed = read_file_context(&consumed.path, metadata.len(), consumed.handle)?;

    Ok(LocalContextResponse {
        name,
        mime_type: parsed.mime_type,
        byte_count: metadata.len(),
        text: parsed.text,
        truncated: parsed.truncated,
    })
}

#[tauri::command]
pub fn list_local_directory(
    request: ListLocalDirectoryRequest,
    grants: tauri::State<'_, LocalContextGrantStore>,
) -> Result<ListLocalDirectoryResponse, String> {
    list_local_directory_with_store(grants.inner(), request)
}

fn list_local_directory_with_store(
    grants: &LocalContextGrantStore,
    request: ListLocalDirectoryRequest,
) -> Result<ListLocalDirectoryResponse, String> {
    let consumed = consume_grant(grants, &request, GrantOperation::List)?;
    let metadata = consumed
        .handle
        .metadata()
        .map_err(|_| "local_context_directory_unavailable".to_string())?;
    if !metadata.is_dir() {
        return Err("Local directory path is not a directory.".to_string());
    }

    #[cfg(unix)]
    let mut entries = list_directory_from_handle(&consumed.handle)?;
    #[cfg(not(unix))]
    let mut entries = list_directory_from_path(&consumed.path)?;
    entries.sort();
    let truncated = entries.len() > MAX_DIRECTORY_ENTRIES;
    if truncated {
        entries.truncate(MAX_DIRECTORY_ENTRIES);
    }
    let text = if entries.is_empty() {
        "(directory is empty)".to_string()
    } else {
        entries.join("\n")
    };

    Ok(ListLocalDirectoryResponse {
        entries,
        text,
        truncated,
    })
}

#[cfg(unix)]
fn list_directory_from_handle(handle: &fs::File) -> Result<Vec<String>, String> {
    use std::{ffi::CStr, mem::MaybeUninit};

    struct DirectoryHandle(*mut libc::DIR);
    impl Drop for DirectoryHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: fdopendir returned this DIR pointer and this guard is
                // its sole owner. closedir also closes the duplicated fd.
                unsafe { libc::closedir(self.0) };
            }
        }
    }

    // fdopendir takes ownership of its fd, so duplicate the retained picker
    // handle. Enumerating through the descriptor prevents a rename/swap race
    // between grant revalidation and directory traversal.
    let duplicated_fd = unsafe { libc::dup(handle.as_raw_fd()) };
    if duplicated_fd < 0 {
        return Err("local_context_directory_unavailable".to_string());
    }
    let directory = unsafe { libc::fdopendir(duplicated_fd) };
    if directory.is_null() {
        unsafe { libc::close(duplicated_fd) };
        return Err("local_context_directory_unavailable".to_string());
    }
    let directory = DirectoryHandle(directory);
    let directory_fd = unsafe { libc::dirfd(directory.0) };
    if directory_fd < 0 {
        return Err("local_context_directory_unavailable".to_string());
    }

    let mut entries = Vec::with_capacity(MAX_DIRECTORY_ENTRIES + 1);
    while entries.len() <= MAX_DIRECTORY_ENTRIES {
        // SAFETY: the DIR pointer remains live for this loop and readdir's
        // returned entry is consumed before the next readdir call.
        let entry = unsafe { libc::readdir(directory.0) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        let name_bytes = name.to_bytes();
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }

        let mut stat = MaybeUninit::<libc::stat>::uninit();
        let status = unsafe {
            libc::fstatat(
                directory_fd,
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if status != 0 {
            return Err("local_context_directory_entry_unavailable".to_string());
        }
        let mode = unsafe { stat.assume_init() }.st_mode;
        if mode & libc::S_IFMT == libc::S_IFLNK {
            continue;
        }
        let mut display_name = String::from_utf8_lossy(name_bytes).to_string();
        if mode & libc::S_IFMT == libc::S_IFDIR {
            display_name.push('/');
        }
        entries.push(display_name);
    }
    Ok(entries)
}

#[cfg(not(unix))]
fn list_directory_from_path(path: &Path) -> Result<Vec<String>, String> {
    let directory_entries =
        fs::read_dir(path).map_err(|_| "local_context_directory_unavailable".to_string())?;
    let mut entries = Vec::new();
    for entry in directory_entries.take(MAX_DIRECTORY_ENTRIES + 1) {
        let entry = entry.map_err(|_| "local_context_directory_entry_unavailable".to_string())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "local_context_directory_entry_unavailable".to_string())?;
        if file_type.is_symlink() {
            continue;
        }
        let mut name = entry.file_name().to_string_lossy().to_string();
        if file_type.is_dir() {
            name.push('/');
        }
        entries.push(name);
    }
    Ok(entries)
}

fn parse_operation(value: &str) -> Result<GrantOperation, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "read" => Ok(GrantOperation::Read),
        "list" => Ok(GrantOperation::List),
        _ => Err("local_context_operation_invalid".to_string()),
    }
}

fn validate_scope(session_id: &str, turn_id: &str) -> Result<(), String> {
    if session_id.trim().is_empty()
        || turn_id.trim().is_empty()
        || session_id.len() > 256
        || turn_id.len() > 256
    {
        return Err("local_context_scope_invalid".to_string());
    }
    Ok(())
}

fn random_grant_id() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn selected_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("selected-context")
        .chars()
        .take(240)
        .collect()
}

fn rejected_grant(path: &Path, error_code: &'static str) -> PickerGrantResult {
    PickerGrantResult {
        name: selected_name(path),
        ok: false,
        grant_id: None,
        mime_type: "application/octet-stream".to_string(),
        decoded_byte_count: 0,
        encoded_byte_count: 0,
        expires_at_ms: None,
        error_code: Some(error_code),
    }
}

fn issue_grants_for_paths(
    store: &LocalContextGrantStore,
    paths: Vec<PathBuf>,
    operation: GrantOperation,
    session_id: &str,
    turn_id: &str,
) -> ChooseLocalContextResponse {
    let now = unix_time_ms();
    let expires_at_ms = now.saturating_add(GRANT_TTL_MS);
    let mut selected_paths = paths.into_iter();
    let bounded_paths = selected_paths
        .by_ref()
        .take(MAX_PICKER_FILES)
        .collect::<Vec<_>>();
    let overflow_path = selected_paths.next();
    let mut results = Vec::with_capacity(MAX_PICKER_FILES + usize::from(overflow_path.is_some()));
    let mut state = match store.state.lock() {
        Ok(state) => state,
        Err(_) => {
            let mut rejected = bounded_paths
                .iter()
                .map(|path| rejected_grant(path, "local_context_grant_store_unavailable"))
                .collect::<Vec<_>>();
            if let Some(path) = overflow_path.as_deref() {
                rejected.push(rejected_grant(path, "attachment_count_limit_exceeded"));
            }
            return ChooseLocalContextResponse {
                results: rejected,
                count_limit: MAX_PICKER_FILES,
                decoded_byte_limit: MAX_PICKER_AGGREGATE_BYTES,
                encoded_byte_limit: MAX_PICKER_AGGREGATE_BYTES.saturating_mul(4) / 3 + 4,
            };
        }
    };
    state.grants.retain(|_, grant| grant.expires_at_ms > now);
    state.budgets.retain(|_, budget| budget.expires_at_ms > now);
    let mut live_grant_count = state.grants.len();
    let mut live_file_count = state
        .grants
        .values()
        .filter(|grant| grant.operation == GrantOperation::Read)
        .count();
    let mut live_byte_count = state
        .grants
        .values()
        .map(|grant| grant.retained_bytes)
        .sum::<u64>();
    let budget_key = (session_id.to_string(), turn_id.to_string());
    let mut budget = state.budgets.remove(&budget_key).unwrap_or_default();

    for selected_path in bounded_paths {
        if budget.count >= MAX_PICKER_FILES {
            results.push(rejected_grant(
                &selected_path,
                "attachment_count_limit_exceeded",
            ));
            continue;
        }
        let selected_metadata = match fs::symlink_metadata(&selected_path) {
            Ok(metadata) if !metadata.file_type().is_symlink() => metadata,
            _ => {
                results.push(rejected_grant(
                    &selected_path,
                    "local_context_symlink_rejected",
                ));
                continue;
            }
        };
        let canonical_path = match fs::canonicalize(&selected_path) {
            Ok(path) => path,
            Err(_) => {
                results.push(rejected_grant(
                    &selected_path,
                    "local_context_selection_unavailable",
                ));
                continue;
            }
        };
        let mut handle = match fs::File::open(&canonical_path) {
            Ok(handle) => handle,
            Err(_) => {
                results.push(rejected_grant(
                    &selected_path,
                    "local_context_selection_unavailable",
                ));
                continue;
            }
        };
        let metadata = match handle.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                results.push(rejected_grant(
                    &selected_path,
                    "local_context_selection_unavailable",
                ));
                continue;
            }
        };
        let selected_identity = match FileIdentity::from_metadata(&selected_metadata) {
            Ok(identity) => identity,
            Err(_) => {
                results.push(rejected_grant(
                    &selected_path,
                    "local_context_identity_unavailable",
                ));
                continue;
            }
        };
        let opened_identity = match FileIdentity::from_metadata(&metadata) {
            Ok(identity) => identity,
            Err(_) => {
                results.push(rejected_grant(
                    &selected_path,
                    "local_context_identity_unavailable",
                ));
                continue;
            }
        };
        if selected_identity != opened_identity {
            results.push(rejected_grant(&selected_path, "local_context_grant_stale"));
            continue;
        }
        let type_matches = match operation {
            GrantOperation::Read => metadata.is_file() && selected_metadata.is_file(),
            GrantOperation::List => metadata.is_dir() && selected_metadata.is_dir(),
        };
        if !type_matches {
            results.push(rejected_grant(
                &selected_path,
                "local_context_grant_type_mismatch",
            ));
            continue;
        }
        let byte_count = if metadata.is_file() {
            metadata.len()
        } else {
            0
        };
        if byte_count > MAX_PICKER_FILE_BYTES {
            results.push(rejected_grant(
                &selected_path,
                "attachment_file_byte_limit_exceeded",
            ));
            continue;
        }
        if budget.decoded_bytes.saturating_add(byte_count) > MAX_PICKER_AGGREGATE_BYTES {
            results.push(rejected_grant(
                &selected_path,
                "attachment_aggregate_byte_limit_exceeded",
            ));
            continue;
        }
        let is_file_grant = operation == GrantOperation::Read;
        if live_grant_count >= MAX_LIVE_CONTEXT_GRANTS
            || (is_file_grant && live_file_count >= MAX_LIVE_CONTEXT_FILE_GRANTS)
            || live_byte_count.saturating_add(byte_count) > MAX_LIVE_CONTEXT_BYTES
        {
            results.push(rejected_grant(
                &selected_path,
                "local_context_global_capacity_exceeded",
            ));
            continue;
        }
        if metadata.is_file() {
            if let Err(error_code) = validate_image_dimensions(&canonical_path, &mut handle) {
                results.push(rejected_grant(&selected_path, error_code));
                continue;
            }
        }
        let identity = opened_identity;
        let content_sha256 = if metadata.is_file() {
            match sha256_open_file(&mut handle) {
                Ok(digest) => Some(digest),
                Err(_) => {
                    results.push(rejected_grant(
                        &selected_path,
                        "local_context_identity_unavailable",
                    ));
                    continue;
                }
            }
        } else {
            None
        };
        budget.count = budget.count.saturating_add(1);
        budget.decoded_bytes = budget.decoded_bytes.saturating_add(byte_count);
        budget.expires_at_ms = expires_at_ms;
        let grant_id = random_grant_id();
        let mime_type = if metadata.is_dir() {
            "text/x-directory-context".to_string()
        } else {
            mime_type_for_path(&canonical_path)
        };
        state.grants.insert(
            grant_id.clone(),
            LocalContextGrant {
                path: canonical_path,
                handle,
                identity,
                operation,
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
                expires_at_ms,
                retained_bytes: byte_count,
                content_sha256,
            },
        );
        live_grant_count = live_grant_count.saturating_add(1);
        if is_file_grant {
            live_file_count = live_file_count.saturating_add(1);
        }
        live_byte_count = live_byte_count.saturating_add(byte_count);
        results.push(PickerGrantResult {
            name: selected_name(&selected_path),
            ok: true,
            grant_id: Some(grant_id),
            mime_type,
            decoded_byte_count: byte_count,
            // Local content remains native and is returned only as bounded text;
            // it is never base64-expanded into renderer memory.
            encoded_byte_count: 0,
            expires_at_ms: Some(expires_at_ms),
            error_code: None,
        });
    }
    if let Some(path) = overflow_path.as_deref() {
        results.push(rejected_grant(path, "attachment_count_limit_exceeded"));
    }

    if budget.count > 0 {
        state.budgets.insert(budget_key, budget);
    }

    ChooseLocalContextResponse {
        results,
        count_limit: MAX_PICKER_FILES,
        decoded_byte_limit: MAX_PICKER_AGGREGATE_BYTES,
        encoded_byte_limit: MAX_PICKER_AGGREGATE_BYTES.saturating_mul(4) / 3 + 4,
    }
}

fn consume_grant(
    store: &LocalContextGrantStore,
    request: &LocalContextRequest,
    operation: GrantOperation,
) -> Result<ConsumedLocalContextGrant, String> {
    validate_scope(&request.session_id, &request.turn_id)?;
    if request.grant_id.len() != 64
        || !request
            .grant_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("local_context_grant_invalid".to_string());
    }
    let now = unix_time_ms();
    let mut grant = {
        let mut state = store
            .state
            .lock()
            .map_err(|_| "local_context_grant_store_unavailable".to_string())?;
        state.grants.retain(|_, grant| grant.expires_at_ms > now);
        state
            .grants
            .remove(&request.grant_id)
            .ok_or_else(|| "local_context_grant_invalid_or_expired".to_string())?
    };
    if grant.operation != operation
        || grant.session_id != request.session_id
        || grant.turn_id != request.turn_id
    {
        return Err("local_context_grant_scope_mismatch".to_string());
    }
    let link_metadata =
        fs::symlink_metadata(&grant.path).map_err(|_| "local_context_grant_stale".to_string())?;
    if link_metadata.file_type().is_symlink() {
        return Err("local_context_grant_stale".to_string());
    }
    let current_path =
        fs::canonicalize(&grant.path).map_err(|_| "local_context_grant_stale".to_string())?;
    if current_path != grant.path {
        return Err("local_context_grant_stale".to_string());
    }
    let metadata =
        fs::metadata(&current_path).map_err(|_| "local_context_grant_stale".to_string())?;
    if FileIdentity::from_metadata(&metadata)? != grant.identity {
        return Err("local_context_grant_stale".to_string());
    }
    let handle_metadata = grant
        .handle
        .metadata()
        .map_err(|_| "local_context_grant_stale".to_string())?;
    if FileIdentity::from_metadata(&handle_metadata)? != grant.identity {
        return Err("local_context_grant_stale".to_string());
    }
    if let Some(expected_digest) = grant.content_sha256 {
        if sha256_open_file(&mut grant.handle)? != expected_digest {
            return Err("local_context_grant_stale".to_string());
        }
    }
    Ok(ConsumedLocalContextGrant {
        path: current_path,
        handle: grant.handle,
    })
}

fn sha256_open_file(file: &mut fs::File) -> Result<[u8; 32], String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "local_context_identity_unavailable".to_string())?;
    let digest = sha256_reader_bounded(file.by_ref(), MAX_PICKER_FILE_BYTES)
        .map_err(|_| "local_context_identity_unavailable".to_string())?
        .ok_or_else(|| "attachment_file_byte_limit_exceeded".to_string())?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "local_context_identity_unavailable".to_string())?;
    Ok(*digest.as_bytes())
}

fn validate_image_dimensions(path: &Path, file: &mut fs::File) -> Result<(), &'static str> {
    // PDFs are rendered only after the bounded native grant; they are not raster images.
    if is_pdf_path(path) || !crate::tools::vision::is_supported_visual_artifact_path(path) {
        return Ok(());
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "attachment_image_header_unavailable")?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(IMAGE_HEADER_INSPECTION_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|_| "attachment_image_header_unavailable")?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "attachment_image_header_unavailable")?;
    let (width, height) =
        image_dimensions(&bytes).ok_or("attachment_image_dimensions_unavailable")?;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || u64::from(width).saturating_mul(u64::from(height)) > MAX_IMAGE_PIXELS
    {
        return Err("attachment_image_dimension_limit_exceeded");
    }
    Ok(())
}

fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((
            u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]),
            u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
        ));
    }
    if bytes.len() >= 10 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Some((
            u16::from_le_bytes([bytes[6], bytes[7]]) as u32,
            u16::from_le_bytes([bytes[8], bytes[9]]) as u32,
        ));
    }
    if bytes.len() >= 26 && bytes.starts_with(b"BM") {
        return Some((
            u32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]),
            u32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]),
        ));
    }
    if bytes.len() >= 30 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        if &bytes[12..16] == b"VP8X" {
            let width = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
            let height = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
            return Some((width, height));
        }
    }
    if bytes.len() > 4 && bytes.starts_with(&[0xff, 0xd8]) {
        let mut index = 2;
        while index + 9 < bytes.len() {
            if bytes[index] != 0xff {
                index += 1;
                continue;
            }
            let marker = bytes[index + 1];
            let length = u16::from_be_bytes([bytes[index + 2], bytes[index + 3]]) as usize;
            if matches!(marker, 0xc0 | 0xc1 | 0xc2) && index + 8 < bytes.len() {
                let height = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]) as u32;
                let width = u16::from_be_bytes([bytes[index + 7], bytes[index + 8]]) as u32;
                return Some((width, height));
            }
            if length < 2 {
                break;
            }
            index = index.saturating_add(2 + length);
        }
    }
    None
}

struct ParsedFileContext {
    mime_type: String,
    text: String,
    truncated: bool,
}

fn read_file_context(
    path: &Path,
    byte_count: u64,
    mut file: fs::File,
) -> Result<ParsedFileContext, String> {
    if is_pdf_path(path) {
        return read_pdf_context(file);
    }

    if crate::tools::vision::is_supported_visual_artifact_path(path) {
        let mut bytes = Vec::with_capacity(byte_count as usize);
        file.by_ref()
            .take(MAX_PICKER_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "local_context_file_read_failed".to_string())?;
        if bytes.len() as u64 > MAX_PICKER_FILE_BYTES {
            return Err("attachment_file_byte_limit_exceeded".to_string());
        }
        let visual_context = crate::tools::vision::analyze_visual_bytes_for_context(path, bytes)?;
        return Ok(ParsedFileContext {
            mime_type: visual_context.mime_type,
            text: visual_context.text,
            truncated: visual_context.truncated,
        });
    }

    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Unable to read local context file: {error}"))?;
    let mut truncated = bytes.len() as u64 > MAX_FILE_BYTES;
    if truncated {
        bytes.truncate(MAX_FILE_BYTES as usize);
    }

    if bytes.iter().any(|byte| *byte == 0) {
        return Ok(binary_context(path, byte_count));
    }

    let text = match String::from_utf8(bytes) {
        Ok(value) => value,
        Err(_) => return Ok(binary_context(path, byte_count)),
    };
    let (text, budget_truncated) = truncate_text_at_boundary(&text, MAX_CONTEXT_TEXT_BYTES);
    truncated |= budget_truncated;

    Ok(ParsedFileContext {
        mime_type: mime_type_for_path(path),
        text,
        truncated,
    })
}

fn read_pdf_context(file: fs::File) -> Result<ParsedFileContext, String> {
    let extraction =
        crate::pdf_containment::extract_pdf_from_open_file(file).map_err(|error| error.message)?;
    let (text, budget_truncated) =
        truncate_text_at_boundary(&extraction.text, MAX_CONTEXT_TEXT_BYTES);
    Ok(ParsedFileContext {
        mime_type: "application/pdf".to_string(),
        text,
        truncated: extraction.truncated || budget_truncated,
    })
}

fn binary_context(path: &Path, byte_count: u64) -> ParsedFileContext {
    ParsedFileContext {
        mime_type: mime_type_for_path(path),
        text: format!(
            "Binary local context preserved as metadata only.\nByte count: {byte_count}\nReason: no safe text parser is registered for this file type."
        ),
        truncated: false,
    }
}

fn is_pdf_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

fn mime_type_for_path(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        "json" | "jsonl" => "application/json",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "md" | "markdown" => "text/markdown",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/x-yaml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "jsx" => "text/javascript",
        "ts" | "tsx" => "application/typescript",
        "toml" | "rs" | "py" | "sh" | "sql" | "log" | "txt" => "text/plain",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn truncate_text_at_boundary(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_string(), false);
    }

    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = text[..end].to_string();
    truncated.push_str("\n[local context truncated]");
    (truncated, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_for_grant(
        result: &PickerGrantResult,
        session: &str,
        turn: &str,
    ) -> LocalContextRequest {
        LocalContextRequest {
            grant_id: result.grant_id.clone().expect("grant id"),
            session_id: session.to_string(),
            turn_id: turn.to_string(),
        }
    }
    #[test]
    fn picker_grant_reads_only_the_exact_selected_file_once() {
        let root = std::env::temp_dir().join(format!(
            "oomu-local-context-grant-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let selected = root.join("selected.txt");
        fs::write(&selected, "approved content").unwrap();
        fs::write(root.join("not-selected.txt"), "private canary").unwrap();
        let store = LocalContextGrantStore::default();
        let issued = issue_grants_for_paths(
            &store,
            vec![selected],
            GrantOperation::Read,
            "session-a",
            "turn-a",
        );
        assert_eq!(issued.results.len(), 1);
        assert!(issued.results[0].ok);
        let request = request_for_grant(&issued.results[0], "session-a", "turn-a");
        let response = read_local_context_with_store(&store, request.clone()).unwrap();
        assert_eq!(response.text, "approved content");
        assert!(!response.text.contains("private canary"));
        assert_eq!(
            read_local_context_with_store(&store, request).unwrap_err(),
            "local_context_grant_invalid_or_expired"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dropped_file_receipt_is_opaque_one_use_and_uses_picker_grants() {
        let root = std::env::temp_dir().join(format!(
            "oomu-local-context-drop-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let selected = root.join("finder-note.txt");
        fs::write(&selected, "dropped content").unwrap();
        let store = LocalContextGrantStore::default();

        let drop_id = register_dropped_local_context(&store, std::slice::from_ref(&selected))
            .unwrap()
            .expect("drop receipt");
        assert_eq!(drop_id.len(), 64);
        assert!(!drop_id.contains("finder-note"));

        let issued = claim_dropped_local_context_with_store(
            &store,
            ClaimDroppedLocalContextRequest {
                drop_id: drop_id.clone(),
                session_id: "session-a".to_string(),
                turn_id: "turn-a".to_string(),
            },
        )
        .unwrap();
        assert_eq!(issued.results.len(), 1);
        assert!(issued.results[0].ok);
        assert_eq!(
            claim_dropped_local_context_with_store(
                &store,
                ClaimDroppedLocalContextRequest {
                    drop_id,
                    session_id: "session-a".to_string(),
                    turn_id: "turn-a".to_string(),
                },
            )
            .unwrap_err(),
            "local_context_drop_invalid_or_expired"
        );

        let response = read_local_context_with_store(
            &store,
            request_for_grant(&issued.results[0], "session-a", "turn-a"),
        )
        .unwrap();
        assert_eq!(response.name, "finder-note.txt");
        assert_eq!(response.text, "dropped content");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn directory_grant_is_operation_and_scope_bound() {
        let root = std::env::temp_dir().join(format!(
            "oomu-local-directory-list-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("reports")).expect("test directory created");
        fs::write(root.join("notes.md"), "# Private notes").expect("test file written");
        let store = LocalContextGrantStore::default();
        let issued = issue_grants_for_paths(
            &store,
            vec![root.clone()],
            GrantOperation::List,
            "session-a",
            "turn-a",
        );
        let mismatched = request_for_grant(&issued.results[0], "session-b", "turn-a");
        assert_eq!(
            list_local_directory_with_store(&store, mismatched).unwrap_err(),
            "local_context_grant_scope_mismatch"
        );

        let issued = issue_grants_for_paths(
            &store,
            vec![root.clone()],
            GrantOperation::List,
            "session-a",
            "turn-a",
        );
        let response = list_local_directory_with_store(
            &store,
            request_for_grant(&issued.results[0], "session-a", "turn-a"),
        )
        .unwrap();

        assert_eq!(response.entries, vec!["notes.md", "reports/"]);
        assert_eq!(response.text, "notes.md\nreports/");
        assert!(!response.truncated);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn replaced_file_invalidates_grant_before_read() {
        let root = std::env::temp_dir().join(format!(
            "oomu-local-context-replaced-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let selected = root.join("selected.txt");
        fs::write(&selected, "original").unwrap();
        let store = LocalContextGrantStore::default();
        let issued = issue_grants_for_paths(
            &store,
            vec![selected.clone()],
            GrantOperation::Read,
            "session",
            "turn",
        );
        fs::rename(&selected, root.join("original.txt")).unwrap();
        fs::write(&selected, "replacement").unwrap();
        let error = read_local_context_with_store(
            &store,
            request_for_grant(&issued.results[0], "session", "turn"),
        )
        .unwrap_err();
        assert_eq!(error, "local_context_grant_stale");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn same_inode_same_length_mutation_is_rejected_by_content_identity() {
        let root = std::env::temp_dir().join(format!(
            "oomu-local-context-mutated-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let selected = root.join("selected.txt");
        fs::write(&selected, "original").unwrap();
        let store = LocalContextGrantStore::default();
        let issued = issue_grants_for_paths(
            &store,
            vec![selected.clone()],
            GrantOperation::Read,
            "session",
            "turn",
        );
        fs::write(&selected, "mutated!").unwrap();
        // Simulate a filesystem where the attacker can restore coarse metadata;
        // the content digest must still reject the in-place mutation.
        {
            let mut state = store.state.lock().unwrap();
            let grant = state
                .grants
                .get_mut(issued.results[0].grant_id.as_deref().unwrap())
                .unwrap();
            grant.identity =
                FileIdentity::from_metadata(&fs::metadata(&selected).unwrap()).unwrap();
        }
        let error = read_local_context_with_store(
            &store,
            request_for_grant(&issued.results[0], "session", "turn"),
        )
        .unwrap_err();
        assert_eq!(error, "local_context_grant_stale");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn per_file_grant_results_preserve_valid_files_and_bound_count() {
        let root = std::env::temp_dir().join(format!(
            "oomu-local-context-results-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut paths = Vec::new();
        for index in 0..=MAX_PICKER_FILES {
            let path = root.join(format!("{index}.txt"));
            fs::write(&path, format!("file {index}")).unwrap();
            paths.push(path);
        }
        paths.insert(1, root.join("missing.txt"));
        let response = issue_grants_for_paths(
            &LocalContextGrantStore::default(),
            paths,
            GrantOperation::Read,
            "session",
            "turn",
        );
        assert!(response.results[0].ok);
        assert!(!response.results[1].ok);
        assert_eq!(
            response.results.last().unwrap().error_code,
            Some("attachment_count_limit_exceeded")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn repeated_picker_calls_share_the_same_turn_budget() {
        let root = std::env::temp_dir().join(format!(
            "oomu-local-context-cumulative-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let store = LocalContextGrantStore::default();
        for index in 0..MAX_PICKER_FILES {
            let path = root.join(format!("first-{index}.txt"));
            fs::write(&path, "ok").unwrap();
            let response = issue_grants_for_paths(
                &store,
                vec![path],
                GrantOperation::Read,
                "session",
                "same-turn",
            );
            assert!(response.results[0].ok);
        }
        let extra = root.join("extra.txt");
        fs::write(&extra, "must be rejected").unwrap();
        let response = issue_grants_for_paths(
            &store,
            vec![extra],
            GrantOperation::Read,
            "session",
            "same-turn",
        );
        assert_eq!(
            response.results[0].error_code,
            Some("attachment_count_limit_exceeded")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn global_capacity_bounds_many_renderer_scopes_and_reclaims_expired_grants() {
        let root = std::env::temp_dir().join(format!(
            "oomu-local-context-global-capacity-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let selected = root.join("selected.txt");
        fs::write(&selected, "bounded").unwrap();
        let store = LocalContextGrantStore::default();

        for index in 0..MAX_LIVE_CONTEXT_FILE_GRANTS {
            let response = issue_grants_for_paths(
                &store,
                vec![selected.clone()],
                GrantOperation::Read,
                &format!("attacker-session-{index}"),
                &format!("attacker-turn-{index}"),
            );
            assert!(response.results[0].ok);
        }
        let overflow = issue_grants_for_paths(
            &store,
            vec![selected.clone()],
            GrantOperation::Read,
            "attacker-session-overflow",
            "attacker-turn-overflow",
        );
        assert_eq!(
            overflow.results[0].error_code,
            Some("local_context_global_capacity_exceeded")
        );
        assert_eq!(
            store.state.lock().unwrap().grants.len(),
            MAX_LIVE_CONTEXT_FILE_GRANTS
        );

        for grant in store.state.lock().unwrap().grants.values_mut() {
            grant.expires_at_ms = unix_time_ms() - 1;
        }
        let reclaimed = issue_grants_for_paths(
            &store,
            vec![selected],
            GrantOperation::Read,
            "fresh-session",
            "fresh-turn",
        );
        assert!(reclaimed.results[0].ok);
        assert_eq!(store.state.lock().unwrap().grants.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn oversized_pixel_dimensions_are_rejected_before_full_read() {
        let root = std::env::temp_dir().join(format!(
            "oomu-local-context-pixels-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("bomb.png");
        let mut header = b"\x89PNG\r\n\x1a\n".to_vec();
        header.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
        header.extend_from_slice(&20_000_u32.to_be_bytes());
        header.extend_from_slice(&20_000_u32.to_be_bytes());
        fs::write(&path, header).unwrap();
        let response = issue_grants_for_paths(
            &LocalContextGrantStore::default(),
            vec![path],
            GrantOperation::Read,
            "session",
            "turn",
        );
        assert_eq!(
            response.results[0].error_code,
            Some("attachment_image_dimension_limit_exceeded")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pdf_grants_bypass_raster_dimension_validation() {
        let path = std::env::temp_dir().join(format!(
            "oomu-pdf-grant-{}-{}.pdf",
            std::process::id(),
            unix_time_ms()
        ));
        fs::write(&path, b"%PDF-1.4\n").unwrap();
        let response = issue_grants_for_paths(
            &LocalContextGrantStore::default(),
            vec![path.clone()],
            GrantOperation::Read,
            "session",
            "turn",
        );
        assert!(response.results[0].ok && response.results[0].mime_type == "application/pdf");
        let _ = fs::remove_file(path);
    }
}
