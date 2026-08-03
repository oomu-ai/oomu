use super::*;

pub(super) async fn prepare_receipt(
    guard: Option<&DirectCommandTurnGuard>,
    operation: &str,
    action: &RequestedAction,
) -> Result<Option<crate::tools::native_operation_receipt::NativeOperationAttempt>, ShieldGateError>
{
    DirectCommandTurnGuard::prepare_receipt(guard, operation, action).await
}

pub(super) struct DirectCommandTurnGuard {
    persistence: PersistenceEngine,
    pub(super) context: ChatTurnPersistenceContext,
    operation: String,
    action: RequestedAction,
    handed_off: bool,
}

fn direct_command_turn_context(
    persistence: &PersistenceEngine,
    request: &ExecuteCommandRequest,
) -> Result<ChatTurnPersistenceContext, ShieldGateError> {
    let required = |field: &str, value: Option<&str>| -> Result<String, ShieldGateError> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| ShieldGateError {
                code: "chat_turn_context_invalid",
                boundary: "DirectCommandTurnGuard",
                message: format!("Direct chat actions require immutable {field}."),
            })
    };
    let mut context = ChatTurnPersistenceContext {
        turn_id: required("turn_id", request.turn_id.as_deref())?,
        generation_token: required("generation_token", request.generation_token.as_deref())?,
        session_id: required("session_id", request.session_id.as_deref())?,
        agent_id: required("agent_id", request.agent_id.as_deref())?,
        provider_id: required("provider_id", request.provider_id.as_deref())?,
        model_id: required("model_id", request.model_id.as_deref())?,
        parent_turn_id: request
            .parent_turn_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        root_turn_id: required("root_turn_id", request.root_turn_id.as_deref())?,
        turn_kind: required("turn_kind", request.turn_kind.as_deref())?,
    };
    if let Some(parent_turn_id) = context.parent_turn_id.as_deref() {
        let parent = persistence
            .select_chat_turn_context(parent_turn_id)
            .map_err(|error| ShieldGateError {
                code: "chat_turn_context_invalid",
                boundary: "DirectCommandTurnGuard",
                message: error.to_string(),
            })?
            .ok_or_else(|| ShieldGateError {
                code: "chat_turn_context_invalid",
                boundary: "DirectCommandTurnGuard",
                message: "Direct chat action parent does not exist.".to_string(),
            })?;
        let provider_matches = context.provider_id.eq_ignore_ascii_case("dynamic")
            || context.provider_id == parent.provider_id;
        let model_matches =
            context.model_id.eq_ignore_ascii_case("dynamic") || context.model_id == parent.model_id;
        if context.session_id != parent.session_id
            || context.agent_id != parent.agent_id
            || context.root_turn_id != parent.root_turn_id
            || !provider_matches
            || !model_matches
        {
            return Err(ShieldGateError {
                code: "chat_turn_context_invalid",
                boundary: "DirectCommandTurnGuard",
                message: "Direct chat action parent crosses an immutable turn boundary."
                    .to_string(),
            });
        }
        context.provider_id = parent.provider_id;
        context.model_id = parent.model_id;
    }
    Ok(context)
}

impl DirectCommandTurnGuard {
    pub(super) fn begin(
        persistence: &PersistenceEngine,
        request: &ExecuteCommandRequest,
    ) -> Result<Option<Self>, ShieldGateError> {
        let context = direct_command_turn_context(persistence, request)?;
        persistence
            .begin_or_validate_running_chat_turn(&context)
            .map_err(|error| ShieldGateError {
                code: "chat_turn_context_invalid",
                boundary: "DirectCommandTurnGuard",
                message: error.to_string(),
            })?;
        Ok(Some(Self {
            persistence: persistence.clone(),
            context,
            operation: request.action.kind.clone(),
            action: request.action.clone(),
            handed_off: false,
        }))
    }

    pub(super) fn validate_accepted(
        persistence: &PersistenceEngine,
        request: &ExecuteCommandRequest,
    ) -> Result<(), ShieldGateError> {
        let context = direct_command_turn_context(persistence, request)?;
        persistence
            .validate_accepted_chat_turn_generation(&context)
            .map_err(|error| ShieldGateError {
                code: "chat_turn_context_invalid",
                boundary: "DirectCommandTurnGuard",
                message: error.to_string(),
            })
    }

    pub(super) fn validate_current(&self) -> Result<(), ShieldGateError> {
        self.persistence
            .validate_chat_turn_generation(&self.context)
            .map_err(|error| ShieldGateError {
                code: "chat_turn_context_stale",
                boundary: "DirectCommandTurnGuard",
                message: error.to_string(),
            })
    }

    pub(super) async fn prepare_receipt(
        guard: Option<&Self>,
        operation: &str,
        action: &RequestedAction,
    ) -> Result<
        Option<crate::tools::native_operation_receipt::NativeOperationAttempt>,
        ShieldGateError,
    > {
        let Some(guard) = guard else {
            return Ok(None);
        };
        guard.validate_current()?;
        Ok(super::native_file_receipt::begin(
            Some(&guard.context),
            operation,
            action.path.as_deref(),
        )
        .await)
    }

    pub(super) fn finalize_output(
        &mut self,
        output: &ExecuteCommandResponse,
    ) -> Result<(), ShieldGateError> {
        self.record_verified_project_read(&self.operation, &self.action, output);
        self.validate_current()?;
        if !output.verified || output.status.as_str() != "completed" {
            self.persistence
                .finish_chat_turn(&self.context, "failed")
                .map_err(|error| ShieldGateError {
                    code: "chat_turn_context_stale",
                    boundary: "DirectCommandTurnGuard",
                    message: error.to_string(),
                })?;
        }
        self.handed_off = true;
        Ok(())
    }

    pub(super) fn record_verified_project_read(
        &self,
        operation: &str,
        action: &RequestedAction,
        output: &ExecuteCommandResponse,
    ) {
        let Some(receipt) = verified_project_read_receipt(self, operation, action, output) else {
            return;
        };
        crate::diagnostic_output::write_functional_acceptance_receipt(&receipt);
    }
}

fn verified_project_read_receipt(
    guard: &DirectCommandTurnGuard,
    operation: &str,
    action: &RequestedAction,
    output: &ExecuteCommandResponse,
) -> Option<serde_json::Value> {
    if normalize_action_kind(operation) != "terminal_execute"
        || !output.verified
        || output.status.as_str() != "completed"
    {
        return None;
    }
    let request = terminal_request(action).ok()?;
    let project_root = development_repo_root()
        .canonicalize()
        .unwrap_or_else(|_| development_repo_root());
    if !request.prompt_free_in_project(&project_root) || !is_git_worktree_status(&request) {
        return None;
    }
    let prompt = guard
        .persistence
        .open_connection()
        .ok()?
        .query_row(
            "SELECT content FROM chat_messages
             WHERE session_id=?1 AND role='user'
               AND json_extract(metadata_json, '$.turnId')=?2
             ORDER BY id ASC LIMIT 1",
            rusqlite::params![guard.context.session_id, guard.context.turn_id],
            |row| row.get::<_, String>(0),
        )
        .ok()?;
    let prompt_sha256 = crate::foundation::digest::sha256_hex(prompt.as_bytes());
    Some(serde_json::json!({
        "schemaVersion": 1,
        "kind": "verified_project_read_execution",
        "sessionId": guard.context.session_id,
        "turnId": guard.context.turn_id,
        "rootTurnId": guard.context.root_turn_id,
        "generationTokenSha256": crate::foundation::digest::sha256_hex(
            guard.context.generation_token.as_bytes()
        ),
        "processId": std::process::id(),
        "requestingProcess": crate::macos_process_identity::current(),
        "promptSha256": prompt_sha256.clone(),
        "requestSha256": prompt_sha256,
        "toolRequestSha256": crate::foundation::digest::sha256_hex(
            &serde_json::to_vec(&request).ok()?
        ),
        "toolId": "terminal_execute",
        "actionClass": "read",
        "readOnly": true,
        "mutationCount": 0,
        "status": "succeeded",
        "verifiedPostcondition": true,
        "resultEvidenceKind": "git_worktree_status",
        "projectRootSha256": crate::foundation::digest::sha256_hex(
            project_root.to_string_lossy().as_bytes()
        ),
        "resultSha256": crate::foundation::digest::sha256_hex(output.message.as_bytes()),
        "recordedAtMs": crate::foundation::clock::unix_time_ms_i64(),
    }))
}

fn is_git_worktree_status(
    request: &crate::tools::terminal_contract::NativeTerminalRequest,
) -> bool {
    let is_git = std::path::Path::new(&request.executable)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("git"));
    if !is_git || request.args.first().map(String::as_str) != Some("status") {
        return false;
    }
    request.args[1..].iter().all(|argument| {
        matches!(
            argument.as_str(),
            "--short" | "--porcelain" | "--porcelain=v1" | "--branch" | "-b"
        )
    })
}

impl Drop for DirectCommandTurnGuard {
    fn drop(&mut self) {
        if !self.handed_off {
            let _ = self.persistence.finish_chat_turn(&self.context, "failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn terminal_request(
        executable: &str,
        args: &[&str],
    ) -> crate::tools::terminal_contract::NativeTerminalRequest {
        crate::tools::terminal_contract::NativeTerminalRequest {
            executable: executable.to_string(),
            args: args
                .iter()
                .map(|argument| (*argument).to_string())
                .collect(),
            env: BTreeMap::new(),
            cwd: None,
            timeout: None,
        }
    }

    #[test]
    fn project_receipt_accepts_only_git_worktree_status() {
        assert!(is_git_worktree_status(&terminal_request(
            "git",
            &["status", "--short"]
        )));
        assert!(is_git_worktree_status(&terminal_request(
            "/usr/bin/git",
            &["status", "--porcelain=v1", "--branch"]
        )));
        assert!(!is_git_worktree_status(&terminal_request(
            "git",
            &["diff", "--stat"]
        )));
        assert!(!is_git_worktree_status(&terminal_request(
            "git",
            &["status", "--ignored"]
        )));
        assert!(!is_git_worktree_status(&terminal_request(
            "rg",
            &["status", "."]
        )));
    }
}
