use crate::mcp::client::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpTool};
use crate::security::sandbox::SandboxRoot;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "taskflow_native";
const SERVER_VERSION: &str = "1.0.0";
const SANDBOX_ENV: &str = "OOMU_MCP_SANDBOX_DIR";

// Bounds keep folder_read grounded in a reasonable slice of the approved folder
// instead of streaming an unbounded tree into the edge model's context window.
const DEFAULT_MAX_FILES: usize = 50;
const MAX_FILES_CEILING: usize = 200;
const MAX_WALK_DEPTH: usize = 8;
const MAX_FILE_BYTES: usize = 64 * 1024;
const MAX_TOTAL_BYTES: usize = 512 * 1024;
const MAX_DISCOVERY_ENTRIES: usize = 4_096;
const MAX_DIALOG_TITLE_BYTES: usize = 256;
const MAX_TRUNCATION_NOTICE_BYTES: usize = 2 * 1024;
const MAX_SELECTION_ID_BYTES: usize = 96;
const STAGED_SELECTIONS_PATH: &str = "workspace/selections";
const SELECTION_NOTE_FILE_NAME: &str = "OOMU_SELECTION_NOTE.txt";

static STAGING_NONCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSourceFolderMetadata {
    pub folder_name: String,
    pub folder_path: String,
    pub file_count: usize,
    pub total_bytes: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy)]
struct StagingLimits {
    max_files: usize,
    max_candidates: usize,
    max_depth: usize,
    max_entries: usize,
    max_file_bytes: usize,
    max_total_bytes: usize,
}

const DEFAULT_STAGING_LIMITS: StagingLimits = StagingLimits {
    max_files: DEFAULT_MAX_FILES,
    max_candidates: MAX_FILES_CEILING,
    max_depth: MAX_WALK_DEPTH,
    max_entries: MAX_DISCOVERY_ENTRIES,
    max_file_bytes: MAX_FILE_BYTES,
    max_total_bytes: MAX_TOTAL_BYTES,
};

#[derive(Debug, Default)]
struct StagingDiscovery {
    candidates: Vec<PathBuf>,
    visited_entries: usize,
    truncated: bool,
}

#[derive(Debug)]
struct StagedSourceFile {
    path: PathBuf,
    bytes: usize,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn choose_workflow_source_folder(
    selection_id: String,
    title: String,
    truncation_notice: String,
) -> Result<Option<WorkflowSourceFolderMetadata>, String> {
    let selection_id = validate_selection_id(&selection_id)?;
    let title = sanitize_dialog_title(&title)?;
    let truncation_notice = sanitize_truncation_notice(&truncation_notice)?;
    let Some(selected_directory) = rfd::AsyncFileDialog::new()
        .set_title(title)
        .pick_folder()
        .await
    else {
        return Ok(None);
    };
    let source = selected_directory.path().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let sandbox_root = crate::mcp::bootstrap::mcp_sandbox_root();
        stage_workflow_source_folder(
            &source,
            &sandbox_root,
            &selection_id,
            &truncation_notice,
            DEFAULT_STAGING_LIMITS,
        )
        .map(Some)
    })
    .await
    .map_err(|error| format!("Folder preparation stopped unexpectedly: {error}"))?
}

/// Real, fully functional native MCP server that backs the `taskflow_native`
/// capabilities advertised by the workflow compiler. It runs in-process (no
/// child process, no network) and every operation is confined to the same
/// canonicalized sandbox jail used by the local filesystem server.
#[derive(Debug, Clone)]
pub struct NativeTaskflowServer {
    sandbox: SandboxRoot,
}

impl NativeTaskflowServer {
    pub fn from_env(env: &HashMap<String, String>) -> Result<Self, String> {
        let sandbox_root = env
            .get(SANDBOX_ENV)
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(crate::mcp::bootstrap::mcp_sandbox_root);
        Self::new(sandbox_root)
    }

    pub fn new(sandbox_root: PathBuf) -> Result<Self, String> {
        Ok(Self {
            sandbox: SandboxRoot::new(sandbox_root)?,
        })
    }

    pub fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id;
        let result = match request.method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION}
            })),
            "tools/list" => Ok(json!({ "tools": tool_list() })),
            "tools/call" => Ok(self.call_tool(request.params)),
            method => Err(format!("Unsupported MCP method: {method}")),
        };

        match result {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(result),
                error: None,
                id,
            },
            Err(message) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(json!({"code": -32000, "message": message})),
                id,
            },
        }
    }

    pub fn handle_notification(&self, notification: JsonRpcNotification) -> Result<(), String> {
        match notification.method.as_str() {
            "notifications/initialized" => Ok(()),
            method => Err(format!("Unsupported MCP notification: {method}")),
        }
    }

    fn call_tool(&self, params: Value) -> Value {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let Some(arguments) = arguments.as_object() else {
            return error_result("Tool arguments must be an object.");
        };

        match name {
            "folder_read" => self
                .folder_read(arguments)
                .unwrap_or_else(|error| error_result(&error)),
            "write_markdown_report" => self
                .write_markdown_report(arguments)
                .unwrap_or_else(|error| error_result(&error)),
            "preview_report" => self
                .preview_report(arguments)
                .unwrap_or_else(|error| error_result(&error)),
            _ => error_result(&format!("Unknown tool: {name}")),
        }
    }

    fn folder_read(&self, arguments: &Map<String, Value>) -> Result<Value, String> {
        let raw_folder = first_string_arg(
            arguments,
            &["folderPath", "folder_path", "folder", "path", "directory"],
        )
        .ok_or_else(|| {
            "folder_read requires an explicit approved sandbox folder path.".to_string()
        })?;
        let max_files = arguments
            .get("maxFiles")
            .or_else(|| arguments.get("max_files"))
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MAX_FILES)
            .min(MAX_FILES_CEILING);

        let folder = self.sandbox.resolve(&raw_folder)?;
        if !folder.is_dir() {
            return Err(format!(
                "folder_read requires an approved sandbox folder. \"{}\" is not a directory inside the sandbox.",
                display_relative(&raw_folder)
            ));
        }

        let mut candidates = Vec::new();
        collect_text_files(&folder, 0, &mut candidates)?;
        candidates.sort_by(|left, right| {
            let left_is_note = is_root_selection_note(&folder, left);
            let right_is_note = is_root_selection_note(&folder, right);
            right_is_note
                .cmp(&left_is_note)
                .then_with(|| left.cmp(right))
        });

        let mut files = Vec::new();
        let mut total_bytes = 0usize;
        let mut truncated = candidates.len() > max_files;
        for path in candidates.into_iter().take(max_files) {
            // Re-resolve every candidate so a symlink swapped in mid-walk still
            // cannot read outside the jail.
            let resolved = self.sandbox.resolve(&path).map_err(|error| {
                format!("folder_read could not revalidate a discovered path: {error}")
            })?;
            let raw = match fs::read_to_string(&resolved) {
                Ok(raw) => raw,
                Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                    // A non-UTF-8 artifact is not a readable text file.
                    continue;
                }
                Err(error) => {
                    return Err(format!(
                        "folder_read could not read discovered file {}: {error}",
                        resolved.display()
                    ));
                }
            };
            let mut content = raw;
            let original_len = content.len();
            if content.len() > MAX_FILE_BYTES {
                truncate_utf8(&mut content, MAX_FILE_BYTES);
            }
            if total_bytes + content.len() > MAX_TOTAL_BYTES {
                truncated = true;
                break;
            }
            total_bytes += content.len();
            files.push(json!({
                "path": self.sandbox.relative_path(&resolved),
                "bytes": original_len,
                "content": content,
            }));
        }

        let folder_label = self.sandbox.relative_path(&folder);
        let text = if files.is_empty() {
            format!("No readable text files were found in \"{folder_label}\".")
        } else {
            let mut sections = Vec::with_capacity(files.len());
            for file in &files {
                let path = file.get("path").and_then(Value::as_str).unwrap_or_default();
                let content = file
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                sections.push(format!("### {path}\n{content}"));
            }
            sections.join("\n\n")
        };

        Ok(text_result(
            &text,
            Some(json!({
                "root": self.sandbox.root().display().to_string(),
                "folder": folder_label,
                "fileCount": files.len(),
                "truncated": truncated,
                "files": files,
            })),
        ))
    }

    fn write_markdown_report(&self, arguments: &Map<String, Value>) -> Result<Value, String> {
        let raw_path = first_string_arg(
            arguments,
            &["reportPath", "report_path", "path", "filePath", "file_path"],
        )
        .ok_or_else(|| "write_markdown_report requires a report path.".to_string())?;
        let content = first_string_arg(
            arguments,
            &["content", "report", "markdown", "body", "text"],
        )
        .ok_or_else(|| {
            "write_markdown_report requires report content. Connect the summary step to this report so it has something to write.".to_string()
        })?;

        let target = self.sandbox.resolve(&raw_path)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Unable to create sandbox parent directory at {}: {error}",
                    parent.display()
                )
            })?;
        }

        fs::write(&target, content.as_bytes())
            .map_err(|error| format!("Unable to write report {}: {error}", target.display()))?;
        let actual = fs::read(&target).map_err(|error| {
            format!(
                "Unable to verify report write at {}: {error}",
                target.display()
            )
        })?;
        if actual != content.as_bytes() {
            return Err(format!(
                "Unable to verify report write at {}: final contents did not match the requested content.",
                target.display()
            ));
        }

        let rel = self.sandbox.relative_path(&target);
        Ok(text_result(
            &format!("Execution Completed: {rel} written successfully."),
            Some(json!({
                "path": target.display().to_string(),
                "relativePath": rel,
                "bytesWritten": content.len(),
            })),
        ))
    }

    fn preview_report(&self, arguments: &Map<String, Value>) -> Result<Value, String> {
        let raw_path = first_string_arg(
            arguments,
            &["reportPath", "report_path", "path", "filePath", "file_path"],
        )
        .ok_or_else(|| "preview_report requires a report path.".to_string())?;
        let target = self.sandbox.resolve(&raw_path)?;
        if !target.is_file() {
            return Err(format!(
                "preview_report could not find a report at \"{}\". Write the report before previewing it.",
                display_relative(&raw_path)
            ));
        }
        let content = fs::read_to_string(&target)
            .map_err(|error| format!("Unable to read report {}: {error}", target.display()))?;
        let rel = self.sandbox.relative_path(&target);
        Ok(text_result(
            &format!("Report ready for review at {rel}:\n\n{content}"),
            Some(json!({
                "path": target.display().to_string(),
                "relativePath": rel,
                "bytes": content.len(),
                "content": content,
            })),
        ))
    }
}

pub fn native_taskflow_tools() -> Result<Vec<McpTool>, String> {
    serde_json::from_value(Value::Array(tool_list()))
        .map_err(|error| format!("Invalid native taskflow tool schema: {error}"))
}

fn validate_selection_id(value: &str) -> Result<String, String> {
    let suffix = value
        .strip_prefix("selection-")
        .filter(|suffix| !suffix.is_empty())
        .ok_or_else(|| "Workflow selection ID is invalid.".to_string())?;
    let valid = value.len() <= MAX_SELECTION_ID_BYTES
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && suffix
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && suffix
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && !suffix.contains("--");
    if !valid {
        return Err("Workflow selection ID is invalid.".to_string());
    }
    Ok(value.to_string())
}

fn staged_selection_path(selection_id: &str) -> String {
    format!("{STAGED_SELECTIONS_PATH}/{selection_id}")
}

fn sanitize_dialog_title(value: &str) -> Result<String, String> {
    let filtered = value
        .chars()
        .filter(|character| !character.is_control() || character.is_whitespace())
        .collect::<String>();
    let sanitized = filtered.split_whitespace().collect::<Vec<_>>().join(" ");
    validate_localized_picker_text(&sanitized, MAX_DIALOG_TITLE_BYTES, "title")?;
    Ok(sanitized)
}

fn sanitize_truncation_notice(value: &str) -> Result<String, String> {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    let sanitized = normalized
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect::<String>();
    let sanitized = sanitized.trim().to_string();
    validate_localized_picker_text(&sanitized, MAX_TRUNCATION_NOTICE_BYTES, "truncation notice")?;
    Ok(sanitized)
}

fn validate_localized_picker_text(
    value: &str,
    max_bytes: usize,
    label: &str,
) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("Localized folder picker {label} is required."));
    }
    if value.len() > max_bytes {
        return Err(format!(
            "Localized folder picker {label} exceeds its safety limit."
        ));
    }
    Ok(())
}

fn stage_workflow_source_folder(
    source: &Path,
    sandbox_root: &Path,
    selection_id: &str,
    truncation_notice: &str,
    limits: StagingLimits,
) -> Result<WorkflowSourceFolderMetadata, String> {
    validate_staging_limits(limits)?;
    let selection_id = validate_selection_id(selection_id)?;
    let folder_path = staged_selection_path(&selection_id);
    if truncation_notice.is_empty()
        || truncation_notice.len() > limits.max_file_bytes
        || truncation_notice.len() > limits.max_total_bytes
    {
        return Err("The localized truncation notice exceeds the staging limits.".to_string());
    }
    let (source_root, source_handle) = open_source_directory(source)?;
    let source_identity = source_handle
        .metadata()
        .map_err(|error| format!("Unable to inspect the chosen folder: {error}"))?;
    revalidate_source_directory(&source_root, &source_handle, &source_identity)?;

    let mut discovery = StagingDiscovery::default();
    discover_staging_candidates(&source_root, &source_root, 0, limits, &mut discovery)?;
    discovery.candidates.sort();
    discovery.candidates.dedup();

    let sandbox_root = prepare_private_sandbox_root(sandbox_root)?;
    let workspace = sandbox_root.join("workspace");
    ensure_private_directory(&workspace)?;
    let selections = workspace.join("selections");
    ensure_private_directory(&selections)?;
    let target = selections.join(&selection_id);
    validate_existing_staged_input(&target)?;
    let prepared = create_unique_private_directory(&selections, ".selection-preparing")?;

    let preparation = (|| {
        let mut staged_sources = Vec::<StagedSourceFile>::new();
        let mut total_bytes = 0usize;
        let mut truncated = discovery.truncated;
        for candidate in discovery.candidates {
            if staged_sources.len() >= limits.max_files {
                truncated = true;
                break;
            }
            revalidate_source_directory(&source_root, &source_handle, &source_identity)?;
            let relative = candidate.strip_prefix(&source_root).map_err(|_| {
                "A discovered file escaped the chosen folder during preparation.".to_string()
            })?;
            if relative == Path::new(SELECTION_NOTE_FILE_NAME) {
                // This name is reserved for the localized completeness note. The
                // source file remains untouched and its omission is disclosed.
                truncated = true;
                continue;
            }
            let bytes =
                match read_staging_candidate(&source_root, &candidate, limits.max_file_bytes)? {
                    CandidateContent::Text(bytes) => bytes,
                    CandidateContent::Ignored => continue,
                    CandidateContent::OverLimit => {
                        truncated = true;
                        continue;
                    }
                };
            if total_bytes.saturating_add(bytes.len()) > limits.max_total_bytes {
                truncated = true;
                break;
            }
            let destination = safe_staging_destination(&prepared, relative)?;
            if let Some(parent) = destination.parent() {
                ensure_private_relative_tree(&prepared, parent)?;
            }
            write_private_file(&destination, &bytes)?;
            total_bytes += bytes.len();
            staged_sources.push(StagedSourceFile {
                path: destination,
                bytes: bytes.len(),
            });
        }
        revalidate_source_directory(&source_root, &source_handle, &source_identity)?;
        if staged_sources.is_empty() {
            return Err(
                "The chosen folder contains no readable UTF-8 text files within the safety limits."
                    .to_string(),
            );
        }
        if truncated {
            while staged_sources.len() >= limits.max_files
                || total_bytes.saturating_add(truncation_notice.len()) > limits.max_total_bytes
            {
                let Some(removed) = staged_sources.pop() else {
                    break;
                };
                fs::remove_file(&removed.path).map_err(|error| {
                    format!("Unable to reserve the localized selection note: {error}")
                })?;
                total_bytes = total_bytes.saturating_sub(removed.bytes);
            }
            if staged_sources.is_empty() {
                return Err(
                    "The chosen folder contains no readable UTF-8 text files within the safety limits."
                        .to_string(),
                );
            }
            let note_path = prepared.join(SELECTION_NOTE_FILE_NAME);
            write_private_file(&note_path, truncation_notice.as_bytes())?;
            total_bytes += truncation_notice.len();
            staged_sources.push(StagedSourceFile {
                path: note_path,
                bytes: truncation_notice.len(),
            });
        }
        Ok((staged_sources.len(), total_bytes, truncated))
    })();

    let (file_count, total_bytes, truncated) = match preparation {
        Ok(prepared_metadata) => prepared_metadata,
        Err(error) => {
            let _ = fs::remove_dir_all(&prepared);
            return Err(error);
        }
    };
    if let Err(error) = publish_prepared_input(&prepared, &target, &selections) {
        let _ = fs::remove_dir_all(&prepared);
        return Err(error);
    }

    Ok(WorkflowSourceFolderMetadata {
        folder_name: display_folder_name(&source_root, &folder_path),
        folder_path,
        file_count,
        total_bytes,
        truncated,
    })
}

#[derive(Debug, PartialEq, Eq)]
enum CandidateContent {
    Text(Vec<u8>),
    Ignored,
    OverLimit,
}

fn validate_staging_limits(limits: StagingLimits) -> Result<(), String> {
    if limits.max_files == 0
        || limits.max_candidates == 0
        || limits.max_entries == 0
        || limits.max_file_bytes == 0
        || limits.max_total_bytes == 0
    {
        return Err("Folder preparation limits must be greater than zero.".to_string());
    }
    Ok(())
}

fn open_source_directory(source: &Path) -> Result<(PathBuf, File), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("Unable to inspect the chosen folder: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Choose a real folder, not a file or symbolic link.".to_string());
    }
    let canonical = fs::canonicalize(source)
        .map_err(|error| format!("Unable to resolve the chosen folder: {error}"))?;
    let handle = File::open(&canonical)
        .map_err(|error| format!("Unable to open the chosen folder: {error}"))?;
    Ok((canonical, handle))
}

fn revalidate_source_directory(
    source_root: &Path,
    source_handle: &File,
    expected_identity: &fs::Metadata,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source_root)
        .map_err(|_| "The chosen folder changed while it was being prepared.".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The chosen folder changed while it was being prepared.".to_string());
    }
    let canonical = fs::canonicalize(source_root)
        .map_err(|_| "The chosen folder changed while it was being prepared.".to_string())?;
    let handle_metadata = source_handle
        .metadata()
        .map_err(|_| "The chosen folder changed while it was being prepared.".to_string())?;
    if canonical != source_root
        || !handle_metadata.is_dir()
        || !metadata_identity_matches(&metadata, expected_identity)
        || !metadata_identity_matches(&handle_metadata, expected_identity)
    {
        return Err("The chosen folder changed while it was being prepared.".to_string());
    }
    Ok(())
}

fn discover_staging_candidates(
    source_root: &Path,
    directory: &Path,
    depth: usize,
    limits: StagingLimits,
    discovery: &mut StagingDiscovery,
) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if depth == 0 => {
            return Err(format!("Unable to read the chosen folder: {error}"));
        }
        Err(_) => return Ok(()),
    };
    let mut paths = Vec::new();
    for entry in entries {
        if discovery.visited_entries >= limits.max_entries {
            discovery.truncated = true;
            break;
        }
        discovery.visited_entries += 1;
        if let Ok(entry) = entry {
            paths.push(entry.path());
        }
    }
    paths.sort();

    let path_count = paths.len();
    for (index, path) in paths.into_iter().enumerate() {
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        let Ok(canonical) = fs::canonicalize(&path) else {
            continue;
        };
        if !canonical.starts_with(source_root) {
            return Err("Folder preparation refused a path outside the chosen folder.".to_string());
        }
        if metadata.is_dir() {
            if depth >= limits.max_depth {
                discovery.truncated = true;
                continue;
            }
            discover_staging_candidates(source_root, &canonical, depth + 1, limits, discovery)?;
            if discovery.visited_entries >= limits.max_entries
                || discovery.candidates.len() >= limits.max_candidates
            {
                if index + 1 < path_count {
                    discovery.truncated = true;
                }
                return Ok(());
            }
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        if discovery.candidates.len() >= limits.max_candidates {
            discovery.truncated = true;
            return Ok(());
        }
        discovery.candidates.push(canonical);
    }
    Ok(())
}

fn read_staging_candidate(
    source_root: &Path,
    candidate: &Path,
    max_file_bytes: usize,
) -> Result<CandidateContent, String> {
    let Ok(metadata) = fs::symlink_metadata(candidate) else {
        return Ok(CandidateContent::Ignored);
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(CandidateContent::Ignored);
    }
    if metadata.len() > max_file_bytes as u64 {
        return Ok(CandidateContent::OverLimit);
    }
    let Ok(canonical) = fs::canonicalize(candidate) else {
        return Ok(CandidateContent::Ignored);
    };
    if !canonical.starts_with(source_root) {
        return Err("Folder preparation refused a file outside the chosen folder.".to_string());
    }
    let Ok(handle) = File::open(&canonical) else {
        return Ok(CandidateContent::Ignored);
    };
    let Ok(handle_metadata) = handle.metadata() else {
        return Ok(CandidateContent::Ignored);
    };
    revalidate_source_file(source_root, &canonical, &handle_metadata)?;
    let mut bytes = Vec::new();
    if handle
        .take(max_file_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Ok(CandidateContent::Ignored);
    }
    if bytes.len() > max_file_bytes {
        return Ok(CandidateContent::OverLimit);
    }
    revalidate_source_file(source_root, &canonical, &handle_metadata)?;
    if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
        return Ok(CandidateContent::Ignored);
    }
    Ok(CandidateContent::Text(bytes))
}

fn revalidate_source_file(
    source_root: &Path,
    candidate: &Path,
    expected_identity: &fs::Metadata,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(candidate)
        .map_err(|_| "A selected file changed during folder preparation.".to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || !metadata_identity_matches(&metadata, expected_identity)
    {
        return Err("A selected file changed during folder preparation.".to_string());
    }
    let canonical = fs::canonicalize(candidate)
        .map_err(|_| "A selected file changed during folder preparation.".to_string())?;
    if canonical != candidate || !canonical.starts_with(source_root) {
        return Err("A selected file escaped the chosen folder.".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn metadata_identity_matches(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn metadata_identity_matches(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.is_dir() == right.is_dir()
        && left.is_file() == right.is_file()
        && left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
}

fn prepare_private_sandbox_root(sandbox_root: &Path) -> Result<PathBuf, String> {
    ensure_private_directory(sandbox_root)?;
    fs::canonicalize(sandbox_root)
        .map_err(|error| format!("Unable to resolve the private MCP sandbox: {error}"))
}

fn ensure_private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!(
                "Private staging requires a real directory at {}.",
                path.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|error| {
                format!(
                    "Unable to create private staging directory {}: {error}",
                    path.display()
                )
            })?;
        }
        Err(error) => {
            return Err(format!(
                "Unable to inspect private staging directory {}: {error}",
                path.display()
            ));
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "Unable to verify private staging directory {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "Private staging requires a real directory at {}.",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            format!(
                "Unable to protect private staging directory {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn create_unique_private_directory(parent: &Path, prefix: &str) -> Result<PathBuf, String> {
    for _ in 0..32 {
        let path = parent.join(unique_staging_name(prefix));
        match fs::create_dir(&path) {
            Ok(()) => {
                if let Err(error) = ensure_private_directory(&path) {
                    let _ = fs::remove_dir_all(&path);
                    return Err(error);
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("Unable to prepare the selected folder: {error}"));
            }
        }
    }
    Err("Unable to reserve private folder preparation space.".to_string())
}

fn unique_staging_name(prefix: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let nonce = STAGING_NONCE.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{timestamp}-{nonce}", std::process::id())
}

fn safe_staging_destination(prepared: &Path, relative: &Path) -> Result<PathBuf, String> {
    use std::path::Component;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Folder preparation found an unsafe relative path.".to_string());
    }
    Ok(prepared.join(relative))
}

fn ensure_private_relative_tree(prepared: &Path, parent: &Path) -> Result<(), String> {
    let relative = parent
        .strip_prefix(prepared)
        .map_err(|_| "Folder preparation found an unsafe output path.".to_string())?;
    let mut current = prepared.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err("Folder preparation found an unsafe output path.".to_string());
        };
        current.push(name);
        ensure_private_directory(&current)?;
    }
    Ok(())
}

fn write_private_file(destination: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(destination).map_err(|error| {
        format!(
            "Unable to stage selected file {}: {error}",
            destination.display()
        )
    })?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            format!(
                "Unable to finish selected file {}: {error}",
                destination.display()
            )
        })
}

fn validate_existing_staged_input(target: &Path) -> Result<(), String> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            "Existing staged input is not a safe directory. Remove it before choosing a folder."
                .to_string(),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Unable to inspect existing staged input: {error}")),
    }
}

fn publish_prepared_input(prepared: &Path, target: &Path, parent: &Path) -> Result<(), String> {
    validate_existing_staged_input(target)?;
    let previous = match fs::symlink_metadata(target) {
        Ok(_) => {
            let backup = parent.join(unique_staging_name(".selection-previous"));
            fs::rename(target, &backup)
                .map_err(|error| format!("Unable to preserve prior staged input: {error}"))?;
            Some(backup)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("Unable to inspect prior staged input: {error}")),
    };

    if let Err(error) = fs::rename(prepared, target) {
        let restore_error = previous
            .as_ref()
            .and_then(|backup| fs::rename(backup, target).err());
        let _ = fs::remove_dir_all(prepared);
        return match restore_error {
            Some(restore_error) => Err(format!(
                "Unable to publish prepared input ({error}) or restore the prior input ({restore_error})."
            )),
            None => Err(format!("Unable to publish prepared input: {error}")),
        };
    }
    if let Some(previous) = previous {
        // Publication has completed. A cleanup failure must not turn a successful,
        // usable selection into a false failure for the user.
        let _ = fs::remove_dir_all(previous);
    }
    Ok(())
}

fn display_folder_name(source_root: &Path, fallback: &str) -> String {
    let raw = source_root
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let name = raw
        .chars()
        .filter(|character| !character.is_control())
        .take(120)
        .collect::<String>();
    if name.trim().is_empty() {
        fallback.to_string()
    } else {
        name
    }
}

fn is_root_selection_note(folder: &Path, candidate: &Path) -> bool {
    candidate
        .strip_prefix(folder)
        .is_ok_and(|relative| relative == Path::new(SELECTION_NOTE_FILE_NAME))
}

fn collect_text_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if depth > MAX_WALK_DEPTH || out.len() >= MAX_FILES_CEILING {
        return Ok(());
    }
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("folder_read could not inspect {}: {error}", dir.display()))?;
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "folder_read could not inspect an entry in {}: {error}",
                dir.display()
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "folder_read could not inspect the type of {}: {error}",
                entry.path().display()
            )
        })?;
        // Never follow symlinks: they are the classic sandbox-escape vector and
        // the sandbox resolver rejects them anyway.
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if file_type.is_dir() {
            directories.push(entry.path());
        } else if file_type.is_file() {
            out.push(entry.path());
            if out.len() >= MAX_FILES_CEILING {
                return Ok(());
            }
        }
    }
    for directory in directories {
        collect_text_files(&directory, depth + 1, out)?;
        if out.len() >= MAX_FILES_CEILING {
            return Ok(());
        }
    }
    Ok(())
}

fn first_string_arg(arguments: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = arguments.get(*key).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn truncate_utf8(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value.push_str("\n…(truncated)");
}

fn display_relative(raw: &str) -> String {
    if raw.trim().is_empty() {
        "(sandbox root)".to_string()
    } else {
        raw.to_string()
    }
}

fn tool_list() -> Vec<Value> {
    vec![
        json!({
            "name": "folder_read",
            "description": "Scan the text files inside an approved sandbox folder and return their contents for grounding.",
            "outputSchema": {
                "type": "object",
                "x-oomu-result-contract": {
                    "kind": "collection",
                    "path": "/structuredContent/files",
                    "emptyIsSuccess": true
                },
                "properties": {
                    "structuredContent": {
                        "type": "object",
                        "properties": {"files": {"type": "array", "items": {}}},
                        "required": ["files"],
                        "additionalProperties": true
                    }
                },
                "required": ["structuredContent"],
                "additionalProperties": true
            },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "folderPath": {
                        "type": "string",
                        "description": "The approved project folder path."
                    }
                },
                "required": ["folderPath"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "write_markdown_report",
            "description": "Write a Markdown report into an approved sandbox folder.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reportPath": {
                        "type": "string",
                        "description": "Markdown report file path."
                    },
                    "content": {
                        "type": "string",
                        "description": "Markdown report content to write."
                    }
                },
                "required": ["reportPath", "content"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "preview_report",
            "description": "Read back a generated Markdown report from the sandbox so it can be reviewed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reportPath": {
                        "type": "string",
                        "description": "The path to the generated report to preview."
                    }
                },
                "required": ["reportPath"],
                "additionalProperties": false
            }
        }),
    ]
}

fn text_result(text: &str, structured: Option<Value>) -> Value {
    let mut result = json!({
        "content": [{"type": "text", "text": text}],
        "isError": false
    });
    if let Some(structured) = structured {
        result["structuredContent"] = structured;
    }
    result
}

fn error_result(message: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": message}],
        "isError": true
    })
}

#[cfg(test)]
#[path = "taskflow_tests.rs"]
mod tests;
