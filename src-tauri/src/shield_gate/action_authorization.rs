use super::*;

pub(super) fn authorize(
    action: RequestedAction,
    context: &ShieldAuthorizationContext,
) -> Result<AuthorizedActions, ShieldGateError> {
    let action = normalize_directory_read_action(action);
    let kind = action.kind.clone();
    match kind.as_str() {
        "get_system_metrics" => authorize_system_metrics(action),
        "file_read" => authorize_file_read(action, context),
        "file_write" => authorize_file_write(action, context),
        "telemetry_archive" => authorize_telemetry_archive(action, context),
        "delete_file" | "trash" | "trash_file" => authorize_file_delete(action, context),
        "codebase_patch" => authorize_codebase_patch(action),
        "codebase_compile" => authorize_codebase_compile(action),
        "file_list" => authorize_file_list(action, context),
        "create_file" | "prepare_release_recovery_agenda" => {
            authorize_registered_file_creation(action, context)
        }
        operation if crate::tools::task_tool_runtime::is_registered(operation) => {
            system_action_semantics::authorize_registered(action, context.shield_approved)
        }
        "system_audit" => Ok(AuthorizedActions::SystemAudit(SystemAuditRequest {
            scope: action
                .principal
                .unwrap_or_else(|| "process_disk_network".to_string()),
        })),
        "web_fetch" => authorize_web_fetch(action),
        "document_index" => authorize_document_index(action),
        "ask_local_document_index" => authorize_local_document_question(action),
        "sovereign_duckduckgo_search" | "duckduckgo_search" => authorize_search(action),
        "airlock_export" => authorize_airlock_export(action),
        "terminal_execute" | "shell_command" | "execute_command" => {
            authorize_terminal(action, context)
        }
        "network_request" => Err(rejected(
            "network_request is structurally recognized but has no deterministic host execution implementation.",
        )),
        _ => Err(rejected(format!(
            "{} is not present in the deterministic allowlist.",
            action.kind
        ))),
    }
}

fn authorize_system_metrics(action: RequestedAction) -> Result<AuthorizedActions, ShieldGateError> {
    let principal = action
        .principal
        .ok_or_else(|| invalid("get_system_metrics requires a principal string."))?;
    Ok(AuthorizedActions::GetSystemMetrics(SystemMetricsRequest {
        principal,
    }))
}

fn authorize_file_read(
    action: RequestedAction,
    context: &ShieldAuthorizationContext,
) -> Result<AuthorizedActions, ShieldGateError> {
    let path = action
        .path
        .ok_or_else(|| invalid("file_read requires a path string."))?;
    match resolve_read_only_action_path("file_read", &path) {
        Ok(path) => Ok(AuthorizedActions::FileRead(FileReadRequest { path })),
        Err(_) if context.shield_approved => Ok(AuthorizedActions::ApprovedExternalFileRead(
            prepare_approved_external_read_target(&path, false)?,
        )),
        Err(error) => Err(error),
    }
}

fn authorize_file_write(
    action: RequestedAction,
    context: &ShieldAuthorizationContext,
) -> Result<AuthorizedActions, ShieldGateError> {
    let path = action
        .path
        .ok_or_else(|| invalid("file_write requires a path string."))?;
    let content = action
        .content
        .ok_or_else(|| invalid("file_write requires a content string."))?;
    match validate_project_quarantine(&path, "file_write") {
        Ok(()) => Ok(AuthorizedActions::FileWrite(FileWriteRequest {
            path,
            content,
        })),
        Err(_) if context.shield_approved => Ok(AuthorizedActions::ApprovedExternalFileWrite(
            prepare_approved_external_write_target(&path, content)?,
        )),
        Err(error) => Err(error),
    }
}

fn authorize_telemetry_archive(
    action: RequestedAction,
    context: &ShieldAuthorizationContext,
) -> Result<AuthorizedActions, ShieldGateError> {
    require_approval(
        context,
        "telemetry_archive requires explicit Shield Gate approval.",
    )?;
    let output_path = action
        .path
        .or(action.content)
        .or(action.principal)
        .ok_or_else(|| invalid("telemetry_archive requires an output_path string."))?;
    let output_path = validate_approved_external_write_target(&output_path)?;
    Ok(AuthorizedActions::TelemetryArchive(
        TelemetryArchiveRequest {
            output_path: output_path.display().to_string(),
        },
    ))
}

fn authorize_file_delete(
    action: RequestedAction,
    context: &ShieldAuthorizationContext,
) -> Result<AuthorizedActions, ShieldGateError> {
    require_approval(
        context,
        "delete_file requires explicit Shield Gate approval.",
    )?;
    let path = action
        .path
        .or(action.content)
        .or(action.principal)
        .ok_or_else(|| invalid("delete_file requires a path string."))?;
    let path = validate_approved_file_delete_target(&path)?;
    Ok(AuthorizedActions::ApprovedFileDelete(FileDeleteRequest {
        path: path.display().to_string(),
    }))
}

pub(super) fn validate_approved_file_delete_target(path: &str) -> Result<PathBuf, ShieldGateError> {
    let requested = expand_shield_home_path(path, "delete_file")?;
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(security_boundary_violation(
            "Approved delete_file rejected path traversal.".to_string(),
        ));
    }

    let candidate = if requested.is_absolute() {
        requested
    } else {
        project_root().join(requested)
    };
    let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ShieldGateError {
                code: "delete_target_not_found",
                boundary: "DeleteFileAuthority",
                message: "The requested file is not there.".to_string(),
            }
        } else {
            security_boundary_violation(
                "The requested file could not be checked safely.".to_string(),
            )
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(security_boundary_violation(
            "delete_file rejected a symbolic link target.".to_string(),
        ));
    }
    if !metadata.is_file() {
        return Err(security_boundary_violation(
            "delete_file target must be a regular file.".to_string(),
        ));
    }

    let canonical_target = fs::canonicalize(&candidate).map_err(|_| {
        security_boundary_violation("delete_file target could not be resolved.".to_string())
    })?;
    let safe_roots = shield_file_delete_safe_roots();
    if safe_roots
        .iter()
        .any(|root| canonical_target == *root || canonical_target.starts_with(root))
    {
        return Ok(canonical_target);
    }

    Err(security_boundary_violation(
        "delete_file rejected a path outside approved local roots.".to_string(),
    ))
}

fn authorize_codebase_patch(action: RequestedAction) -> Result<AuthorizedActions, ShieldGateError> {
    let target_file_path = action
        .path
        .ok_or_else(|| invalid("codebase_patch requires a target_file_path string."))?;
    let search_pattern = action
        .principal
        .ok_or_else(|| invalid("codebase_patch requires a search_pattern string."))?;
    let replacement_content = action
        .content
        .ok_or_else(|| invalid("codebase_patch requires a replacement_content string."))?;
    if search_pattern.trim().is_empty() {
        return Err(invalid("codebase_patch search_pattern cannot be empty."));
    }
    validate_codebase_patch_target(&target_file_path)?;
    Ok(AuthorizedActions::CodebasePatch(CodebasePatchRequest {
        target_file_path,
        search_pattern,
        replacement_content,
    }))
}

fn authorize_codebase_compile(
    action: RequestedAction,
) -> Result<AuthorizedActions, ShieldGateError> {
    let target = action
        .principal
        .or(action.path)
        .or(action.content)
        .ok_or_else(|| invalid("codebase_compile requires a target string."))?;
    let target = CodebaseCompileTarget::parse(&target)?;
    validate_codebase_compile_root()?;
    Ok(AuthorizedActions::CodebaseCompile(CodebaseCompileRequest {
        target,
    }))
}

fn authorize_file_list(
    action: RequestedAction,
    context: &ShieldAuthorizationContext,
) -> Result<AuthorizedActions, ShieldGateError> {
    let path = action
        .path
        .ok_or_else(|| invalid("file_list requires a path string."))?;
    match resolve_read_only_action_path("file_list", &path) {
        Ok(path) => Ok(AuthorizedActions::FileList(FileListRequest { path })),
        Err(_) if context.shield_approved => Ok(AuthorizedActions::ApprovedExternalFileList(
            prepare_approved_external_read_target(&path, true)?,
        )),
        Err(error) => Err(error),
    }
}

fn authorize_registered_file_creation(
    action: RequestedAction,
    context: &ShieldAuthorizationContext,
) -> Result<AuthorizedActions, ShieldGateError> {
    require_approval(context, "Creating this file requires your approval.")?;
    let path = action
        .path
        .as_deref()
        .ok_or_else(|| invalid("File creation requires an exact destination path."))?;
    validate_approved_external_write_target(path)?;
    crate::tools::task_tool_runtime::authorize(action)
        .map(AuthorizedActions::RegisteredTaskTool)
        .map_err(|message| invalid(message))
}

fn authorize_web_fetch(action: RequestedAction) -> Result<AuthorizedActions, ShieldGateError> {
    let url = action
        .path
        .ok_or_else(|| invalid("web_fetch requires a URL string in path."))?;
    validate_web_url(&url)?;
    Ok(AuthorizedActions::WebFetch)
}

fn authorize_document_index(action: RequestedAction) -> Result<AuthorizedActions, ShieldGateError> {
    if let Some(workspace) = action.path.as_deref() {
        validate_project_quarantine(workspace, "document_index")?;
    }
    Ok(AuthorizedActions::DocumentIndex)
}

fn authorize_local_document_question(
    action: RequestedAction,
) -> Result<AuthorizedActions, ShieldGateError> {
    action
        .principal
        .ok_or_else(|| invalid("ask_local_document_index requires a question string."))?;
    Ok(AuthorizedActions::AskLocalDocumentIndex)
}

fn authorize_search(action: RequestedAction) -> Result<AuthorizedActions, ShieldGateError> {
    let query = action
        .principal
        .or(action.path)
        .ok_or_else(|| invalid("sovereign_duckduckgo_search requires a query string."))?
        .trim()
        .to_string();
    if query.is_empty() {
        return Err(invalid(
            "sovereign_duckduckgo_search query cannot be empty.",
        ));
    }
    let max_results = action
        .content
        .as_deref()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(5)
        .clamp(1, 5);
    Ok(AuthorizedActions::SovereignDuckDuckGoSearch(
        SovereignDuckDuckGoSearchRequest {
            query,
            max_results,
            session_id: None,
        },
    ))
}

fn authorize_airlock_export(action: RequestedAction) -> Result<AuthorizedActions, ShieldGateError> {
    let artifact_path = action
        .content
        .ok_or_else(|| invalid("airlock_export requires an Ark artifact path in content."))?;
    validate_project_quarantine(&artifact_path, "airlock_export")?;
    let mount_path = action
        .path
        .ok_or_else(|| invalid("airlock_export requires a secure mount path in path."))?;
    let mission_id = action
        .principal
        .ok_or_else(|| invalid("airlock_export requires a mission id in principal."))?;
    Ok(AuthorizedActions::AirlockExport(AirlockExportRequest {
        artifact_path,
        mount_path,
        mission_id,
    }))
}

fn authorize_terminal(
    action: RequestedAction,
    context: &ShieldAuthorizationContext,
) -> Result<AuthorizedActions, ShieldGateError> {
    let is_direct_command_text = normalize_action_kind(&action.kind) != "terminal_execute";
    let mut request = terminal_request(&action)?;
    if request.cwd.is_none() {
        request.cwd = context.trusted_working_directory.clone();
    }
    let project_root = development_repo_root()
        .canonicalize()
        .unwrap_or_else(|_| development_repo_root());
    if !context.shield_approved
        && (is_direct_command_text || !request.prompt_free_in_project(&project_root))
    {
        return Err(rejected(
            "This terminal command requires approval before it can run.",
        ));
    }
    Ok(AuthorizedActions::ApprovedSystemExecution(request))
}

fn require_approval(
    context: &ShieldAuthorizationContext,
    message: &'static str,
) -> Result<(), ShieldGateError> {
    context
        .shield_approved
        .then_some(())
        .ok_or_else(|| rejected(message))
}

fn invalid(message: impl Into<String>) -> ShieldGateError {
    ShieldGateError {
        code: "shield_gate_invalid_input",
        boundary: "AuthorizedActions",
        message: message.into(),
    }
}

fn rejected(message: impl Into<String>) -> ShieldGateError {
    ShieldGateError {
        code: "shield_gate_rejected",
        boundary: "AuthorizedActions",
        message: message.into(),
    }
}
