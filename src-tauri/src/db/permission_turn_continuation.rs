use super::*;
use crate::macos_permission_broker::MacosPermissionState;

mod cancellation;
mod restored_checkpoint;
pub use cancellation::{CancelPermissionTurnRequest, CancelPermissionTurnResult};

const WAITING: &str = "permission_waiting";

#[derive(Clone, Debug)]
pub(crate) struct PermissionTurnContinuationCandidate {
    execution_id: String,
    plan_id: String,
    context: ChatTurnPersistenceContext,
    message_id: i64,
    message_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PermissionTurnContinuationReceipt {
    kind: &'static str,
    receipt_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_id: Option<String>,
    capability_id: String,
    native_receipt_id: String,
    prior_unmet_native_receipt_id: String,
    session_id: String,
    turn_id: String,
    root_turn_id: String,
    generation_token: String,
    generation_token_sha256: String,
    message_id: i64,
    message_sha256: String,
    from_turn_state: &'static str,
    to_turn_state: &'static str,
    permission_state: MacosPermissionState,
    reused_message: bool,
    response_claimed: bool,
    process_id: u32,
    recorded_at_ms: i64,
}

impl PersistenceEngine {
    pub(crate) fn permission_turn_continuation_candidate(
        &self,
        execution_id: &str,
    ) -> rusqlite::Result<PermissionTurnContinuationCandidate> {
        let execution_id = execution_id.trim();
        if execution_id.is_empty() {
            return Err(invalid_continuation());
        }
        let connection = self.open_connection()?;
        connection.query_row(
            "SELECT executions.execution_id,executions.plan_id,
                    turns.turn_id,turns.generation_token,turns.session_id,turns.agent_id,
                    turns.provider_id,turns.model_id,turns.parent_turn_id,turns.root_turn_id,
                    turns.turn_kind,messages.id,messages.content
             FROM agent_executions executions
             JOIN chat_turns turns
               ON turns.turn_id=executions.turn_id
              AND turns.generation_token=executions.generation_token
             JOIN chat_messages messages
               ON messages.workspace_id=turns.workspace_id
              AND messages.session_id=turns.session_id
              AND messages.agent_id=turns.agent_id
              AND messages.role='user'
              AND json_extract(messages.metadata_json,'$.turnId')=turns.turn_id
              AND json_extract(messages.metadata_json,'$.generationToken')=turns.generation_token
             WHERE executions.execution_id=?1 AND executions.status='halted'
               AND turns.status IN ('completed','escalated')
               AND turns.response_claimed_at_ms IS NULL
               AND json_extract(messages.metadata_json,
                    '$.permissionContinuation.state')='waiting'",
            params![execution_id],
            |row| {
                let message: String = row.get(12)?;
                Ok(PermissionTurnContinuationCandidate {
                    execution_id: row.get(0)?,
                    plan_id: row.get(1)?,
                    context: ChatTurnPersistenceContext {
                        turn_id: row.get(2)?,
                        generation_token: row.get(3)?,
                        session_id: row.get(4)?,
                        agent_id: row.get(5)?,
                        provider_id: row.get(6)?,
                        model_id: row.get(7)?,
                        parent_turn_id: row.get(8)?,
                        root_turn_id: row.get(9)?,
                        turn_kind: row.get(10)?,
                    },
                    message_id: row.get(11)?,
                    message_sha256: sha256_hex(message.as_bytes()),
                })
            },
        )
    }

    pub(crate) fn prepare_permission_execution_retry(
        &self,
        candidate: PermissionTurnContinuationCandidate,
        capability_id: &str,
    ) -> rusqlite::Result<()> {
        let connection = self.open_connection()?;
        let message = connection
            .query_row(
                "SELECT messages.content FROM agent_executions executions
             JOIN chat_turns turns
               ON turns.turn_id=executions.turn_id
              AND turns.generation_token=executions.generation_token
             JOIN chat_messages messages ON messages.id=?5
             WHERE executions.execution_id=?1 AND executions.plan_id=?2
               AND executions.status='running' AND turns.status='running'
               AND turns.turn_id=?3 AND turns.generation_token=?4
               AND json_extract(messages.metadata_json,
                    '$.permissionContinuation.state')='waiting'
               AND json_extract(messages.metadata_json,
                    '$.permissionContinuation.capabilityId')=?6",
                params![
                    candidate.execution_id,
                    candidate.plan_id,
                    candidate.context.turn_id,
                    candidate.context.generation_token,
                    candidate.message_id,
                    capability_id.trim(),
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if message
            .as_deref()
            .map(|message| sha256_hex(message.as_bytes()))
            .as_deref()
            != Some(candidate.message_sha256.as_str())
        {
            return Err(invalid_continuation());
        }
        Ok(())
    }

    pub(crate) fn pause_permission_turn_for_native_receipt(
        &self,
        context: &ChatTurnPersistenceContext,
        capability_id: &str,
        native_receipt_id: &str,
        permission_state: MacosPermissionState,
        native_error_code: Option<&str>,
    ) -> rusqlite::Result<bool> {
        validate_continuation_input(context, capability_id, native_receipt_id)?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let now = unix_time_ms();
        let error_code =
            stable_permission_error_code(capability_id.trim(), permission_state, native_error_code);
        let changed = transaction.execute(
            "UPDATE chat_messages AS messages
             SET metadata_json=json_set(COALESCE(metadata_json,'{}'),
                 '$.turnState',?1,
                 '$.permissionContinuation.state','waiting',
                 '$.permissionContinuation.capabilityId',?2,
                 '$.permissionContinuation.unmetNativeReceiptId',?3,
                 '$.permissionContinuation.pausedAtMs',?4,
                 '$.permissionContinuation.errorCode',?5,
                 '$.permissionContinuation.boundary','macos_permission_broker')
             WHERE messages.workspace_id=?6 AND messages.session_id=?7
               AND messages.agent_id=?8 AND messages.role='user'
               AND json_extract(messages.metadata_json,'$.turnId')=?9
               AND json_extract(messages.metadata_json,'$.generationToken')=?10
               AND json_extract(messages.metadata_json,'$.turnState') IN ('accepted','escalated')
               AND EXISTS (SELECT 1 FROM chat_turns turns
                 WHERE turns.workspace_id=messages.workspace_id
                   AND turns.turn_id=?9 AND turns.generation_token=?10
                   AND turns.session_id=?7 AND turns.agent_id=?8
                   AND turns.provider_id=?11 AND turns.model_id=?12
                   AND turns.root_turn_id=?13 AND turns.turn_kind=?14
                   AND COALESCE(turns.parent_turn_id,'')=COALESCE(?15,'')
                   AND turns.response_claimed_at_ms IS NULL)",
            params![
                WAITING,
                capability_id.trim(),
                native_receipt_id.trim(),
                now,
                error_code,
                self.workspace_id,
                context.session_id,
                context.agent_id,
                context.turn_id,
                context.generation_token,
                context.provider_id,
                context.model_id,
                context.root_turn_id,
                context.turn_kind,
                context.parent_turn_id,
            ],
        )?;
        if changed > 1 {
            return Err(invalid_continuation());
        }
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub(crate) fn prepare_permission_turn_retry(
        &self,
        context: &ChatTurnPersistenceContext,
        capability_id: &str,
    ) -> rusqlite::Result<bool> {
        validate_continuation_input(context, capability_id, "pending")?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let current_process = i64::from(std::process::id());
        let existing: Option<(Option<String>, Option<String>, Option<i64>)> = transaction
            .query_row(
                "SELECT json_extract(messages.metadata_json,
                        '$.permissionContinuation.state'),
                        json_extract(messages.metadata_json,
                        '$.permissionContinuation.capabilityId'),
                        json_extract(messages.metadata_json,
                        '$.permissionContinuation.retryProcessId')
                 FROM chat_messages messages
                 JOIN chat_turns turns
                   ON turns.workspace_id=messages.workspace_id
                  AND turns.turn_id=?1 AND turns.generation_token=?2
                  AND turns.session_id=?3 AND turns.agent_id=?4
                  AND turns.provider_id=?5 AND turns.model_id=?6
                  AND turns.root_turn_id=?7 AND turns.turn_kind=?8
                  AND COALESCE(turns.parent_turn_id,'')=COALESCE(?9,'')
                 WHERE messages.workspace_id=?10 AND messages.session_id=?3
                   AND messages.agent_id=?4 AND messages.role='user'
                   AND json_extract(messages.metadata_json,'$.turnId')=?1
                   AND json_extract(messages.metadata_json,'$.generationToken')=?2
                   AND turns.response_claimed_at_ms IS NULL",
                params![
                    context.turn_id,
                    context.generation_token,
                    context.session_id,
                    context.agent_id,
                    context.provider_id,
                    context.model_id,
                    context.root_turn_id,
                    context.turn_kind,
                    context.parent_turn_id,
                    self.workspace_id,
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((state, stored_capability, retry_process_id)) = existing else {
            transaction.commit()?;
            return Ok(false);
        };
        if state.is_none() && stored_capability.is_none() && retry_process_id.is_none() {
            transaction.commit()?;
            return Ok(false);
        }
        let (Some(state), Some(stored_capability)) = (state, stored_capability) else {
            return Err(invalid_continuation());
        };
        if stored_capability != capability_id.trim() {
            return Err(invalid_continuation());
        }
        if state != "waiting" {
            return Err(invalid_continuation());
        }
        let changed = transaction.execute(
            "UPDATE chat_messages SET metadata_json=json_set(metadata_json,
                 '$.turnState','accepted',
                 '$.permissionContinuation.state','retrying',
                 '$.permissionContinuation.retryProcessId',?1,
                 '$.permissionContinuation.retriedAtMs',?2)
             WHERE workspace_id=?3 AND session_id=?4 AND agent_id=?5 AND role='user'
               AND json_extract(metadata_json,'$.turnId')=?6
               AND json_extract(metadata_json,'$.generationToken')=?7
               AND json_extract(metadata_json,
                    '$.permissionContinuation.state')='waiting'",
            params![
                current_process,
                unix_time_ms(),
                self.workspace_id,
                context.session_id,
                context.agent_id,
                context.turn_id,
                context.generation_token,
            ],
        )?;
        if changed != 1 {
            return Err(invalid_continuation());
        }
        transaction.commit()?;
        Ok(true)
    }

    pub(crate) fn complete_permission_turn_continuation(
        &self,
        context: &ChatTurnPersistenceContext,
        capability_id: &str,
        native_receipt_id: &str,
        permission_state: MacosPermissionState,
    ) -> rusqlite::Result<Option<PermissionTurnContinuationReceipt>> {
        validate_continuation_input(context, capability_id, native_receipt_id)?;
        if !usable_permission(permission_state) {
            return Err(invalid_continuation());
        }
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let state = transaction
            .query_row(
                "SELECT messages.id,messages.content,
                        json_extract(messages.metadata_json,
                          '$.permissionContinuation.unmetNativeReceiptId'),
                        (SELECT executions.execution_id FROM agent_executions executions
                          WHERE executions.turn_id=?1
                            AND executions.generation_token=?2
                          ORDER BY executions.updated_at_ms DESC LIMIT 1)
                 FROM chat_messages messages
                 JOIN chat_turns turns
                   ON turns.workspace_id=messages.workspace_id
                  AND turns.turn_id=?1 AND turns.generation_token=?2
                  AND turns.session_id=?3 AND turns.agent_id=?4
                  AND turns.provider_id=?5 AND turns.model_id=?6
                  AND turns.root_turn_id=?7 AND turns.turn_kind=?8
                  AND COALESCE(turns.parent_turn_id,'')=COALESCE(?9,'')
                 WHERE messages.workspace_id=?10 AND messages.session_id=?3
                   AND messages.agent_id=?4 AND messages.role='user'
                   AND json_extract(messages.metadata_json,'$.turnId')=?1
                   AND json_extract(messages.metadata_json,'$.generationToken')=?2
                   AND json_extract(messages.metadata_json,
                        '$.permissionContinuation.capabilityId')=?11
                   AND json_extract(messages.metadata_json,
                        '$.permissionContinuation.state')='retrying'
                   AND json_extract(messages.metadata_json,
                        '$.permissionContinuation.retryProcessId')=?12
                   AND turns.status='running' AND turns.response_claimed_at_ms IS NULL",
                params![
                    context.turn_id,
                    context.generation_token,
                    context.session_id,
                    context.agent_id,
                    context.provider_id,
                    context.model_id,
                    context.root_turn_id,
                    context.turn_kind,
                    context.parent_turn_id,
                    self.workspace_id,
                    capability_id.trim(),
                    i64::from(std::process::id()),
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((message_id, message, prior_receipt_id, execution_id)) = state else {
            transaction.commit()?;
            return Ok(None);
        };
        let recorded_at_ms = unix_time_ms();
        let message_sha256 = sha256_hex(message.as_bytes());
        let binding = sha256_chunks(&[
            b"permission-turn-continued-v2",
            context.turn_id.as_bytes(),
            context.generation_token.as_bytes(),
            capability_id.trim().as_bytes(),
            prior_receipt_id.as_bytes(),
            native_receipt_id.trim().as_bytes(),
            message_sha256.as_bytes(),
        ])
        .to_hex();
        let receipt = PermissionTurnContinuationReceipt {
            kind: "permission_turn_continued",
            receipt_id: format!(
                "permission-turn-continued-{recorded_at_ms}-{}",
                &binding[..16]
            ),
            execution_id,
            capability_id: capability_id.trim().to_string(),
            native_receipt_id: native_receipt_id.trim().to_string(),
            prior_unmet_native_receipt_id: prior_receipt_id,
            session_id: context.session_id.clone(),
            turn_id: context.turn_id.clone(),
            root_turn_id: context.root_turn_id.clone(),
            generation_token: context.generation_token.clone(),
            generation_token_sha256: sha256_hex(context.generation_token.as_bytes()),
            message_id,
            message_sha256,
            from_turn_state: WAITING,
            to_turn_state: "accepted",
            permission_state,
            reused_message: true,
            response_claimed: false,
            process_id: std::process::id(),
            recorded_at_ms,
        };
        let encoded = serde_json::to_string(&receipt)
            .map_err(|_| rusqlite::Error::InvalidParameterName("receipt encoding failed".into()))?;
        let changed = transaction.execute(
            "UPDATE chat_messages SET metadata_json=json_set(metadata_json,
                 '$.turnState','accepted',
                 '$.permissionContinuation.state','completed',
                 '$.permissionContinuation.completedAtMs',?1,
                 '$.permissionContinuation.nativeReceiptId',?2,
                 '$.permissionContinuation.receipt',json(?3))
             WHERE id=?4 AND json_extract(metadata_json,
                 '$.permissionContinuation.state')='retrying'",
            params![
                recorded_at_ms,
                native_receipt_id.trim(),
                encoded,
                message_id
            ],
        )?;
        if changed != 1 {
            return Err(invalid_continuation());
        }
        restored_checkpoint::insert(
            &transaction,
            self,
            context,
            capability_id,
            native_receipt_id,
            recorded_at_ms,
        )?;
        transaction.commit()?;
        Ok(Some(receipt))
    }
}

fn stable_permission_error_code(
    capability_id: &str,
    state: MacosPermissionState,
    native_error_code: Option<&str>,
) -> String {
    if let Some(code) = native_error_code.map(str::trim).filter(|code| {
        !code.is_empty()
            && code.len() <= 96
            && code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            && ["permission", "authorization", "access", "timeout"]
                .iter()
                .any(|token| code.contains(token))
    }) {
        return code.to_string();
    }
    let state = match state {
        MacosPermissionState::NotRequested => "not_requested",
        MacosPermissionState::Limited => "limited",
        MacosPermissionState::Denied | MacosPermissionState::RequiresSettings => "denied",
        MacosPermissionState::Restricted => "restricted",
        MacosPermissionState::Stale => "stale",
        MacosPermissionState::Unsupported => "unsupported",
        MacosPermissionState::Allowed | MacosPermissionState::WhenUsed => "required",
    };
    format!("{capability_id}_permission_{state}")
}

fn validate_continuation_input(
    context: &ChatTurnPersistenceContext,
    capability_id: &str,
    receipt_id: &str,
) -> rusqlite::Result<()> {
    validate_chat_turn_context_fields(context)?;
    let capability_id = capability_id.trim();
    let receipt_id = receipt_id.trim();
    if capability_id.is_empty()
        || capability_id.len() > 80
        || receipt_id.is_empty()
        || receipt_id.len() > 240
    {
        return Err(invalid_continuation());
    }
    Ok(())
}

fn usable_permission(state: MacosPermissionState) -> bool {
    matches!(
        state,
        MacosPermissionState::Allowed
            | MacosPermissionState::Limited
            | MacosPermissionState::WhenUsed
    )
}

fn invalid_continuation() -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(
        "permission continuation does not match one active accepted turn".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{AcceptChatTurnRequest, CreateChatSessionRequest};

    fn fixture(name: &str) -> (std::path::PathBuf, PersistenceEngine, AcceptChatTurnRequest) {
        let root = std::env::temp_dir().join(format!(
            "oomu-permission-continuation-{name}-{}",
            unix_time_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: format!("agent-{name}"),
                provider_id: "local".to_string(),
                model_id: "model".to_string(),
                title: Some(format!("Permission {name}")),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let request = AcceptChatTurnRequest {
            turn_id: format!("turn-{name}"),
            generation_token: format!("generation-{name}"),
            parent_turn_id: None,
            root_turn_id: format!("turn-{name}"),
            turn_kind: "root".to_string(),
            session_id: session.id,
            agent_id: session.agent_id,
            provider_id: "local".to_string(),
            model_id: "model".to_string(),
            message: format!("Read my calendar for {name}."),
        };
        engine.accept_chat_turn(request.clone()).unwrap();
        (root, engine, request)
    }

    #[test]
    fn fresh_turn_is_not_mistaken_for_a_permission_retry() {
        let (root, engine, request) = fixture("fresh");
        assert!(!engine
            .prepare_permission_turn_retry(&request.persistence_context(), "calendar")
            .unwrap());
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE chat_messages SET metadata_json=json_set(metadata_json,
                 '$.permissionContinuation.state','waiting') WHERE role='user'",
                [],
            )
            .unwrap();
        assert!(engine
            .prepare_permission_turn_retry(&request.persistence_context(), "calendar")
            .is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn denied_restart_retry_and_success_reuse_one_exact_turn_once() {
        let (root, engine, request) = fixture("restart");
        let context = request.persistence_context();
        assert!(engine
            .pause_permission_turn_for_native_receipt(
                &context,
                "calendar",
                "apple-operation-denied",
                MacosPermissionState::Denied,
                Some("calendar_permission_denied"),
            )
            .unwrap());
        engine.mark_interrupted_actions().unwrap();
        let resumed = engine
            .resume_interrupted_chat_turn(request.clone())
            .unwrap();
        let original_message_id: i64 = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT id FROM chat_messages WHERE role='user'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(resumed.message_id, original_message_id);
        assert!(engine
            .prepare_permission_turn_retry(&context, "calendar")
            .unwrap());
        assert!(engine
            .prepare_permission_turn_retry(&context, "calendar")
            .is_err());
        let receipt = engine
            .complete_permission_turn_continuation(
                &context,
                "calendar",
                "apple-operation-restored",
                MacosPermissionState::Allowed,
            )
            .unwrap()
            .unwrap();
        assert_eq!(receipt.message_id, original_message_id);
        assert_eq!(receipt.native_receipt_id, "apple-operation-restored");
        assert_eq!(
            receipt.prior_unmet_native_receipt_id,
            "apple-operation-denied"
        );
        assert_eq!(receipt.from_turn_state, WAITING);
        assert_eq!(receipt.to_turn_state, "accepted");
        assert!(receipt.reused_message && !receipt.response_claimed);
        let (restored_count, restored_receipt, restored_capability): (i64, String, String) = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*),json_extract(metadata_json,'$.nativeReceiptId'),
                        json_extract(metadata_json,'$.capabilityId')
                 FROM chat_messages WHERE json_extract(metadata_json,
                      '$.permissionRestoredForTurnId')=?1",
                params![context.turn_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(restored_count, 1);
        assert_eq!(restored_receipt, "apple-operation-restored");
        assert_eq!(restored_capability, "calendar");
        assert!(engine
            .complete_permission_turn_continuation(
                &context,
                "calendar",
                "apple-operation-restored",
                MacosPermissionState::Allowed,
            )
            .unwrap()
            .is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restarted_permission_turn_cancels_once_without_removing_prior_chat() {
        let (root, engine, request) = fixture("cancel");
        let prior_message_id = engine
            .insert_chat_message(
                &request.session_id,
                &request.agent_id,
                "system",
                "Earlier conversation stays here.",
            )
            .unwrap();
        assert!(engine
            .pause_permission_turn_for_native_receipt(
                &request.persistence_context(),
                "calendar",
                "apple-operation-denied",
                MacosPermissionState::Denied,
                Some("calendar_permission_denied"),
            )
            .unwrap());
        engine.mark_interrupted_actions().unwrap();
        let cancel = CancelPermissionTurnRequest {
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            generation_token: request.generation_token.clone(),
            capability_id: "calendar".to_string(),
        };
        let first = engine.cancel_permission_turn(cancel.clone()).unwrap();
        let duplicate = engine.cancel_permission_turn(cancel).unwrap();
        assert!(first.cancelled);
        assert!(!duplicate.cancelled);
        assert_eq!(duplicate.receipt_id, first.receipt_id);
        let connection = engine.open_connection().unwrap();
        let (turn_status, terminal_count, prior_count): (String, i64, i64) = connection
            .query_row(
                "SELECT
                   (SELECT status FROM chat_turns WHERE turn_id=?1),
                   (SELECT COUNT(*) FROM chat_messages WHERE json_extract(
                     metadata_json,'$.terminalResultForTurnId')=?1),
                   (SELECT COUNT(*) FROM chat_messages WHERE id=?2
                     AND content='Earlier conversation stays here.')",
                params![request.turn_id, prior_message_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(turn_status, "cancelled");
        assert_eq!(terminal_count, 1);
        assert_eq!(prior_count, 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
