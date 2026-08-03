use super::super::{adapter, manifest, ConnectorCapabilityGrant};
use crate::{db::PersistenceEngine, p0_contracts::ConnectorId};
use rusqlite::{params, OptionalExtension};

#[derive(Clone, Debug)]
pub(in crate::connectors) struct ConnectorAccountAuthority {
    pub manifest_id: String,
    pub account_label: String,
    pub connection_state: String,
    pub all_projects_enabled: bool,
    pub enabled_project_ids: Vec<String>,
    pub capability_grants: Vec<ConnectorCapabilityGrant>,
}

pub(in crate::connectors) fn account_authority(
    engine: &PersistenceEngine,
    connector_id: &str,
) -> Result<Option<ConnectorAccountAuthority>, String> {
    let connector_id = ConnectorId::parse(connector_id)
        .map_err(|_| "connector_planned_account_invalid".to_string())?
        .to_string();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let account = connection
        .query_row(
            "SELECT a.manifest_id,a.account_label,a.granted_scopes_json,a.connection_state,m.account_kind,a.all_projects_enabled FROM connector_accounts a LEFT JOIN connector_account_metadata m ON m.connector_id=a.connector_id WHERE a.connector_id=?1",
            params![connector_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)? != 0,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((
        manifest_id,
        account_label,
        scopes_json,
        connection_state,
        account_kind,
        all_projects_enabled,
    )) = account
    else {
        return Ok(None);
    };
    let granted_scopes: Vec<String> = serde_json::from_str(&scopes_json)
        .map_err(|_| "connector_planned_scope_state_invalid".to_string())?;
    let capability_grants = adapter::for_manifest(&manifest_id)
        .map(|registered| registered.capability_grants(&granted_scopes, account_kind.as_deref()))
        .or_else(|| {
            (manifest_id == "google_workspace")
                .then(|| manifest::google_capability_grants(&granted_scopes))
        })
        .unwrap_or_default();
    let mut statement = connection
        .prepare("SELECT project_id FROM connector_project_bindings WHERE connector_id=?1 AND enabled=1 ORDER BY project_id")
        .map_err(|error| error.to_string())?;
    let enabled_project_ids = statement
        .query_map(params![connector_id], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|error| error.to_string())?;
    Ok(Some(ConnectorAccountAuthority {
        manifest_id,
        account_label,
        connection_state,
        all_projects_enabled,
        enabled_project_ids,
        capability_grants,
    }))
}

pub(in crate::connectors) fn require_project_scope(
    engine: &PersistenceEngine,
    connector_id: &str,
    project_id: &str,
) -> Result<ConnectorAccountAuthority, String> {
    let project_id = crate::projects::repository::validate_user_project(engine, project_id)
        .map_err(|_| "connector_project_context_invalid".to_string())?;
    let account = account_authority(engine, connector_id)?
        .ok_or_else(|| "connector_account_not_found".to_string())?;
    if !matches!(
        account.connection_state.as_str(),
        "authorized" | "reachable"
    ) {
        return Err("connector_account_reconnect_required".to_string());
    }
    if !account.all_projects_enabled
        && !account
            .enabled_project_ids
            .iter()
            .any(|id| id == &project_id)
    {
        return Err("connector_project_authorization_required".to_string());
    }
    Ok(account)
}
