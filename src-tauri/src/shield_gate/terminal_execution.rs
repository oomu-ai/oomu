use super::*;

pub(super) fn terminal_request_for_action(
    action: &RequestedAction,
) -> Result<crate::tools::terminal_contract::NativeTerminalRequest, ShieldGateError> {
    let kind = normalize_action_kind(&action.kind);
    if kind == "terminal_execute" {
        let payload = action.content.as_deref().ok_or_else(|| {
            security_boundary_violation("A typed terminal request is required.".to_string())
        })?;
        let request =
            serde_json::from_str::<crate::tools::terminal_contract::NativeTerminalRequest>(payload)
                .map_err(|_| {
                    security_boundary_violation("The terminal request was not valid.".to_string())
                })?
                .validate()
                .map_err(security_boundary_violation);
        return request.and_then(|request| {
            request
                .validate_protected_deletion_roots(&[
                    crate::settings::app_data_root(),
                    development_repo_root(),
                ])
                .map_err(security_boundary_violation)?;
            Ok(request)
        });
    }

    let command = action
        .content
        .as_deref()
        .or(action.path.as_deref())
        .or(action.principal.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            security_boundary_violation("A terminal command is required.".to_string())
        })?;
    super::terminal_runtime::direct_command_request(command)
        .map_err(security_boundary_violation)
        .and_then(|request| {
            request
                .validate_protected_deletion_roots(&[
                    crate::settings::app_data_root(),
                    development_repo_root(),
                ])
                .map_err(security_boundary_violation)?;
            Ok(request)
        })
}

pub(super) fn handle_approved_system_execution(
    request: SystemExecutionRequest,
) -> ExecuteCommandResponse {
    let prepared = match prepare_terminal_execution(request) {
        Ok(prepared) => prepared,
        Err(message) => return terminal_error(message),
    };
    let (output, timed_out) = match run_terminal_process(&prepared) {
        Ok(result) => result,
        Err(message) => return terminal_error(message),
    };
    build_terminal_response(prepared, output, timed_out)
}

struct PreparedTerminalExecution {
    request: SystemExecutionRequest,
    working_directory: PathBuf,
    timeout: Duration,
    request_hash: String,
    display_command: String,
    environment_keys: String,
}

fn prepare_terminal_execution(
    request: SystemExecutionRequest,
) -> Result<PreparedTerminalExecution, String> {
    let mut request = request.validate()?;
    request.validate_protected_deletion_roots(&[
        crate::settings::app_data_root(),
        development_repo_root(),
    ])?;
    let working_directory = request
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| "Choose a Project folder before running a local command.".to_string())?
        .canonicalize()
        .ok()
        .filter(|path| path.is_dir())
        .ok_or_else(|| "The selected working folder is not available.".to_string())?;
    request.executable = super::terminal_runtime::resolve_terminal_executable(
        &request.executable,
        &working_directory,
    )?
    .display()
    .to_string();
    let request_hash =
        crate::foundation::digest::sha256_hex(&serde_json::to_vec(&request).unwrap_or_default());
    Ok(PreparedTerminalExecution {
        timeout: Duration::from_millis(request.timeout_ms()),
        display_command: request.display_command(),
        environment_keys: request.environment_keys().join(","),
        request,
        working_directory,
        request_hash,
    })
}

fn run_terminal_process(
    prepared: &PreparedTerminalExecution,
) -> Result<(std::process::Output, bool), String> {
    let mut command = Command::new(&prepared.request.executable);
    command
        .args(&prepared.request.args)
        .envs(&prepared.request.env)
        .env(
            "PATH",
            super::terminal_runtime::deterministic_terminal_path(),
        )
        .current_dir(&prepared.working_directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    harden_prompt_free_git_environment(&mut command, &prepared.request);
    let mut child = command
        .spawn()
        .map_err(|error| format!("The approved terminal command could not start: {error}"))?;
    let started = Instant::now();
    let mut timed_out = false;
    let output = loop {
        match child.try_wait() {
            Ok(Some(_status)) => break child.wait_with_output(),
            Ok(None) if started.elapsed() >= prepared.timeout => {
                timed_out = true;
                let _ = child.kill();
                break child.wait_with_output();
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => {
                return Err(format!(
                    "OOMU could not verify the approved command status: {error}"
                ))
            }
        }
    };
    output
        .map(|output| (output, timed_out))
        .map_err(|error| format!("OOMU could not collect the approved command result: {error}"))
}

fn harden_prompt_free_git_environment(command: &mut Command, request: &SystemExecutionRequest) {
    let is_git = Path::new(&request.executable)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("git"));
    if !is_git
        || request.classification().tier != crate::tool_security::CapabilityRiskTier::ReadOnly
    {
        return;
    }

    // Read-only Git commands still consult user and repository configuration.
    // Disable hooks, file-monitor helpers, pagers, and external diff programs so
    // a repository cannot turn a prompt-free inspection into process execution.
    for key in [
        "GIT_CONFIG_PARAMETERS",
        "GIT_EXTERNAL_DIFF",
        "GIT_PAGER",
        "PAGER",
    ] {
        command.env_remove(key);
    }
    command.env("GIT_OPTIONAL_LOCKS", "0");
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    command.env("GIT_CONFIG_GLOBAL", "/dev/null");
    command.env("GIT_PAGER", "");
    command.env("GIT_CONFIG_COUNT", "3");
    command.env("GIT_CONFIG_KEY_0", "core.fsmonitor");
    command.env("GIT_CONFIG_VALUE_0", "false");
    command.env("GIT_CONFIG_KEY_1", "core.hooksPath");
    command.env("GIT_CONFIG_VALUE_1", "/dev/null");
    command.env("GIT_CONFIG_KEY_2", "diff.external");
    command.env("GIT_CONFIG_VALUE_2", "");
}

fn build_terminal_response(
    prepared: PreparedTerminalExecution,
    output: std::process::Output,
    timed_out: bool,
) -> ExecuteCommandResponse {
    let exit_success = output.status.success();
    let exit_status = output
        .status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string());
    let stdout = crate::redaction::redacted_log_text(&truncate_for_receipt(
        &String::from_utf8_lossy(&output.stdout),
        1200,
    ));
    let stderr = crate::redaction::redacted_log_text(&truncate_for_receipt(
        &String::from_utf8_lossy(&output.stderr),
        1200,
    ));
    let status = if timed_out || !exit_success {
        CommandStatus::Failed
    } else {
        CommandStatus::Completed
    };
    let cwd_after = prepared.working_directory.canonicalize().ok();
    let pwd_postcondition = Path::new(&prepared.request.executable)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("pwd"))
        .then(|| {
            PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
                .canonicalize()
                .ok()
        })
        .flatten();
    let postcondition_verified = exit_success
        && !timed_out
        && cwd_after.as_deref() == Some(prepared.working_directory.as_path())
        && pwd_postcondition
            .as_deref()
            .is_none_or(|observed| observed == prepared.working_directory.as_path());
    let mut message = if timed_out {
        format!(
            "The approved terminal command was stopped after {} seconds.",
            prepared.timeout.as_secs()
        )
    } else if exit_success {
        format!("The terminal command finished successfully. Exit status: {exit_status}.")
    } else {
        format!("The terminal command exited with status {exit_status}.")
    };
    if !stdout.trim().is_empty() {
        message.push_str(&format!("\nstdout:\n{stdout}"));
    }
    if !stderr.trim().is_empty() {
        message.push_str(&format!("\nstderr:\n{stderr}"));
    }
    let verified = matches!(&status, CommandStatus::Completed) && postcondition_verified;

    ExecuteCommandResponse {
        operation: "terminal_execute".to_string(),
        status,
        message,
        metrics: None,
        claims: vec![format!(
            "CLAIM native_terminal_receipt schema=oomu.native_terminal.v1 evidence_kind=observed_native request_sha256={} command_b64={} cwd_b64={} env_keys_b64={} exit_status={exit_status} timed_out={timed_out} postcondition_verified={postcondition_verified} direct_process=true",
            prepared.request_hash,
            URL_SAFE_NO_PAD.encode(prepared.display_command.as_bytes()),
            URL_SAFE_NO_PAD.encode(prepared.working_directory.to_string_lossy().as_bytes()),
            URL_SAFE_NO_PAD.encode(prepared.environment_keys.as_bytes()),
        )],
        verified,
        model_used: None,
    }
}

fn terminal_error(message: String) -> ExecuteCommandResponse {
    ExecuteCommandResponse::from_tool_error(ToolError {
        operation: "terminal_execute".to_string(),
        message,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};

    #[test]
    fn prompt_free_git_status_cannot_execute_a_repository_fsmonitor_hook() {
        let root = std::env::temp_dir().join(format!(
            "oomu-terminal-git-hardening-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ms_i64(),
        ));
        fs::create_dir_all(&root).unwrap();
        let marker = root.join("fsmonitor-ran");
        let helper = root.join("fsmonitor-helper.sh");
        fs::write(
            &helper,
            format!("#!/bin/sh\nprintf called > '{}'\n", marker.display()),
        )
        .unwrap();
        let mut permissions = fs::metadata(&helper).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&helper, permissions).unwrap();

        assert!(Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["config", "core.fsmonitor", helper.to_str().unwrap()])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());

        let prepared = prepare_terminal_execution(SystemExecutionRequest {
            executable: "git".to_string(),
            args: vec!["status".to_string(), "--short".to_string()],
            env: BTreeMap::new(),
            cwd: Some(root.display().to_string()),
            timeout: Some(10_000),
        })
        .unwrap();
        let (output, timed_out) = run_terminal_process(&prepared).unwrap();

        assert!(output.status.success());
        assert!(!timed_out);
        assert!(!marker.exists(), "repository fsmonitor helper must not run");
        let _ = fs::remove_dir_all(root);
    }
}
