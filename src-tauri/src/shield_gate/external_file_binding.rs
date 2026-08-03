use super::*;
use std::ffi::{CStr, CString};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt},
    io::{AsRawFd, FromRawFd},
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApprovedExternalFileReadBinding {
    pub(crate) canonical_path: String,
    device: u64,
    inode: u64,
    byte_count: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApprovedExternalDirectoryBinding {
    pub(crate) canonical_path: String,
    parent_path: String,
    parent_device: u64,
    parent_inode: u64,
    target_device: Option<u64>,
    target_inode: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct ApprovedExternalFileWriteBinding(ApprovedExternalWriteRequest);

impl ApprovedExternalFileWriteBinding {
    pub(crate) fn canonical_path(&self) -> &str {
        &self.0.path
    }

    pub(crate) fn missing_component_count(&self) -> usize {
        self.0.missing_components.len()
    }

    pub(crate) fn target_existed_when_bound(&self) -> bool {
        self.0.expected_target_identity.is_some()
    }
}

impl ApprovedExternalDirectoryBinding {
    pub(crate) fn existed_when_bound(&self) -> bool {
        self.target_device.is_some() && self.target_inode.is_some()
    }
}

pub(crate) struct ApprovedExternalFileContents {
    pub(crate) canonical_path: PathBuf,
    pub(crate) bytes: Zeroizing<Vec<u8>>,
    pub(crate) sha256: String,
}

pub(crate) fn bind_approved_external_file_write(
    path: &str,
) -> Result<ApprovedExternalFileWriteBinding, ShieldGateError> {
    prepare_approved_external_write_target(path, String::new())
        .map(ApprovedExternalFileWriteBinding)
}

pub(crate) fn write_bound_approved_external_file_atomically(
    binding: &ApprovedExternalFileWriteBinding,
    content: &str,
) -> Result<usize, ShieldGateError> {
    let mut request = binding.0.clone();
    request.content = content.to_string();
    write_bound_external_target_atomically(&request).map_err(approved_chat_file_error)
}

pub(crate) fn bind_approved_external_file_read(
    path: &str,
) -> Result<ApprovedExternalFileReadBinding, ShieldGateError> {
    let canonical_path = validate_approved_external_read_target(path, false)?;
    let metadata = fs::symlink_metadata(&canonical_path)
        .map_err(|_| approved_chat_file_error("The approved file is no longer available."))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(security_boundary_violation(
            "Approved file_read target must remain a regular file.".to_string(),
        ));
    }
    Ok(ApprovedExternalFileReadBinding {
        canonical_path: canonical_path.display().to_string(),
        device: metadata.dev(),
        inode: metadata.ino(),
        byte_count: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
    })
}

pub(crate) fn read_bound_approved_external_file_bounded(
    binding: &ApprovedExternalFileReadBinding,
    maximum_bytes: usize,
) -> Result<ApprovedExternalFileContents, ShieldGateError> {
    if maximum_bytes == 0 || maximum_bytes > 16 * 1024 * 1024 {
        return Err(approved_chat_file_error(
            "The approved file read has an invalid size limit.",
        ));
    }
    let canonical_path = PathBuf::from(&binding.canonical_path);
    let mut file = open_bound_external_target(
        &canonical_path,
        ApprovedFileIdentity {
            device: binding.device,
            inode: binding.inode,
        },
        false,
    )
    .map_err(approved_chat_file_error)?;
    let metadata = file
        .metadata()
        .map_err(|_| approved_chat_file_error("The approved file could not be checked."))?;
    if metadata.len() != binding.byte_count
        || metadata.mtime() != binding.modified_seconds
        || metadata.mtime_nsec() != binding.modified_nanoseconds
    {
        return Err(approved_chat_file_error(
            "The approved file changed before OOMU could read it. Nothing was changed.",
        ));
    }
    let mut bytes = Vec::with_capacity((binding.byte_count as usize).min(maximum_bytes));
    (&mut file)
        .take((maximum_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| approved_chat_file_error("The approved file could not be read safely."))?;
    if bytes.len() > maximum_bytes {
        return Err(approved_chat_file_error(format!(
            "The approved file is larger than the {maximum_bytes} byte decision-pack limit."
        )));
    }
    let final_metadata = file
        .metadata()
        .map_err(|_| approved_chat_file_error("The approved file could not be rechecked."))?;
    if final_metadata.len() != binding.byte_count
        || final_metadata.mtime() != binding.modified_seconds
        || final_metadata.mtime_nsec() != binding.modified_nanoseconds
    {
        return Err(approved_chat_file_error(
            "The approved file changed while OOMU was reading it. Nothing was changed.",
        ));
    }
    let sha256 = sha256_hex(&bytes);
    Ok(ApprovedExternalFileContents {
        canonical_path,
        bytes: Zeroizing::new(bytes),
        sha256,
    })
}

pub(crate) fn bind_approved_external_directory_creation(
    path: &str,
) -> Result<ApprovedExternalDirectoryBinding, ShieldGateError> {
    let requested = expand_shield_home_path(path, "create_file")?;
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
        || !requested.is_absolute()
        || requested.file_name().is_none()
    {
        return Err(security_boundary_violation(
            "Approved directory creation requires an absolute non-traversing target.".to_string(),
        ));
    }
    let parent = requested.parent().ok_or_else(|| {
        security_boundary_violation(
            "Approved directory creation requires an existing parent folder.".to_string(),
        )
    })?;
    let parent_metadata = fs::symlink_metadata(parent).map_err(|_| {
        security_boundary_violation(
            "Approved directory creation requires an existing parent folder.".to_string(),
        )
    })?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(security_boundary_violation(
            "Approved directory creation requires a real parent folder.".to_string(),
        ));
    }
    let parent_path = fs::canonicalize(parent).map_err(|_| {
        security_boundary_violation(
            "Approved directory creation parent could not be resolved.".to_string(),
        )
    })?;
    let parent_metadata = fs::symlink_metadata(&parent_path).map_err(|_| {
        security_boundary_violation(
            "Approved directory creation parent could not be inspected.".to_string(),
        )
    })?;
    let name = requested.file_name().ok_or_else(|| {
        security_boundary_violation("Approved output folder has no safe name.".to_string())
    })?;
    let canonical_path = parent_path.join(name);
    let target_identity = match fs::symlink_metadata(&canonical_path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            Some((metadata.dev(), metadata.ino()))
        }
        Ok(_) => {
            return Err(security_boundary_violation(
                "Approved output target must be a real folder or a new folder.".to_string(),
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            return Err(security_boundary_violation(
                "Approved output folder could not be inspected.".to_string(),
            ))
        }
    };
    Ok(ApprovedExternalDirectoryBinding {
        canonical_path: canonical_path.display().to_string(),
        parent_path: parent_path.display().to_string(),
        parent_device: parent_metadata.dev(),
        parent_inode: parent_metadata.ino(),
        target_device: target_identity.map(|value| value.0),
        target_inode: target_identity.map(|value| value.1),
    })
}

pub(crate) fn create_bound_approved_external_directory(
    binding: &ApprovedExternalDirectoryBinding,
) -> Result<PathBuf, ShieldGateError> {
    let parent = open_bound_external_target(
        Path::new(&binding.parent_path),
        ApprovedFileIdentity {
            device: binding.parent_device,
            inode: binding.parent_inode,
        },
        true,
    )
    .map_err(approved_chat_file_error)?;
    let target = PathBuf::from(&binding.canonical_path);
    let name = target
        .file_name()
        .ok_or_else(|| approved_chat_file_error("The approved output folder has no safe name."))?;
    let name = CString::new(name.as_bytes())
        .map_err(|_| approved_chat_file_error("The approved output folder name is not valid."))?;
    if binding.target_device.is_none() {
        let created = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
        if created != 0 {
            return Err(approved_chat_file_error(
                "The approved output folder changed before OOMU could create it. Nothing was changed.",
            ));
        }
    }
    let directory = open_directory_at(&parent, &name).map_err(approved_chat_file_error)?;
    let metadata = directory.metadata().map_err(|_| {
        approved_chat_file_error("The approved output folder could not be checked.")
    })?;
    if let (Some(device), Some(inode)) = (binding.target_device, binding.target_inode) {
        if metadata.dev() != device || metadata.ino() != inode {
            return Err(approved_chat_file_error(
                "The approved output folder changed before OOMU could use it. Nothing was changed.",
            ));
        }
    }
    Ok(target)
}

pub(super) fn required_approved_chat_file_binding(
    field: &'static str,
    value: Option<&str>,
) -> Result<String, ShieldGateError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| approved_chat_file_error(format!("Approved file access requires {field}.")))
}

pub(super) fn resolve_external_action_target(
    action: &RequestedAction,
    normalized_kind: &str,
) -> Option<Result<PathBuf, ShieldGateError>> {
    let path = action.path.as_deref()?;
    match normalized_kind {
        "file_read" if resolve_read_only_action_path("file_read", path).is_err() => {
            Some(validate_approved_external_read_target(path, false))
        }
        "file_list" if resolve_read_only_action_path("file_list", path).is_err() => {
            Some(validate_approved_external_read_target(path, true))
        }
        "file_write" if validate_project_quarantine(path, "file_write").is_err() => {
            Some(validate_approved_external_write_target(path))
        }
        "create_file"
        | "prepare_release_recovery_agenda"
        | "prepare_background_agent_comparison"
        | "prepare_milestone_constraint_recovery_plan" => {
            Some(validate_approved_external_write_target(path))
        }
        "create_decision_pack" => Some(
            bind_approved_external_directory_creation(path)
                .map(|binding| PathBuf::from(binding.canonical_path)),
        ),
        _ => None,
    }
}

pub(super) fn is_project_file_write_action(kind: &str) -> bool {
    matches!(
        kind,
        "file_write"
            | "create_file"
            | "configure_channel"
            | "prepare_release_recovery_agenda"
            | "prepare_background_agent_comparison"
            | "prepare_milestone_constraint_recovery_plan"
    )
}

pub(crate) fn reviewed_action_class(action_kind: &str) -> String {
    let normalized = normalize_action_kind(action_kind);
    match normalized.as_str() {
        "file_read" | "file_list" => "filesystem_read".to_string(),
        other if is_project_file_write_action(other) => "filesystem_write".to_string(),
        other => other.to_string(),
    }
}

pub(super) fn normalize_directory_read_action(mut action: RequestedAction) -> RequestedAction {
    if normalize_action_kind(&action.kind) != "file_read" {
        return action;
    }
    let Some(path) = action.path.as_deref() else {
        return action;
    };
    let Ok(requested) = expand_shield_home_path(path, "file_list") else {
        return action;
    };
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return action;
    }
    let candidate = if requested.is_absolute() {
        requested
    } else {
        project_root().join(requested)
    };
    if fs::symlink_metadata(candidate)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        action.kind = "file_list".to_string();
    }
    action
}

pub(super) fn inspect_approved_chat_read_target(
    action: &RequestedAction,
    path: &str,
) -> Result<(PathBuf, fs::Metadata, bool), ShieldGateError> {
    let normalized_action = normalize_directory_read_action(action.clone());
    let is_directory = normalize_action_kind(&normalized_action.kind) == "file_list";
    let canonical_path = validate_approved_external_read_target(path, is_directory)?;
    let metadata = fs::metadata(&canonical_path)
        .map_err(|_| approved_chat_file_error("The selected item is no longer available."))?;
    Ok((canonical_path, metadata, is_directory))
}

pub(super) fn approved_chat_read_mime_type(path: &Path, is_directory: bool) -> String {
    if is_directory {
        "text/plain".to_string()
    } else {
        crate::tools::vision::visual_mime_type_for_path(path)
            .unwrap_or_else(|| "text/plain".to_string())
    }
}

pub(super) fn approved_chat_read_byte_count(
    metadata: &fs::Metadata,
    content: &str,
    media_bytes: Option<&Zeroizing<Vec<u8>>>,
    is_directory: bool,
) -> usize {
    media_bytes.map_or_else(
        || {
            if is_directory {
                content.len()
            } else {
                metadata.len() as usize
            }
        },
        |bytes| bytes.len(),
    )
}

pub(super) fn prepare_external_filesystem_binding(
    action: &RequestedAction,
) -> Result<Option<(AuthorizedActions, RequestedAction)>, ShieldGateError> {
    let action = normalize_directory_read_action(action.clone());
    let Some(path) = action.path.as_deref() else {
        return Ok(None);
    };
    let normalized_kind = normalize_action_kind(&action.kind);
    let prepared = match normalized_kind.as_str() {
        "file_read" if resolve_read_only_action_path("file_read", path).is_err() => {
            let request = prepare_approved_external_read_target(path, false)?;
            let canonical_path = request.path.clone();
            (
                AuthorizedActions::ApprovedExternalFileRead(request),
                canonical_path,
            )
        }
        "file_list" if resolve_read_only_action_path("file_list", path).is_err() => {
            let request = prepare_approved_external_read_target(path, true)?;
            let canonical_path = request.path.clone();
            (
                AuthorizedActions::ApprovedExternalFileList(request),
                canonical_path,
            )
        }
        "file_write" if validate_project_quarantine(path, "file_write").is_err() => {
            let request = prepare_approved_external_write_target(
                path,
                action.content.clone().ok_or_else(|| ShieldGateError {
                    code: "shield_gate_invalid_input",
                    boundary: "AuthorizedActions",
                    message: "file_write requires a content string.".to_string(),
                })?,
            )?;
            let canonical_path = request.path.clone();
            (
                AuthorizedActions::ApprovedExternalFileWrite(request),
                canonical_path,
            )
        }
        _ => return Ok(None),
    };
    let mut bound_action = action;
    bound_action.path = Some(prepared.1);
    Ok(Some((prepared.0, bound_action)))
}

pub(crate) fn validate_approved_external_write_target(
    path: &str,
) -> Result<PathBuf, ShieldGateError> {
    let requested = expand_shield_home_path(path, "file_write")?;
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(security_boundary_violation(
            "Approved external file_write rejected path traversal.".to_string(),
        ));
    }
    if !requested.is_absolute() {
        return Err(security_boundary_violation(
            "Approved external file_write requires an absolute target path.".to_string(),
        ));
    }

    let requested_metadata = match fs::symlink_metadata(&requested) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            return Err(security_boundary_violation(
                "Approved external file_write target could not be inspected.".to_string(),
            ));
        }
    };
    if requested_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(security_boundary_violation(
            "Approved external file_write rejected a symbolic link target.".to_string(),
        ));
    }
    if requested_metadata
        .as_ref()
        .is_some_and(|metadata| !metadata.is_file())
    {
        return Err(security_boundary_violation(
            "Approved external file_write target must be a regular file or a new file.".to_string(),
        ));
    }

    // Resolve the nearest existing ancestor and rebuild the missing suffix on
    // its canonical path. The path shown for consent is therefore the path that
    // will actually be written, even when an earlier component is a symlink.
    let mut existing = requested.as_path();
    let mut missing_suffix = Vec::new();
    loop {
        match fs::symlink_metadata(existing) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(security_boundary_violation(
                        "Approved external file_write rejected a symbolic link parent.".to_string(),
                    ));
                }
                if existing != requested && !metadata.is_dir() {
                    return Err(security_boundary_violation(
                        "Approved external file_write parent is not a folder.".to_string(),
                    ));
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = existing.file_name() else {
                    return Err(security_boundary_violation(
                        "Approved external file_write could not resolve a safe parent directory."
                            .to_string(),
                    ));
                };
                missing_suffix.push(name.to_os_string());
                let Some(parent) = existing.parent() else {
                    return Err(security_boundary_violation(
                        "Approved external file_write could not resolve a safe parent directory."
                            .to_string(),
                    ));
                };
                existing = parent;
            }
            Err(_) => {
                return Err(security_boundary_violation(
                    "Approved external file_write parent could not be inspected.".to_string(),
                ));
            }
        }
    }

    let mut resolved = fs::canonicalize(existing).map_err(|_| {
        security_boundary_violation(
            "Approved external file_write could not resolve a safe parent directory.".to_string(),
        )
    })?;
    for component in missing_suffix.into_iter().rev() {
        resolved.push(component);
    }
    let resolved_parent = resolved.parent().ok_or_else(|| {
        security_boundary_violation(
            "Approved external file_write rejected a filesystem root target.".to_string(),
        )
    })?;
    if resolved_parent.parent().is_none()
        || resolved_parent
            .parent()
            .is_some_and(|parent| parent.parent().is_none())
    {
        return Err(security_boundary_violation(
            "Approved external file_write rejected a root-level directory target.".to_string(),
        ));
    }

    let project_root = fs::canonicalize(project_root()).unwrap_or_else(|_| project_root());
    if resolved.starts_with(project_root) {
        return Err(security_boundary_violation(
            "Approved external file_write received an in-sandbox path.".to_string(),
        ));
    }
    Ok(resolved)
}

pub(crate) fn validate_approved_external_read_target(
    path: &str,
    require_directory: bool,
) -> Result<PathBuf, ShieldGateError> {
    let operation = if require_directory {
        "file_list"
    } else {
        "file_read"
    };
    let requested = expand_shield_home_path(path, operation)?;
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(security_boundary_violation(format!(
            "Approved external {operation} rejected path traversal."
        )));
    }
    if !requested.is_absolute() {
        return Err(security_boundary_violation(format!(
            "Approved external {operation} requires an absolute path."
        )));
    }
    let metadata = fs::symlink_metadata(&requested).map_err(|_| {
        security_boundary_violation(format!(
            "Approved external {operation} requires an existing local target."
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(security_boundary_violation(format!(
            "Approved external {operation} rejected a symbolic link target."
        )));
    }
    let canonical = fs::canonicalize(&requested).map_err(|_| {
        security_boundary_violation(format!(
            "Approved external {operation} could not resolve the selected target."
        ))
    })?;
    if canonical.parent().is_none() {
        return Err(security_boundary_violation(format!(
            "Approved external {operation} rejected a filesystem root target."
        )));
    }
    if require_directory && !canonical.is_dir() {
        return Err(security_boundary_violation(
            "Approved external file_list target must be a folder.".to_string(),
        ));
    }
    if !require_directory && !canonical.is_file() {
        return Err(security_boundary_violation(
            "Approved external file_read target must be a regular file.".to_string(),
        ));
    }
    Ok(canonical)
}

pub(super) fn approved_file_identity(metadata: &fs::Metadata) -> ApprovedFileIdentity {
    ApprovedFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

pub(super) fn prepare_approved_external_read_target(
    path: &str,
    require_directory: bool,
) -> Result<ApprovedExternalReadRequest, ShieldGateError> {
    let canonical = validate_approved_external_read_target(path, require_directory)?;
    let metadata = fs::symlink_metadata(&canonical).map_err(|_| {
        security_boundary_violation(
            "The selected local item changed before approval could be prepared.".to_string(),
        )
    })?;
    if metadata.file_type().is_symlink()
        || (require_directory && !metadata.is_dir())
        || (!require_directory && !metadata.is_file())
    {
        return Err(security_boundary_violation(
            "The selected local item changed before approval could be prepared.".to_string(),
        ));
    }
    Ok(ApprovedExternalReadRequest {
        path: canonical.display().to_string(),
        expected_identity: approved_file_identity(&metadata),
    })
}

pub(super) fn prepare_approved_external_write_target(
    path: &str,
    content: String,
) -> Result<ApprovedExternalWriteRequest, ShieldGateError> {
    let canonical_target = validate_approved_external_write_target(path)?;
    let target_metadata = match fs::symlink_metadata(&canonical_target) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            return Err(security_boundary_violation(
                "The selected file changed before approval could be prepared.".to_string(),
            ));
        }
    };
    let expected_target_identity = target_metadata.as_ref().map(approved_file_identity);

    let mut missing_components = Vec::new();
    let mut anchor = if target_metadata.is_some() {
        let file_name = canonical_target.file_name().ok_or_else(|| {
            security_boundary_violation(
                "The selected file has no safe name inside its folder.".to_string(),
            )
        })?;
        missing_components.push(file_name.to_os_string());
        canonical_target.parent().ok_or_else(|| {
            security_boundary_violation("The selected file has no safe parent folder.".to_string())
        })?
    } else {
        canonical_target.as_path()
    };

    if target_metadata.is_none() {
        loop {
            match fs::symlink_metadata(anchor) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(security_boundary_violation(
                            "The selected file's parent folder changed before approval."
                                .to_string(),
                        ));
                    }
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let name = anchor.file_name().ok_or_else(|| {
                        security_boundary_violation(
                            "The selected file has no safe parent folder.".to_string(),
                        )
                    })?;
                    missing_components.push(name.to_os_string());
                    anchor = anchor.parent().ok_or_else(|| {
                        security_boundary_violation(
                            "The selected file has no safe parent folder.".to_string(),
                        )
                    })?;
                }
                Err(_) => {
                    return Err(security_boundary_violation(
                        "The selected file's parent folder could not be inspected.".to_string(),
                    ));
                }
            }
        }
        missing_components.reverse();
    }

    if missing_components.is_empty() {
        return Err(security_boundary_violation(
            "The selected file has no safe name inside its folder.".to_string(),
        ));
    }
    let anchor_path = fs::canonicalize(anchor).map_err(|_| {
        security_boundary_violation(
            "The selected file's parent folder could not be resolved.".to_string(),
        )
    })?;
    let anchor_metadata = fs::symlink_metadata(&anchor_path).map_err(|_| {
        security_boundary_violation(
            "The selected file's parent folder could not be inspected.".to_string(),
        )
    })?;
    if !anchor_metadata.is_dir() || anchor_metadata.file_type().is_symlink() {
        return Err(security_boundary_violation(
            "The selected file's parent is not a safe folder.".to_string(),
        ));
    }

    Ok(ApprovedExternalWriteRequest {
        path: canonical_target.display().to_string(),
        content,
        anchor_path,
        anchor_identity: approved_file_identity(&anchor_metadata),
        missing_components,
        expected_target_identity,
    })
}

pub(super) fn open_bound_external_target(
    path: &Path,
    expected_identity: ApprovedFileIdentity,
    require_directory: bool,
) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true).custom_flags(
        libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | if require_directory {
                libc::O_DIRECTORY
            } else {
                0
            },
    );
    let file = options.open(path).map_err(|error| {
        log_external_read_io_failure("open_target", &error);
        "The approved local item is no longer available.".to_string()
    })?;
    let metadata = file.metadata().map_err(|error| {
        log_external_read_io_failure("inspect_target", &error);
        "The approved local item could not be checked.".to_string()
    })?;
    let identity_matches = approved_file_identity(&metadata) == expected_identity;
    let target_kind_matches =
        (require_directory && metadata.is_dir()) || (!require_directory && metadata.is_file());
    if !identity_matches || !target_kind_matches {
        eprintln!(
            "SHIELD_EXTERNAL_READ_BINDING_FAILURE stage=target_identity_or_kind_mismatch identity_matches={} target_kind_matches={}",
            identity_matches, target_kind_matches
        );
        return Err(
            "The approved local item changed before OOMU could use it. Nothing was changed."
                .to_string(),
        );
    }
    Ok(file)
}

pub(super) fn list_bound_external_directory(directory: &fs::File) -> Result<Vec<String>, String> {
    struct DirectoryHandle(*mut libc::DIR);
    impl Drop for DirectoryHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { libc::closedir(self.0) };
            }
        }
    }

    let duplicated_fd = unsafe { libc::dup(directory.as_raw_fd()) };
    if duplicated_fd < 0 {
        log_external_read_io_failure(
            "duplicate_directory_handle",
            &std::io::Error::last_os_error(),
        );
        return Err("The approved folder could not be opened.".to_string());
    }
    let raw_directory = unsafe { libc::fdopendir(duplicated_fd) };
    if raw_directory.is_null() {
        log_external_read_io_failure("open_directory_stream", &std::io::Error::last_os_error());
        unsafe { libc::close(duplicated_fd) };
        return Err("The approved folder could not be opened.".to_string());
    }
    let directory = DirectoryHandle(raw_directory);
    let directory_fd = unsafe { libc::dirfd(directory.0) };
    if directory_fd < 0 {
        log_external_read_io_failure("read_directory_handle", &std::io::Error::last_os_error());
        return Err("The approved folder could not be opened.".to_string());
    }

    let mut entries = Vec::with_capacity(MAX_APPROVED_EXTERNAL_DIRECTORY_ENTRIES);
    while entries.len() < MAX_APPROVED_EXTERNAL_DIRECTORY_ENTRIES {
        clear_directory_errno();
        let entry = unsafe { libc::readdir(directory.0) };
        if entry.is_null() {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error().unwrap_or_default() != 0 {
                log_external_read_io_failure("read_directory_entry", &error);
                return Err("The approved folder could not be read completely.".to_string());
            }
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        let bytes = name.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                directory_fd,
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            log_external_read_io_failure(
                "inspect_directory_entry",
                &std::io::Error::last_os_error(),
            );
            return Err(
                "An item in the approved folder changed while OOMU was reading it.".to_string(),
            );
        }
        let mode = unsafe { stat.assume_init() }.st_mode;
        if mode & libc::S_IFMT == libc::S_IFLNK {
            continue;
        }
        let mut display_name = String::from_utf8_lossy(bytes).to_string();
        if mode & libc::S_IFMT == libc::S_IFDIR {
            display_name.push('/');
        }
        entries.push(display_name);
    }
    Ok(entries)
}

fn log_external_read_io_failure(stage: &str, error: &std::io::Error) {
    eprintln!(
        "SHIELD_EXTERNAL_READ_IO_FAILURE stage={} error_kind={:?} errno={}",
        stage,
        error.kind(),
        error.raw_os_error().unwrap_or_default()
    );
}

#[cfg(target_os = "macos")]
fn clear_directory_errno() {
    unsafe {
        *libc::__error() = 0;
    }
}

#[cfg(target_os = "linux")]
fn clear_directory_errno() {
    unsafe {
        *libc::__errno_location() = 0;
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn clear_directory_errno() {}

fn component_c_string(component: &OsString) -> Result<CString, String> {
    CString::new(component.as_os_str().as_bytes())
        .map_err(|_| "The approved file name is not valid.".to_string())
}

fn open_directory_at(parent: &fs::File, name: &CString) -> Result<fs::File, String> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err("The approved file's folder could not be opened safely.".to_string());
    }
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}

fn open_bound_external_write_parent(
    request: &ApprovedExternalWriteRequest,
) -> Result<(fs::File, CString), String> {
    let mut current_directory =
        open_bound_external_target(&request.anchor_path, request.anchor_identity, true)?;
    let (file_name, parent_components) = request
        .missing_components
        .split_last()
        .ok_or_else(|| "The approved file has no safe name.".to_string())?;

    for component in parent_components {
        let name = component_c_string(component)?;
        let created = unsafe { libc::mkdirat(current_directory.as_raw_fd(), name.as_ptr(), 0o700) };
        if created != 0 {
            return Err(
                "The approved file's folder changed before OOMU could create it. Nothing was changed."
                    .to_string(),
            );
        }
        current_directory = open_directory_at(&current_directory, &name)?;
    }

    Ok((current_directory, component_c_string(file_name)?))
}

fn verify_bound_external_write_target(
    parent: &fs::File,
    name: &CString,
    expected_identity: Option<ApprovedFileIdentity>,
) -> Result<Option<u32>, String> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        if expected_identity.is_none() && error.kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(
            "The approved file changed before OOMU could save it. Nothing was changed.".to_string(),
        );
    }
    let file = unsafe { fs::File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|_| "The approved file could not be checked.".to_string())?;
    if !metadata.is_file() || expected_identity != Some(approved_file_identity(&metadata)) {
        return Err(
            "The approved file changed before OOMU could save it. Nothing was changed.".to_string(),
        );
    }
    Ok(Some(metadata.mode() & 0o777))
}

fn create_bound_external_write_temp(
    parent: &fs::File,
    permissions: Option<u32>,
) -> Result<(fs::File, CString), String> {
    for _ in 0..8 {
        let name = CString::new(format!(
            ".oomu-write-{}-{}.tmp",
            std::process::id(),
            new_approval_token()
        ))
        .map_err(|_| "The temporary file name is not valid.".to_string())?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                permissions.unwrap_or(0o600),
            )
        };
        if fd >= 0 {
            return Ok((unsafe { fs::File::from_raw_fd(fd) }, name));
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists {
            break;
        }
    }
    Err(
        "OOMU couldn't prepare a safe temporary file. The original file was not changed."
            .to_string(),
    )
}

fn bound_target_identity(
    parent: &fs::File,
    name: &CString,
) -> Result<Option<ApprovedFileIdentity>, String> {
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Ok(None);
    }
    let file = unsafe { fs::File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|_| "The approved file could not be checked.".to_string())?;
    metadata
        .is_file()
        .then(|| approved_file_identity(&metadata))
        .map(Some)
        .ok_or_else(|| "The approved file changed before OOMU could save it.".to_string())
}

fn remove_bound_external_write_temp(
    parent: &fs::File,
    name: &CString,
    prepared_identity: ApprovedFileIdentity,
) {
    if bound_target_identity(parent, name).ok().flatten() == Some(prepared_identity) {
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0);
        }
    }
}

pub(super) fn commit_bound_external_write_temp(
    parent: &fs::File,
    temporary_name: &CString,
    target_name: &CString,
    expected_target_identity: Option<ApprovedFileIdentity>,
    prepared_identity: ApprovedFileIdentity,
) -> Result<(), String> {
    if let Some(expected_target_identity) = expected_target_identity {
        #[cfg(target_os = "macos")]
        let exchanged = unsafe {
            libc::renameatx_np(
                parent.as_raw_fd(),
                temporary_name.as_ptr(),
                parent.as_raw_fd(),
                target_name.as_ptr(),
                libc::RENAME_SWAP,
            )
        };
        #[cfg(not(target_os = "macos"))]
        let exchanged = -1;

        if exchanged != 0 {
            return Err(
                "The approved file changed before OOMU could save it. The original file was not changed."
                    .to_string(),
            );
        }
        let published_identity = bound_target_identity(parent, target_name)?;
        let displaced_identity = bound_target_identity(parent, temporary_name)?;
        if published_identity == Some(prepared_identity)
            && displaced_identity == Some(expected_target_identity)
        {
            unsafe {
                libc::unlinkat(parent.as_raw_fd(), temporary_name.as_ptr(), 0);
            }
            return Ok(());
        }

        // The target changed in the final race window. Exchange the names back
        // only while both still identify the files we just exchanged, so an
        // unrelated later change is never overwritten during rollback.
        if published_identity == Some(prepared_identity)
            && bound_target_identity(parent, temporary_name)? == displaced_identity
        {
            #[cfg(target_os = "macos")]
            unsafe {
                libc::renameatx_np(
                    parent.as_raw_fd(),
                    temporary_name.as_ptr(),
                    parent.as_raw_fd(),
                    target_name.as_ptr(),
                    libc::RENAME_SWAP,
                );
            }
        }
    } else {
        // A same-directory hard link publishes a new file atomically without
        // replacing a target that appeared after approval.
        let linked = unsafe {
            libc::linkat(
                parent.as_raw_fd(),
                temporary_name.as_ptr(),
                parent.as_raw_fd(),
                target_name.as_ptr(),
                0,
            )
        };
        if linked == 0 {
            remove_bound_external_write_temp(parent, temporary_name, prepared_identity);
            return Ok(());
        }
    }
    Err(
        "The approved file changed before OOMU could save it. The original file was not changed."
            .to_string(),
    )
}

pub(super) fn write_bound_external_target_atomically(
    request: &ApprovedExternalWriteRequest,
) -> Result<usize, String> {
    let (parent, target_name) = open_bound_external_write_parent(request)?;
    let permissions = verify_bound_external_write_target(
        &parent,
        &target_name,
        request.expected_target_identity,
    )?;
    let (mut temporary, temporary_name) = create_bound_external_write_temp(&parent, permissions)?;
    let prepared_identity = approved_file_identity(
        &temporary
            .metadata()
            .map_err(|_| "OOMU couldn't verify the prepared file.".to_string())?,
    );
    let result = (|| {
        temporary
            .write_all(request.content.as_bytes())
            .and_then(|()| temporary.flush())
            .and_then(|()| temporary.sync_all())
            .map_err(|_| {
                "The approved external file write failed. The original file was not changed."
                    .to_string()
            })?;

        temporary.seek(SeekFrom::Start(0)).map_err(|_| {
            "OOMU couldn't verify the prepared file. The original file was not changed.".to_string()
        })?;
        let mut actual = Vec::with_capacity(request.content.len());
        temporary.read_to_end(&mut actual).map_err(|_| {
            "OOMU couldn't verify the prepared file. The original file was not changed.".to_string()
        })?;
        if actual != request.content.as_bytes() {
            return Err(
                "OOMU couldn't verify the prepared file. The original file was not changed."
                    .to_string(),
            );
        }

        verify_bound_external_write_target(
            &parent,
            &target_name,
            request.expected_target_identity,
        )?;
        commit_bound_external_write_temp(
            &parent,
            &temporary_name,
            &target_name,
            request.expected_target_identity,
            prepared_identity,
        )?;
        let _ = parent.sync_all();
        Ok(request.content.len())
    })();
    if result.is_err() {
        remove_bound_external_write_temp(&parent, &temporary_name, prepared_identity);
    }
    result
}
