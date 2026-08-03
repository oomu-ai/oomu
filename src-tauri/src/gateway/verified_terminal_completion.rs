use super::AutoTurnRegistration;
use crate::db::{ChatTurnPersistenceContext, CompleteClaimedChatTurnRequest, PersistenceEngine};
use crate::inference::ChatTurnResponse;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const PROJECT_STATUS_COMMAND: &str = "/usr/bin/git status --short --branch";

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedProjectStatus {
    cwd: PathBuf,
    project: String,
    branch: String,
    changed_path_count: usize,
}

pub(super) fn persist_verified_project_status_completion(
    persistence: &PersistenceEngine,
    registration: &AutoTurnRegistration,
    completed_data: &str,
) -> Result<Option<ChatTurnResponse>, String> {
    let Some(receipt) = verified_project_status(completed_data)? else {
        return Ok(None);
    };
    persist_project_status(persistence, registration, receipt).map(Some)
}

fn verified_project_status(completed_data: &str) -> Result<Option<VerifiedProjectStatus>, String> {
    let Ok(completion) = serde_json::from_str::<Value>(completed_data) else {
        return Ok(None);
    };
    let Some(outputs) = completion.get("outputs").and_then(Value::as_array) else {
        return Ok(None);
    };
    if outputs.len() != 1
        || outputs[0].get("operation").and_then(Value::as_str) != Some("terminal_execute")
    {
        return Ok(None);
    }
    let output = &outputs[0];
    if completion.get("status").and_then(Value::as_str) != Some("completed")
        || completion.get("verified").and_then(Value::as_bool) != Some(true)
        || output.get("status").and_then(Value::as_str) != Some("completed")
        || output.get("verified").and_then(Value::as_bool) != Some(true)
    {
        return Err("OOMU could not verify the completed project check.".to_string());
    }
    let terminal_claims = output
        .get("claims")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(native_terminal_claim_body)
        .collect::<Vec<_>>();
    if terminal_claims.len() != 1 {
        return Err("OOMU could not verify the completed project check.".to_string());
    }
    let native = crate::verifier::native_terminal_claim::parse_and_verify(terminal_claims[0])
        .map_err(|_| "OOMU could not verify the completed project check.".to_string())?;
    if native.command != PROJECT_STATUS_COMMAND {
        return Ok(None);
    }
    let stdout = output
        .get("message")
        .and_then(Value::as_str)
        .and_then(|message| message.split_once("\nstdout:\n").map(|(_, value)| value))
        .map(|value| {
            value
                .split_once("\nstderr:\n")
                .map_or(value, |(stdout, _)| stdout)
        })
        .ok_or_else(|| "OOMU could not verify the completed project check.".to_string())?;
    let mut lines = stdout.lines().filter(|line| !line.trim().is_empty());
    let branch = lines
        .next()
        .and_then(parse_branch_line)
        .ok_or_else(|| "OOMU could not verify the completed project check.".to_string())?;
    let changed_path_count = lines.count();
    let project = native
        .cwd
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("project")
        .to_string();
    Ok(Some(VerifiedProjectStatus {
        cwd: native.cwd,
        project,
        branch,
        changed_path_count,
    }))
}

fn native_terminal_claim_body(claim: &str) -> Option<&str> {
    let body = claim.strip_prefix("CLAIM ").unwrap_or(claim);
    body.starts_with("native_terminal_receipt ").then_some(body)
}

fn parse_branch_line(line: &str) -> Option<String> {
    let branch = line.strip_prefix("## ")?.split("...").next()?.trim();
    (!branch.is_empty()).then(|| branch.to_string())
}

fn persist_project_status(
    persistence: &PersistenceEngine,
    registration: &AutoTurnRegistration,
    receipt: VerifiedProjectStatus,
) -> Result<ChatTurnResponse, String> {
    let parent = persistence
        .select_chat_turn_context(&registration.parent_turn_id)
        .map_err(|_| "The completed project check could not be added to this chat.".to_string())?
        .ok_or_else(|| "The project check's original chat turn was not found.".to_string())?;
    if parent.session_id != registration.callback.session_id
        || parent.agent_id != registration.agent_id
        || parent.root_turn_id != registration.root_turn_id
    {
        return Err(
            "The completed project check no longer matches its original chat turn.".to_string(),
        );
    }
    let session = persistence
        .select_chat_session_by_id(&registration.callback.session_id)
        .map_err(|_| "The project check's chat is no longer available.".to_string())?;
    let context = ChatTurnPersistenceContext {
        turn_id: completion_identity("turn"),
        generation_token: completion_identity("generation"),
        session_id: parent.session_id.clone(),
        agent_id: parent.agent_id.clone(),
        provider_id: parent.provider_id.clone(),
        model_id: parent.model_id.clone(),
        parent_turn_id: Some(parent.turn_id.clone()),
        root_turn_id: parent.root_turn_id.clone(),
        turn_kind: crate::db::AUTO_TURN_KIND.to_string(),
    };
    persistence
        .begin_or_claim_chat_turn_response(&context)
        .map_err(|_| "The completed project check could not be added to this chat.".to_string())?;
    let text = localized_project_status(persistence, &registration.locale, &receipt);
    let metadata = json!({
        "eventKind": "verified_native_project_status_completion",
        "responseSource": "verified_native_receipt",
        "verifiedNativeExecutionReceipt": true,
        "turnId": context.turn_id,
        "generationToken": context.generation_token,
        "sessionId": context.session_id,
        "agentId": context.agent_id,
        "rootTurnId": context.root_turn_id,
        "parentTurnId": context.parent_turn_id,
        "turnKind": context.turn_kind,
        "project": receipt.project,
        "branch": receipt.branch,
        "changedPathCount": receipt.changed_path_count,
        "cwdSha256": crate::foundation::digest::sha256_hex(receipt.cwd.to_string_lossy().as_bytes()),
    });
    if let Err(error) = persistence.complete_claimed_chat_turn(CompleteClaimedChatTurnRequest {
        context: context.clone(),
        role: "assistant".to_string(),
        content: text.clone(),
        message_provider_id: context.provider_id.clone(),
        message_model_id: context.model_id.clone(),
        metadata: metadata.clone(),
        session_title: None,
        session_provider_id: session.provider_id,
        session_model_id: session.model_id,
        status: "completed".to_string(),
    }) {
        let _ = persistence.finish_chat_turn(&context, "failed");
        return Err(format!(
            "The completed project check could not be added to this chat: {error}"
        ));
    }
    crate::diagnostic_output::write_functional_acceptance_receipt(&json!({
        "kind": "verified_native_project_status",
        "sessionId": context.session_id,
        "turnId": context.turn_id,
        "parentTurnId": context.parent_turn_id,
        "project": receipt.project,
        "branch": receipt.branch,
        "changedPathCount": receipt.changed_path_count,
        "assistantCompletion": text,
    }));
    Ok(ChatTurnResponse {
        text,
        session_id: context.session_id,
        turn_id: context.turn_id,
        generation_token: context.generation_token,
        metadata: Some(metadata),
        route_escalation: None,
    })
}

fn localized_project_status(
    persistence: &PersistenceEngine,
    locale: &str,
    receipt: &VerifiedProjectStatus,
) -> String {
    let clean = receipt.changed_path_count == 0;
    let key = if clean {
        "/gateway/auto_turn/project_status_clean"
    } else {
        "/gateway/auto_turn/project_status_dirty"
    };
    let fallback = if clean {
        "The {project} working tree is clean{branch}."
    } else {
        "The {project} working tree has changes{branch}."
    };
    let template =
        crate::settings::locale_state_for_engine(persistence, Some(locale.trim().to_string()))
            .ok()
            .and_then(|state| {
                state
                    .translations
                    .pointer(key)
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| fallback.to_string());
    template
        .replace("{project}", &receipt.project)
        .replace("{branch}", &receipt.branch.replace('`', ""))
}

fn completion_identity(prefix: &str) -> String {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    format!(
        "{prefix}-{:x}-{:x}",
        crate::foundation::clock::unix_time_ns_u128(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    fn completion(stdout: &str) -> String {
        let cwd = std::env::temp_dir().canonicalize().unwrap();
        let claim = format!(
            "CLAIM native_terminal_receipt schema=oomu.native_terminal.v1 evidence_kind=observed_native request_sha256={} command_b64={} cwd_b64={} env_keys_b64= exit_status=0 timed_out=false postcondition_verified=true direct_process=true",
            "a".repeat(64),
            URL_SAFE_NO_PAD.encode(PROJECT_STATUS_COMMAND),
            URL_SAFE_NO_PAD.encode(cwd.to_string_lossy().as_bytes()),
        );
        json!({
            "status": "completed",
            "verified": true,
            "outputs": [{
                "operation": "terminal_execute",
                "status": "completed",
                "message": format!("The terminal command finished successfully. Exit status: 0.\nstdout:\n{stdout}"),
                "claims": [claim],
                "verified": true,
            }],
        })
        .to_string()
    }

    #[test]
    fn parses_verified_dirty_and_clean_project_status() {
        let dirty = verified_project_status(&completion(
            "## main...origin/main\n M src/lib.rs\n?? note.md",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(dirty.branch, "main");
        assert_eq!(dirty.changed_path_count, 2);

        let clean = verified_project_status(&completion("## main...origin/main"))
            .unwrap()
            .unwrap();
        assert_eq!(clean.changed_path_count, 0);
    }

    #[test]
    fn ignores_other_terminal_commands_and_rejects_unverified_status() {
        let other = completion("## main").replace(
            &URL_SAFE_NO_PAD.encode(PROJECT_STATUS_COMMAND),
            &URL_SAFE_NO_PAD.encode("/usr/bin/git log -1"),
        );
        assert!(verified_project_status(&other).unwrap().is_none());
        let unverified = completion("## main").replace("\"verified\":true", "\"verified\":false");
        assert!(verified_project_status(&unverified).is_err());
    }
}
