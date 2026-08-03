use super::*;
use rusqlite::{params, Transaction};

pub(super) fn insert(
    transaction: &Transaction<'_>,
    engine: &PersistenceEngine,
    context: &ChatTurnPersistenceContext,
    capability_id: &str,
    native_receipt_id: &str,
    recorded_at_ms: i64,
) -> rusqlite::Result<()> {
    let metadata = serde_json::to_string(&serde_json::json!({
        "turnId": context.turn_id,
        "generationToken": context.generation_token,
        "rootTurnId": context.root_turn_id,
        "turnKind": context.turn_kind,
        "turnState": "accepted",
        "checkpointForTurnId": context.turn_id,
        "permissionRestoredForTurnId": context.turn_id,
        "nativeReceiptId": native_receipt_id.trim(),
        "capabilityId": capability_id.trim(),
        "localizationKey": "sprint_301.permission_recovery.restored",
        "uiOnlyCheckpoint": true,
        "checkpointKind": "permission_recovery_restored",
    }))
    .map_err(|_| invalid_continuation())?;
    let inserted = transaction.execute(
        "INSERT INTO chat_messages (
           workspace_id,session_id,agent_id,role,content,provider_id,model_id,
           metadata_json,is_compacted,compaction_type,timestamp_ms,encryption_state
         ) SELECT ?1,?2,?3,'system','permission_recovery_restored',?4,?5,?6,
                  0,'raw',?7,?8
         WHERE NOT EXISTS (SELECT 1 FROM chat_messages
           WHERE workspace_id=?1 AND session_id=?2
             AND json_extract(metadata_json,'$.permissionRestoredForTurnId')=?9)",
        params![
            engine.workspace_id,
            context.session_id,
            context.agent_id,
            context.provider_id,
            context.model_id,
            metadata,
            recorded_at_ms,
            get_current_encryption_state(),
            context.turn_id,
        ],
    )?;
    (inserted == 1)
        .then_some(())
        .ok_or_else(invalid_continuation)
}
