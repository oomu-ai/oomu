use super::super::{ConnectorProjectScope, SetConnectorProjectScopeRequest};
use crate::{
    db::PersistenceEngine, foundation::clock::unix_time_ms_i64, p0_contracts::ConnectorId,
};
use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;

#[cfg(test)]
pub(in crate::connectors) fn set_project_binding(
    engine: &PersistenceEngine,
    connector_id: &str,
    project_id: &str,
    enabled: bool,
) -> Result<(), String> {
    let project = crate::projects::repository::user_managed_project_id(project_id)?;
    let connector = ConnectorId::parse(connector_id)?.to_string();
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let account_state: String = connection
        .query_row(
            "SELECT connection_state FROM connector_accounts WHERE connector_id=?1",
            params![connector],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Connector account was not found.".to_string())?;
    if enabled && !matches!(account_state.as_str(), "authorized" | "reachable") {
        return Err("Only an authorized connector can be enabled for a project.".to_string());
    }
    let now = unix_time_ms_i64();
    connection.execute("INSERT INTO connector_project_bindings (connector_id,project_id,enabled,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,?4,?4) ON CONFLICT(connector_id,project_id) DO UPDATE SET enabled=excluded.enabled,updated_at_ms=excluded.updated_at_ms", params![connector, project, enabled, now]).map_err(|e| e.to_string())?;
    Ok(())
}

pub(in crate::connectors) fn set_project_scope(
    engine: &PersistenceEngine,
    request: SetConnectorProjectScopeRequest,
) -> Result<ConnectorProjectScope, String> {
    engine.require_durable_store("save connector project access")?;
    let connector_id = ConnectorId::parse(&request.connector_id)?.to_string();
    let mut unique = HashSet::with_capacity(request.enabled_project_ids.len());
    let mut enabled_project_ids = Vec::with_capacity(request.enabled_project_ids.len());
    for raw_project_id in request.enabled_project_ids {
        let project_id = crate::projects::repository::user_managed_project_id(&raw_project_id)?;
        if !unique.insert(project_id.clone()) {
            return Err("connector_project_scope_duplicate_project".to_string());
        }
        enabled_project_ids.push(project_id);
    }
    enabled_project_ids.sort();
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let connection_state: String = transaction.query_row(
        "SELECT connection_state FROM connector_accounts WHERE connector_id=?1 AND connection_state!='disconnected'",
        params![connector_id],
        |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?
        .ok_or_else(|| "connector_account_not_found".to_string())?;
    if (request.all_projects_enabled || !enabled_project_ids.is_empty())
        && !matches!(connection_state.as_str(), "authorized" | "reachable")
    {
        return Err("connector_project_scope_reconnect_required".to_string());
    }
    for project_id in &enabled_project_ids {
        let exists = transaction
            .query_row(
                "SELECT 1 FROM projects WHERE project_id=?1 AND archived_at_ms IS NULL LIMIT 1",
                params![project_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .is_some();
        if !exists {
            return Err("connector_project_scope_project_not_found".to_string());
        }
    }
    let now = unix_time_ms_i64();
    transaction.execute(
        "UPDATE connector_accounts SET all_projects_enabled=?2,project_scope_reviewed_at_ms=?3,updated_at_ms=?3 WHERE connector_id=?1",
        params![connector_id, request.all_projects_enabled, now],
    ).map_err(|error| error.to_string())?;
    transaction.execute(
        "UPDATE connector_project_bindings SET enabled=0,updated_at_ms=?2 WHERE connector_id=?1",
        params![connector_id, now],
    ).map_err(|error| error.to_string())?;
    for project_id in &enabled_project_ids {
        transaction.execute(
            "INSERT INTO connector_project_bindings (connector_id,project_id,enabled,created_at_ms,updated_at_ms) VALUES (?1,?2,1,?3,?3) ON CONFLICT(connector_id,project_id) DO UPDATE SET enabled=1,updated_at_ms=excluded.updated_at_ms",
            params![connector_id, project_id, now],
        ).map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(ConnectorProjectScope {
        connector_id,
        all_projects_enabled: request.all_projects_enabled,
        enabled_project_ids,
        project_scope_reviewed_at_ms: now,
        updated_at_ms: now,
    })
}
