use super::*;
use rusqlite::{params, OptionalExtension, Transaction};

#[derive(Clone, Debug, Deserialize)]
pub struct CancelPermissionTurnRequest {
    pub session_id: String,
    pub turn_id: String,
    pub generation_token: String,
    pub capability_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelPermissionTurnResult {
    pub cancelled: bool,
    pub receipt_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionTurnCancellationReceipt {
    kind: &'static str,
    receipt_id: String,
    session_id: String,
    turn_id: String,
    generation_token_sha256: String,
    capability_id: String,
    message_id: i64,
    message_sha256: String,
    reused_message: bool,
    response_claimed: bool,
    process_id: u32,
    recorded_at_ms: i64,
}

struct ActiveCancellation {
    message_id: i64,
    message: String,
    existing_receipt: Option<String>,
    agent_id: String,
    provider_id: String,
    model_id: String,
    root_turn_id: String,
    parent_turn_id: Option<String>,
    turn_kind: String,
    turn_status: String,
    response_claimed_at_ms: Option<i64>,
}

impl PersistenceEngine {
    pub fn cancel_permission_turn(
        &self,
        request: CancelPermissionTurnRequest,
    ) -> rusqlite::Result<CancelPermissionTurnResult> {
        validate_request(&request)?;
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let active = load_active(&transaction, self, &request)?.ok_or_else(invalid_continuation)?;
        if let Some(result) = existing_result(active.existing_receipt.as_deref())? {
            transaction.commit()?;
            return Ok(result);
        }
        validate_active(&active)?;
        let receipt = cancellation_receipt(&request, &active);
        cancel_exact_turn(&transaction, self, &request, &active, &receipt)?;
        insert_terminal_message(
            &transaction,
            self,
            &request,
            &active,
            receipt.recorded_at_ms,
        )?;
        transaction.commit()?;
        emit_acceptance_receipt(&receipt);
        Ok(CancelPermissionTurnResult {
            cancelled: true,
            receipt_id: receipt.receipt_id,
        })
    }
}

fn load_active(
    transaction: &Transaction<'_>,
    engine: &PersistenceEngine,
    request: &CancelPermissionTurnRequest,
) -> rusqlite::Result<Option<ActiveCancellation>> {
    transaction
        .query_row(
            "SELECT messages.id,messages.content,
                    json_extract(messages.metadata_json,'$.permissionContinuation.cancelReceipt'),
                    turns.agent_id,turns.provider_id,turns.model_id,turns.root_turn_id,
                    turns.parent_turn_id,turns.turn_kind,turns.status,
                    turns.response_claimed_at_ms
             FROM chat_turns turns JOIN chat_messages messages
               ON messages.workspace_id=turns.workspace_id
              AND messages.session_id=turns.session_id AND messages.agent_id=turns.agent_id
              AND messages.role='user'
              AND json_extract(messages.metadata_json,'$.turnId')=turns.turn_id
              AND json_extract(messages.metadata_json,'$.generationToken')=turns.generation_token
             WHERE turns.workspace_id=?1 AND turns.session_id=?2 AND turns.turn_id=?3
               AND turns.generation_token=?4
               AND json_extract(messages.metadata_json,
                    '$.permissionContinuation.capabilityId')=?5",
            params![
                engine.workspace_id,
                request.session_id.trim(),
                request.turn_id.trim(),
                request.generation_token.trim(),
                request.capability_id.trim(),
            ],
            |row| {
                Ok(ActiveCancellation {
                    message_id: row.get(0)?,
                    message: row.get(1)?,
                    existing_receipt: row.get(2)?,
                    agent_id: row.get(3)?,
                    provider_id: row.get(4)?,
                    model_id: row.get(5)?,
                    root_turn_id: row.get(6)?,
                    parent_turn_id: row.get(7)?,
                    turn_kind: row.get(8)?,
                    turn_status: row.get(9)?,
                    response_claimed_at_ms: row.get(10)?,
                })
            },
        )
        .optional()
}

fn existing_result(encoded: Option<&str>) -> rusqlite::Result<Option<CancelPermissionTurnResult>> {
    let Some(encoded) = encoded else {
        return Ok(None);
    };
    let receipt_id = serde_json::from_str::<Value>(encoded)
        .ok()
        .and_then(|receipt| receipt.get("receiptId")?.as_str().map(str::to_string))
        .filter(|receipt_id| !receipt_id.trim().is_empty())
        .ok_or_else(invalid_continuation)?;
    Ok(Some(CancelPermissionTurnResult {
        cancelled: false,
        receipt_id,
    }))
}

fn validate_active(active: &ActiveCancellation) -> rusqlite::Result<()> {
    (active.response_claimed_at_ms.is_none()
        && matches!(
            active.turn_status.as_str(),
            "running" | "failed" | "escalated"
        ))
    .then_some(())
    .ok_or_else(invalid_continuation)
}

fn cancellation_receipt(
    request: &CancelPermissionTurnRequest,
    active: &ActiveCancellation,
) -> PermissionTurnCancellationReceipt {
    let recorded_at_ms = unix_time_ms();
    let message_sha256 = sha256_hex(active.message.as_bytes());
    let binding = sha256_chunks(&[
        b"permission-turn-cancelled-v1",
        request.session_id.trim().as_bytes(),
        request.turn_id.trim().as_bytes(),
        request.generation_token.trim().as_bytes(),
        request.capability_id.trim().as_bytes(),
        message_sha256.as_bytes(),
    ])
    .to_hex();
    PermissionTurnCancellationReceipt {
        kind: "permission_turn_cancelled",
        receipt_id: format!(
            "permission-turn-cancelled-{recorded_at_ms}-{}",
            &binding[..16]
        ),
        session_id: request.session_id.trim().to_string(),
        turn_id: request.turn_id.trim().to_string(),
        generation_token_sha256: sha256_hex(request.generation_token.trim().as_bytes()),
        capability_id: request.capability_id.trim().to_string(),
        message_id: active.message_id,
        message_sha256,
        reused_message: true,
        response_claimed: false,
        process_id: std::process::id(),
        recorded_at_ms,
    }
}

fn cancel_exact_turn(
    transaction: &Transaction<'_>,
    engine: &PersistenceEngine,
    request: &CancelPermissionTurnRequest,
    active: &ActiveCancellation,
    receipt: &PermissionTurnCancellationReceipt,
) -> rusqlite::Result<()> {
    let turn_changed = transaction.execute(
        "UPDATE chat_turns SET status='cancelled',completed_at_ms=?1
         WHERE workspace_id=?2 AND session_id=?3 AND turn_id=?4 AND generation_token=?5
           AND agent_id=?6 AND provider_id=?7 AND model_id=?8 AND root_turn_id=?9
           AND turn_kind=?10 AND COALESCE(parent_turn_id,'')=COALESCE(?11,'')
           AND status=?12 AND response_claimed_at_ms IS NULL",
        params![
            receipt.recorded_at_ms,
            engine.workspace_id,
            request.session_id.trim(),
            request.turn_id.trim(),
            request.generation_token.trim(),
            &active.agent_id,
            &active.provider_id,
            &active.model_id,
            &active.root_turn_id,
            &active.turn_kind,
            &active.parent_turn_id,
            &active.turn_status,
        ],
    )?;
    let encoded = serde_json::to_string(receipt).map_err(|_| invalid_continuation())?;
    let message_changed = transaction.execute(
        "UPDATE chat_messages SET metadata_json=json_set(metadata_json,
             '$.turnState','cancelled','$.permissionContinuation.state','cancelled',
             '$.permissionContinuation.cancelReceipt',json(?1))
         WHERE id=?2 AND json_extract(metadata_json,
             '$.permissionContinuation.state') IN ('waiting','retrying')",
        params![encoded, active.message_id],
    )?;
    (turn_changed == 1 && message_changed == 1)
        .then_some(())
        .ok_or_else(invalid_continuation)
}

fn insert_terminal_message(
    transaction: &Transaction<'_>,
    engine: &PersistenceEngine,
    request: &CancelPermissionTurnRequest,
    active: &ActiveCancellation,
    recorded_at_ms: i64,
) -> rusqlite::Result<()> {
    let metadata = serde_json::to_string(&serde_json::json!({
        "turnId": request.turn_id.trim(),
        "generationToken": request.generation_token.trim(),
        "rootTurnId": active.root_turn_id,
        "turnKind": active.turn_kind,
        "turnState": "cancelled",
        "terminalResultForTurnId": request.turn_id.trim(),
        "localizationKey": "sprint_301.permission_recovery.cancelled",
        "uiOnlyCheckpoint": true,
        "checkpointKind": "permission_recovery_cancelled",
    }))
    .map_err(|_| invalid_continuation())?;
    let inserted = transaction.execute(
        "INSERT INTO chat_messages (
           workspace_id,session_id,agent_id,role,content,provider_id,model_id,
           metadata_json,is_compacted,compaction_type,timestamp_ms,encryption_state
         ) SELECT ?1,?2,?3,'system','permission_recovery_cancelled',?4,?5,?6,
                  0,'raw',?7,?8
         WHERE NOT EXISTS (SELECT 1 FROM chat_messages
           WHERE workspace_id=?1 AND session_id=?2
             AND json_extract(metadata_json,'$.terminalResultForTurnId')=?9)",
        params![
            engine.workspace_id,
            request.session_id.trim(),
            &active.agent_id,
            &active.provider_id,
            &active.model_id,
            metadata,
            recorded_at_ms,
            get_current_encryption_state(),
            request.turn_id.trim(),
        ],
    )?;
    (inserted == 1)
        .then_some(())
        .ok_or_else(invalid_continuation)
}

fn validate_request(request: &CancelPermissionTurnRequest) -> rusqlite::Result<()> {
    [
        request.session_id.as_str(),
        request.turn_id.as_str(),
        request.generation_token.as_str(),
        request.capability_id.as_str(),
    ]
    .iter()
    .all(|value| !value.trim().is_empty() && value.len() <= 512)
    .then_some(())
    .ok_or_else(invalid_continuation)
}

fn emit_acceptance_receipt(receipt: &PermissionTurnCancellationReceipt) {
    if crate::diagnostic_output::native_acceptance_enabled() {
        crate::diagnostic_output::write_functional_acceptance_receipt(
            &serde_json::to_value(receipt).unwrap_or(Value::Null),
        );
    }
}
