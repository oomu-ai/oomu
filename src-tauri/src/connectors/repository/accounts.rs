use crate::{
    db::PersistenceEngine,
    foundation::{clock::unix_time_ms_i64, digest::sha256_hex},
    p0_contracts::ConnectorId,
};
use rusqlite::{params, Connection, OptionalExtension};

mod row;

pub(super) use row::from_row;

pub(in crate::connectors) fn identity_binding_hash(
    engine: &PersistenceEngine,
    connector_id: &str,
) -> Result<Option<String>, String> {
    let id = ConnectorId::parse(connector_id)?.to_string();
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT account_subject_hash FROM connector_accounts WHERE connector_id=?1 AND connection_state!='disconnected'",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "connector_account_not_found".to_string())
}

pub(in crate::connectors) fn tenant_binding_hash(
    engine: &PersistenceEngine,
    connector_id: &str,
) -> Result<Option<String>, String> {
    let id = ConnectorId::parse(connector_id)?.to_string();
    let tenant: Option<String> = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT tenant_id FROM connector_account_metadata WHERE connector_id=?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(tenant.map(|value| sha256_hex(value.as_bytes())))
}

pub(super) fn reconcile_account_shells(connection: &mut Connection) -> Result<(), String> {
    // A probed OAuth shell may no longer be `configured`. Retire anonymous
    // shells only after their real attempt expires; identity-bound accounts
    // remain visible for recovery.
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE connector_accounts
             SET connection_state='disconnected', updated_at_ms=?1
             WHERE connection_state NOT IN ('authorized','reachable','disconnected')
               AND credential_ref LIKE 'credential_%'
               AND account_subject_hash IS NULL
               AND (connection_state='configured' OR EXISTS (
                 SELECT 1 FROM connector_oauth_attempts history
                 WHERE history.connector_id=connector_accounts.connector_id
               ))
               AND NOT EXISTS (
                 SELECT 1 FROM connector_oauth_attempts attempt
                 WHERE attempt.connector_id=connector_accounts.connector_id
                   AND attempt.outcome='pending' AND attempt.expires_at_ms>=?1
               )",
            params![unix_time_ms_i64()],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE connector_accounts AS duplicate
             SET connection_state='disconnected', updated_at_ms=?1
             WHERE duplicate.connection_state NOT IN ('authorized','reachable','disconnected')
               AND duplicate.account_subject_hash IS NOT NULL
               AND EXISTS (
                 SELECT 1 FROM connector_accounts AS ready
                 WHERE ready.connector_id!=duplicate.connector_id
                   AND ready.manifest_id=duplicate.manifest_id
                   AND ready.account_subject_hash=duplicate.account_subject_hash
                   AND ready.connection_state IN ('authorized','reachable')
               )",
            params![unix_time_ms_i64()],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}
