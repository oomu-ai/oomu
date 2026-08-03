use super::*;
use crate::tools::terminal_contract::NativeTerminalRequest;
use std::collections::{BTreeMap, HashSet};

pub(super) fn direct_command_request(command: &str) -> Result<NativeTerminalRequest, String> {
    let mut words = direct_command_words(command)?;
    if words.is_empty() {
        return Err("A terminal command is required.".to_string());
    }
    NativeTerminalRequest {
        executable: words.remove(0),
        args: words,
        env: BTreeMap::new(),
        cwd: None,
        timeout: None,
    }
    .validate()
}

pub(super) fn begin(
    persistence: &PersistenceEngine,
    request: &mut ExecuteCommandRequest,
) -> Result<Option<DirectCommandTurnGuard>, ShieldGateError> {
    bind_direct_terminal_scope(persistence, request)?;
    DirectCommandTurnGuard::begin(persistence, request)
}

pub(super) fn bind_direct_terminal_scope(
    persistence: &PersistenceEngine,
    request: &mut ExecuteCommandRequest,
) -> Result<(), ShieldGateError> {
    let kind = normalize_action_kind(&request.action.kind);
    if !matches!(
        kind.as_str(),
        "terminal_execute" | "shell_command" | "execute_command"
    ) {
        return Ok(());
    }
    let session_id = required_terminal_binding("the conversation", request.session_id.as_deref())?;
    let session = persistence
        .select_chat_session_by_id(&session_id)
        .map_err(|_| {
            project_root_error(
                "OOMU could not verify this conversation's terminal scope.".to_string(),
            )
        })?;
    let mut terminal = super::terminal_request(&request.action)?;
    expand_terminal_home_paths(&mut terminal)?;
    let working_directory = match request.project_id.as_deref() {
        Some(project_id) => project_terminal_working_directory(
            persistence,
            &session,
            project_id,
            terminal.cwd.as_deref(),
        )?,
        None => global_terminal_working_directory(&session, &terminal)?,
    };
    terminal.cwd = Some(working_directory.display().to_string());
    request.action = RequestedAction {
        kind: "terminal_execute".to_string(),
        principal: None,
        path: None,
        content: Some(serde_json::to_string(&terminal).map_err(|_| {
            project_root_error("OOMU could not bind the command to its Project folder.".to_string())
        })?),
    };
    Ok(())
}

fn required_terminal_binding(label: &str, value: Option<&str>) -> Result<String, ShieldGateError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            project_root_error(format!(
                "OOMU could not verify this command because {label} is missing."
            ))
        })
}

fn project_terminal_working_directory(
    persistence: &PersistenceEngine,
    session: &crate::db::ChatSessionRecord,
    project_id: &str,
    requested_cwd: Option<&str>,
) -> Result<PathBuf, ShieldGateError> {
    if session.project_id.as_deref() != Some(project_id) {
        return Err(project_root_error(
            "This conversation is not bound to the selected Project. Choose the Project again before running the command."
                .to_string(),
        ));
    }
    let root = crate::projects::path_scope::single_active_project_root(persistence, project_id)
        .map_err(project_root_error)?;
    bounded_terminal_working_directory(
        requested_cwd,
        &root,
        "The command folder must stay inside the selected Project.",
    )
}

fn global_terminal_working_directory(
    session: &crate::db::ChatSessionRecord,
    terminal: &NativeTerminalRequest,
) -> Result<PathBuf, ShieldGateError> {
    if session.project_id.is_some() {
        return Err(project_root_error(
            "This conversation is bound to a Project, but the command omitted that Project."
                .to_string(),
        ));
    }
    if !matches!(
        terminal.classification().tier,
        crate::tool_security::CapabilityRiskTier::ReadOnly
            | crate::tool_security::CapabilityRiskTier::FileRead
    ) {
        return Err(project_root_error(
            "Choose a Project before running commands that can change files, execute code, or use the network."
                .to_string(),
        ));
    }
    let home = dirs::home_dir()
        .and_then(|path| path.canonicalize().ok())
        .filter(|path| path.is_dir())
        .ok_or_else(|| {
            project_root_error(
                "OOMU could not verify the home folder for this read-only command.".to_string(),
            )
        })?;
    bounded_terminal_working_directory(
        terminal.cwd.as_deref(),
        &home,
        "A global-chat command folder must stay inside your home folder.",
    )
}

fn bounded_terminal_working_directory(
    requested_cwd: Option<&str>,
    root: &Path,
    boundary_message: &str,
) -> Result<PathBuf, ShieldGateError> {
    let Some(cwd) = requested_cwd else {
        return Ok(root.to_path_buf());
    };
    let requested = Path::new(cwd);
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let canonical = candidate.canonicalize().map_err(|_| {
        project_root_error("The requested command folder is unavailable.".to_string())
    })?;
    if !canonical.is_dir() || !canonical.starts_with(root) {
        return Err(project_root_error(boundary_message.to_string()));
    }
    Ok(canonical)
}

fn expand_terminal_home_paths(request: &mut NativeTerminalRequest) -> Result<(), ShieldGateError> {
    if let Some(cwd) = request.cwd.as_mut() {
        expand_terminal_home_value(cwd)?;
    }
    for argument in &mut request.args {
        expand_terminal_home_value(argument)?;
    }
    Ok(())
}

fn expand_terminal_home_value(value: &mut String) -> Result<(), ShieldGateError> {
    if value != "~" && !value.starts_with("~/") {
        return Ok(());
    }
    let expanded = expand_shield_home_path(value, "terminal_execute")?;
    if expanded
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(security_boundary_violation(
            "terminal_execute rejected home-relative path traversal.".to_string(),
        ));
    }
    *value = expanded.display().to_string();
    Ok(())
}

fn project_root_error(message: String) -> ShieldGateError {
    ShieldGateError {
        code: "project_root_required",
        boundary: "ProjectTerminalScope",
        message,
    }
}

pub(super) fn resolve_terminal_executable(
    executable: &str,
    working_directory: &Path,
) -> Result<PathBuf, String> {
    let project_root = working_directory
        .canonicalize()
        .map_err(|_| "The selected Project folder is unavailable.".to_string())?;
    let requested = Path::new(executable);
    if requested.is_absolute() {
        return checked_executable(requested);
    }
    if requested.components().count() > 1 {
        if requested
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(
                "The command executable cannot leave the selected Project folder.".to_string(),
            );
        }
        let candidate = project_root.join(requested);
        let resolved = checked_executable(&candidate)?;
        if !resolved.starts_with(&project_root) {
            return Err(
                "The command executable cannot leave the selected Project folder.".to_string(),
            );
        }
        return Ok(resolved);
    }
    terminal_search_directories()
        .into_iter()
        .map(|directory| directory.join(requested))
        .find_map(|candidate| checked_executable(&candidate).ok())
        .ok_or_else(|| {
            format!(
                "The '{}' command is not installed in a verified command location.",
                executable.trim()
            )
        })
}

pub(super) fn deterministic_terminal_path() -> OsString {
    std::env::join_paths(terminal_search_directories())
        .unwrap_or_else(|_| OsString::from("/usr/bin:/bin:/usr/sbin:/sbin"))
}

fn terminal_search_directories() -> Vec<PathBuf> {
    let mut directories = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/local/sbin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/sbin"),
    ];
    if let Some(home) = dirs::home_dir() {
        for relative in [
            ".cargo/bin",
            ".local/bin",
            ".volta/bin",
            ".bun/bin",
            ".nvm/current/bin",
            ".asdf/shims",
            ".local/share/mise/shims",
        ] {
            directories.push(home.join(relative));
        }
    }
    directories.extend(
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .filter(|path| path.is_absolute()),
    );
    let mut seen = HashSet::new();
    directories.retain(|path| seen.insert(path.clone()));
    directories
}

fn checked_executable(path: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path)
        .map_err(|_| "The requested command executable is unavailable.".to_string())?;
    let metadata = fs::metadata(&canonical)
        .map_err(|_| "The requested command executable is unavailable.".to_string())?;
    if !metadata.is_file() {
        return Err("The requested command executable is not a file.".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("The requested command file is not executable.".to_string());
        }
    }
    Ok(canonical)
}

fn direct_command_words(command: &str) -> Result<Vec<String>, String> {
    if command.chars().any(|character| {
        matches!(
            character,
            '&' | '|' | ';' | '`' | '\n' | '\r' | '>' | '<' | '$'
        )
    }) {
        return Err(
            "Run one executable at a time; shell operators and substitutions are not accepted."
                .to_string(),
        );
    }
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in command.trim().chars() {
        if escaped {
            word.push(character);
            escaped = false;
            continue;
        }
        match (quote, character) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (Some('\''), _) => word.push(character),
            (_, '\\') => escaped = true,
            (None, character) if character.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            (_, character) => word.push(character),
        }
    }
    if escaped || quote.is_some() {
        return Err("The command contains an unfinished quote or escape.".to_string());
    }
    if !word.is_empty() {
        words.push(word);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct_request(
        session: &crate::db::ChatSessionRecord,
        command: &str,
        project_id: Option<String>,
    ) -> ExecuteCommandRequest {
        ExecuteCommandRequest {
            action: RequestedAction {
                kind: "shell_command".to_string(),
                principal: None,
                path: None,
                content: Some(command.to_string()),
            },
            logical_certificate: None,
            session_id: Some(session.id.clone()),
            turn_id: Some("turn-terminal".to_string()),
            generation_token: Some("generation-terminal".to_string()),
            agent_id: Some(session.agent_id.clone()),
            provider_id: Some(session.provider_id.clone()),
            model_id: Some(session.model_id.clone()),
            parent_turn_id: None,
            root_turn_id: Some("turn-terminal".to_string()),
            turn_kind: Some("root".to_string()),
            project_id,
            task_run_id: None,
        }
    }

    #[test]
    fn direct_command_text_becomes_one_typed_process_without_a_shell() {
        let request = direct_command_request("git status --short").unwrap();
        assert_eq!(request.executable, "git");
        assert_eq!(request.args, ["status", "--short"]);
        let quoted = direct_command_request("printf 'hello world'").unwrap();
        assert_eq!(quoted.executable, "printf");
        assert_eq!(quoted.args, ["hello world"]);
        assert!(direct_command_request("pwd && whoami").is_err());
        assert!(direct_command_request("echo $(whoami)").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn executable_resolution_accepts_a_verified_project_script() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "oomu-terminal-resolution-{}-{}",
            std::process::id(),
            unix_time_ms_i64()
        ));
        fs::create_dir_all(&root).unwrap();
        let script = root.join("tool");
        fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&script, permissions).unwrap();
        assert_eq!(
            resolve_terminal_executable("./tool", &root).unwrap(),
            fs::canonicalize(&script).unwrap()
        );
        assert_eq!(
            resolve_terminal_executable("pwd", &root)
                .unwrap()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("pwd")
        );
        assert!(resolve_terminal_executable("../tool", &root).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn direct_command_binds_to_the_conversations_verified_project_root() {
        use crate::projects::{BindProjectRecordRequest, CreateProjectRequest, ProjectDataPolicy};
        let root = std::env::temp_dir().join(format!(
            "oomu-terminal-project-root-{}-{}",
            std::process::id(),
            unix_time_ms_i64()
        ));
        let selected = root.join("selected");
        fs::create_dir_all(&selected).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let project = crate::projects::repository::create(
            &persistence,
            CreateProjectRequest {
                name: "Terminal Project".to_string(),
                description: String::new(),
                data_policy: ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let selected = fs::canonicalize(selected).unwrap();
        persistence
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO project_sources (source_id, project_id, source_kind, canonical_path, grant_reference, grant_state, indexing_state, file_count, created_at_ms, updated_at_ms) VALUES (?1, ?2, 'local_folder', ?3, ?4, 'active', 'ready', 0, ?5, ?5)",
                rusqlite::params![
                    "source_terminal_test",
                    project.project_id,
                    selected.to_string_lossy(),
                    "0".repeat(64),
                    unix_time_ms_i64(),
                ],
            )
            .unwrap();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-terminal".to_string(),
                provider_id: "provider-terminal".to_string(),
                model_id: "model-terminal".to_string(),
                title: Some("Terminal".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        crate::projects::repository::bind_record(
            &persistence,
            BindProjectRecordRequest {
                project_id: Some(project.project_id.clone()),
                record_kind: "chat_session".to_string(),
                record_id: session.id.clone(),
            },
        )
        .unwrap();
        let mut request = ExecuteCommandRequest {
            action: RequestedAction {
                kind: "shell_command".to_string(),
                principal: None,
                path: None,
                content: Some("npm test".to_string()),
            },
            logical_certificate: None,
            session_id: Some(session.id),
            turn_id: Some("turn-terminal".to_string()),
            generation_token: Some("generation-terminal".to_string()),
            agent_id: Some(session.agent_id),
            provider_id: Some(session.provider_id),
            model_id: Some(session.model_id),
            parent_turn_id: None,
            root_turn_id: Some("turn-terminal".to_string()),
            turn_kind: Some("root".to_string()),
            project_id: Some(project.project_id),
            task_run_id: None,
        };
        bind_direct_terminal_scope(&persistence, &mut request).unwrap();
        let bound = super::super::terminal_request(&request.action).unwrap();
        assert_eq!(bound.executable, "npm");
        assert_eq!(bound.args, ["test"]);
        assert_eq!(bound.cwd, Some(selected.display().to_string()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn global_chat_allows_only_home_scoped_read_only_terminal_commands() {
        let root = std::env::temp_dir().join(format!(
            "oomu-global-terminal-scope-{}-{}",
            std::process::id(),
            unix_time_ms_i64()
        ));
        fs::create_dir_all(&root).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-global-terminal".to_string(),
                provider_id: "provider-global-terminal".to_string(),
                model_id: "model-global-terminal".to_string(),
                title: Some("Global terminal".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let home = dirs::home_dir()
            .and_then(|path| path.canonicalize().ok())
            .unwrap();
        let downloads = home.join("Downloads");
        let mut listing = direct_request(&session, "ls ~/Downloads", None);

        bind_direct_terminal_scope(&persistence, &mut listing).unwrap();

        let bound = super::super::terminal_request(&listing.action).unwrap();
        assert_eq!(bound.executable, "ls");
        assert_eq!(bound.args, [downloads.display().to_string()]);
        assert_eq!(bound.cwd, Some(home.display().to_string()));
        let approval = super::super::build_shield_approval_request(&listing.action)
            .expect("Downloads read requires visual consent");
        assert_eq!(approval.action_class, "filesystem_read");
        assert_eq!(approval.target_path, Some(downloads.display().to_string()));

        for command in [
            "touch changed.txt",
            "node --version",
            "curl https://example.com",
        ] {
            let mut rejected = direct_request(&session, command, None);
            let error = bind_direct_terminal_scope(&persistence, &mut rejected)
                .expect_err("non-read-only global terminal work must stay Project-bound");
            assert_eq!(error.code, "project_root_required");
        }
        let _ = fs::remove_dir_all(root);
    }
}
