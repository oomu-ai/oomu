use super::super::super::{adapter, manifest, ConnectorAccount, ConnectorCapabilityGrant};
use rusqlite::{params, Connection, Row};

pub(in crate::connectors::repository) fn from_row(
    connection: &Connection,
    row: &Row<'_>,
) -> rusqlite::Result<ConnectorAccount> {
    let connector_id: String = row.get(0)?;
    let manifest_id: String = row.get(1)?;
    let granted_scopes = granted_scopes(row)?;
    let account_kind: Option<String> = row.get(14)?;
    Ok(ConnectorAccount {
        connector_id: connector_id.clone(),
        manifest_id: manifest_id.clone(),
        account_label: row.get(2)?,
        granted_scopes: granted_scopes.clone(),
        connection_state: row.get(4)?,
        schema_version: row.get::<_, i64>(5)? as u32,
        token_expires_at_ms: row.get(6)?,
        last_probe_at_ms: row.get(7)?,
        last_probe_code: row.get(8)?,
        all_projects_enabled: row.get::<_, i64>(18)? != 0,
        project_scope_reviewed_at_ms: row.get(19)?,
        enabled_project_ids: load_project_bindings(connection, &connector_id)?,
        identity_binding_hash: row.get(9)?,
        tenant_id: row.get(10)?,
        tenant_label: row.get(11)?,
        account_id: row.get(12)?,
        account_principal: row.get(13)?,
        account_kind: account_kind.clone(),
        capability_grants: capability_grants(
            &manifest_id,
            &granted_scopes,
            account_kind.as_deref(),
        ),
        data_routing: data_routing(row, &manifest_id)?,
        consent_reviewed_at_ms: row.get(16)?,
        identity_verified_at_ms: row.get(17)?,
    })
}

fn load_project_bindings(
    connection: &Connection,
    connector_id: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut bindings = connection.prepare(
        "SELECT project_id FROM connector_project_bindings WHERE connector_id=?1 AND enabled=1 ORDER BY project_id",
    )?;
    let projects = bindings
        .query_map(params![connector_id], |item| item.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(projects)
}

fn granted_scopes(row: &Row<'_>) -> rusqlite::Result<Vec<String>> {
    let scopes: String = row.get(3)?;
    Ok(serde_json::from_str(&scopes).unwrap_or_default())
}

fn capability_grants(
    manifest_id: &str,
    granted_scopes: &[String],
    account_kind: Option<&str>,
) -> Vec<ConnectorCapabilityGrant> {
    adapter::for_manifest(manifest_id)
        .map(|registered| registered.capability_grants(granted_scopes, account_kind))
        .or_else(|| {
            (manifest_id == "google_workspace")
                .then(|| manifest::google_capability_grants(granted_scopes))
        })
        .unwrap_or_default()
}

fn data_routing(row: &Row<'_>, manifest_id: &str) -> rusqlite::Result<Vec<String>> {
    let metadata_routing: Option<String> = row.get(15)?;
    Ok(metadata_routing
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_else(|| {
            manifest::manifest(manifest_id)
                .map(|item| item.data_destinations)
                .unwrap_or_default()
        }))
}
