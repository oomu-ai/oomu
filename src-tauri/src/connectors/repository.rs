mod accounts;
mod authority;
#[cfg(test)]
mod oauth_tests;
mod scope;
#[cfg(test)]
mod scope_tests;

pub(super) use accounts::{identity_binding_hash, tenant_binding_hash};
pub(super) use authority::account_authority;
pub(super) use authority::require_project_scope;
#[cfg(test)]
pub(super) use scope::set_project_binding;
pub(super) use scope::set_project_scope;

use super::{ConnectorAccount, ConnectorConnectionStatus, ConnectorIdentityMetadata, SetupState};
use crate::{
    db::PersistenceEngine,
    foundation::{clock::unix_time_ms_i64, digest::sha256_hex},
    p0_contracts::{ConnectorId, ProjectId, TaskId, TaskRunId},
};
use rusqlite::{params, OptionalExtension};

pub(super) fn create_account(
    engine: &PersistenceEngine,
    manifest_id: &str,
    version: u32,
) -> Result<String, String> {
    engine.require_durable_store("connect an external service")?;
    let connector_id = ConnectorId::new().to_string();
    let credential_ref = format!("credential_{}", sha256_hex(connector_id.as_bytes()));
    let now = unix_time_ms_i64();
    engine.open_connection().map_err(|e| e.to_string())?.execute(
        "INSERT INTO connector_accounts (connector_id,manifest_id,credential_ref,connection_state,schema_version,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,'configured',?4,?5,?5)",
        params![connector_id, manifest_id, credential_ref, version, now],
    ).map_err(|e| e.to_string())?;
    Ok(connector_id)
}

pub(super) fn validate_oauth_account(
    engine: &PersistenceEngine,
    connector_id: &str,
    manifest_id: &str,
) -> Result<String, String> {
    let id = ConnectorId::parse(connector_id)?.to_string();
    let existing: Option<String> = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT manifest_id FROM connector_accounts WHERE connector_id=?1 AND connection_state!='disconnected'",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    match existing.as_deref() {
        Some(value) if value == manifest_id => Ok(id),
        Some(_) => Err("connector_manifest_identity_mismatch".to_string()),
        None => Err("connector_account_not_found".to_string()),
    }
}

pub(super) fn account_granted_scopes(
    engine: &PersistenceEngine,
    connector_id: &str,
) -> Result<Vec<String>, String> {
    let id = ConnectorId::parse(connector_id)?.to_string();
    let raw: String = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT granted_scopes_json FROM connector_accounts WHERE connector_id=?1 AND connection_state!='disconnected'",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "connector_account_not_found".to_string())?;
    serde_json::from_str(&raw).map_err(|_| "connector_scope_state_invalid".to_string())
}

pub(super) fn record_oauth_attempt(
    engine: &PersistenceEngine,
    attempt_id: &str,
    connector_id: &str,
    state_hash: &str,
    redirect_uri: &str,
    expires_at_ms: i64,
) -> Result<(), String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let now = unix_time_ms_i64();
    transaction.execute(
        "INSERT INTO connector_oauth_attempts (attempt_id,connector_id,state_hash,redirect_uri,expires_at_ms,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6)",
        params![attempt_id, connector_id, state_hash, redirect_uri, expires_at_ms, now],
    ).map_err(|error| error.to_string())?;
    let changed = transaction
        .execute(
            "UPDATE connector_accounts
         SET connection_state=CASE
               WHEN connection_state='disconnected' AND account_subject_hash IS NULL
                 THEN 'configured'
               ELSE connection_state
             END,
             last_probe_code='oauth_started',
             last_probe_at_ms=?2,
             updated_at_ms=?2
         WHERE connector_id=?1",
            params![connector_id, now],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("connector_account_not_found".to_string());
    }
    transaction.commit().map_err(|error| error.to_string())
}

pub(super) fn connection_status(
    engine: &PersistenceEngine,
    connector_id: &str,
) -> Result<ConnectorConnectionStatus, String> {
    let id = ConnectorId::parse(connector_id)?.to_string();
    let row: Option<(String, String, Option<i64>, Option<String>)> = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT connection_state,granted_scopes_json,last_probe_at_ms,last_probe_code FROM connector_accounts WHERE connector_id=?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let (connection_state, scopes_json, last_probe_at_ms, last_probe_code) =
        row.ok_or_else(|| "connector_account_not_found".to_string())?;
    let granted_scopes = serde_json::from_str(&scopes_json)
        .map_err(|_| "connector_scope_state_invalid".to_string())?;
    Ok(ConnectorConnectionStatus {
        connector_id: id,
        connection_state,
        granted_scopes,
        last_probe_at_ms,
        last_probe_code,
    })
}

pub(super) fn finish_oauth(
    engine: &PersistenceEngine,
    attempt_id: &str,
    connector_id: &str,
    account_label: &str,
    subject: &str,
    scopes: &[String],
    expires_at_ms: Option<i64>,
    refresh_expires_at_ms: Option<i64>,
    metadata: Option<&ConnectorIdentityMetadata>,
) -> Result<(), String> {
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let now = unix_time_ms_i64();
    let scope_json = serde_json::to_string(scopes).map_err(|e| e.to_string())?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let changed = transaction.execute("UPDATE connector_oauth_attempts SET outcome='completed',completed_at_ms=?2 WHERE attempt_id=?1 AND outcome='pending' AND expires_at_ms>=?2", params![attempt_id, now]).map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("OAuth attempt expired or was already consumed.".to_string());
    }
    let binding_hash = sha256_hex(subject.as_bytes());
    let existing_ready: Option<String> = transaction.query_row(
        "SELECT connector_id FROM connector_accounts WHERE manifest_id=(SELECT manifest_id FROM connector_accounts WHERE connector_id=?1) AND account_subject_hash=?2 AND connector_id!=?1 AND connection_state IN ('authorized','reachable') LIMIT 1",
        params![connector_id, binding_hash],
        |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    if existing_ready.is_some() {
        return Err("connector_account_already_connected".to_string());
    }
    if let Some(metadata) = metadata {
        if metadata.identity_binding_hash != binding_hash {
            return Err("connector_identity_binding_invalid".to_string());
        }
        let routing =
            serde_json::to_string(&metadata.data_routing).map_err(|error| error.to_string())?;
        transaction.execute(
            "INSERT INTO connector_account_metadata (connector_id,tenant_id,tenant_label,account_id,account_principal,account_kind,identity_binding_hash,data_routing_json,consent_reviewed_at_ms,identity_verified_at_ms,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11) ON CONFLICT(connector_id) DO UPDATE SET tenant_id=excluded.tenant_id,tenant_label=excluded.tenant_label,account_id=excluded.account_id,account_principal=excluded.account_principal,account_kind=excluded.account_kind,identity_binding_hash=excluded.identity_binding_hash,data_routing_json=excluded.data_routing_json,consent_reviewed_at_ms=excluded.consent_reviewed_at_ms,identity_verified_at_ms=excluded.identity_verified_at_ms,updated_at_ms=excluded.updated_at_ms",
            params![connector_id, metadata.tenant_id, metadata.tenant_label, metadata.account_id, metadata.account_principal, metadata.account_kind, binding_hash, routing, metadata.consent_reviewed_at_ms, metadata.identity_verified_at_ms, now],
        ).map_err(|error| error.to_string())?;
    }
    transaction.execute("UPDATE connector_accounts SET account_label=?2,account_subject_hash=?3,granted_scopes_json=?4,token_expires_at_ms=?5,refresh_expires_at_ms=?6,connection_state='authorized',last_probe_code='oauth_completed',last_probe_at_ms=?7,updated_at_ms=?7 WHERE connector_id=?1", params![connector_id, account_label, binding_hash, scope_json, expires_at_ms, refresh_expires_at_ms, now]).map_err(|e| e.to_string())?;
    transaction.execute(
        "UPDATE connector_accounts SET connection_state='disconnected',updated_at_ms=?3 WHERE connector_id!=?1 AND manifest_id=(SELECT manifest_id FROM connector_accounts WHERE connector_id=?1) AND account_subject_hash=?2 AND connection_state NOT IN ('authorized','reachable','disconnected')",
        params![connector_id, binding_hash, now],
    ).map_err(|error| error.to_string())?;
    transaction.commit().map_err(|e| e.to_string())
}

pub(super) fn fail_oauth(
    engine: &PersistenceEngine,
    attempt_id: &str,
    code: &str,
    preserve_connection: bool,
) -> Result<(), String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let now = unix_time_ms_i64();
    let attempt_changed = transaction
        .execute(
            "UPDATE connector_oauth_attempts SET outcome='failed',completed_at_ms=?2 WHERE attempt_id=?1 AND outcome='pending'",
            params![attempt_id, now],
        )
        .map_err(|error| error.to_string())?;
    if attempt_changed != 1 {
        return Err("OAuth failure could not claim the pending attempt.".to_string());
    }
    let account_changed = transaction
        .execute(
            "UPDATE connector_accounts SET connection_state=CASE WHEN ?4 THEN connection_state ELSE 'disconnected' END,last_probe_code=?2,last_probe_at_ms=?3,updated_at_ms=?3 WHERE connector_id=(SELECT connector_id FROM connector_oauth_attempts WHERE attempt_id=?1)",
            params![attempt_id, code, now, preserve_connection],
        )
        .map_err(|error| error.to_string())?;
    if account_changed != 1 {
        return Err("OAuth failure could not update its connector account.".to_string());
    }
    transaction.commit().map_err(|error| error.to_string())
}

pub(super) fn list_accounts(engine: &PersistenceEngine) -> Result<Vec<ConnectorAccount>, String> {
    let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
    accounts::reconcile_account_shells(&mut connection)?;
    let mut statement = connection.prepare(
        "SELECT a.connector_id,a.manifest_id,a.account_label,a.granted_scopes_json,a.connection_state,a.schema_version,a.token_expires_at_ms,a.last_probe_at_ms,a.last_probe_code,a.account_subject_hash,m.tenant_id,m.tenant_label,m.account_id,m.account_principal,m.account_kind,m.data_routing_json,m.consent_reviewed_at_ms,m.identity_verified_at_ms,a.all_projects_enabled,a.project_scope_reviewed_at_ms FROM connector_accounts a LEFT JOIN connector_account_metadata m ON m.connector_id=a.connector_id WHERE a.connection_state!='disconnected' ORDER BY a.updated_at_ms DESC"
    ).map_err(|e| e.to_string())?;
    let accounts = statement
        .query_map([], |row| accounts::from_row(&connection, row))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(accounts)
}

pub(super) fn account_manifest(
    engine: &PersistenceEngine,
    connector_id: &str,
) -> Result<String, String> {
    let id = ConnectorId::parse(connector_id)?.to_string();
    engine.open_connection().map_err(|e| e.to_string())?.query_row("SELECT manifest_id FROM connector_accounts WHERE connector_id=?1 AND connection_state!='disconnected'", params![id], |row| row.get(0)).optional().map_err(|e| e.to_string())?.ok_or_else(|| "Connector account was not found.".to_string())
}

pub(super) fn require_project_enabled(
    engine: &PersistenceEngine,
    connector_id: &str,
    project_id: &str,
) -> Result<String, String> {
    require_project_scope(engine, connector_id, project_id).map(|account| account.manifest_id)
}

pub(super) fn validate_task_binding(
    engine: &PersistenceEngine,
    project_id: &str,
    task_id: Option<&str>,
    task_run_id: Option<&str>,
) -> Result<(Option<String>, Option<String>), String> {
    let project = ProjectId::parse(project_id)?.to_string();
    let task = task_id
        .map(TaskId::parse)
        .transpose()?
        .map(|id| id.to_string());
    let task_run = task_run_id
        .map(TaskRunId::parse)
        .transpose()?
        .map(|id| id.to_string());
    let Some(run) = task_run.as_deref() else {
        return Ok((task, None));
    };
    let task = task
        .as_deref()
        .ok_or_else(|| "connector_task_id_required_for_run".to_string())?;
    let matches: Option<i64> = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT 1 FROM task_runs WHERE task_run_id=?1 AND task_id=?2 AND project_id=?3 LIMIT 1",
            params![run, task, project],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if matches.is_none() {
        return Err("connector_task_binding_mismatch".to_string());
    }
    Ok((Some(task.to_string()), Some(run.to_string())))
}

pub(super) fn record_probe(
    engine: &PersistenceEngine,
    connector_id: &str,
    state: &str,
    code: &str,
    expires_at_ms: Option<i64>,
) -> Result<(), String> {
    let changed = engine.open_connection().map_err(|e| e.to_string())?.execute("UPDATE connector_accounts SET connection_state=?2,last_probe_code=?3,last_probe_at_ms=?4,token_expires_at_ms=COALESCE(?5,token_expires_at_ms),updated_at_ms=?4 WHERE connector_id=?1", params![connector_id,state,code,unix_time_ms_i64(),expires_at_ms]).map_err(|e| e.to_string())?;
    if changed == 1 {
        Ok(())
    } else {
        Err("Connector account was not found.".to_string())
    }
}

pub(super) fn disconnect(engine: &PersistenceEngine, connector_id: &str) -> Result<(), String> {
    let id = ConnectorId::parse(connector_id)?.to_string();
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    transaction.execute("UPDATE connector_accounts SET connection_state='disconnected',account_label='',account_subject_hash=NULL,granted_scopes_json='[]',token_expires_at_ms=NULL,refresh_expires_at_ms=NULL,updated_at_ms=?2 WHERE connector_id=?1", params![id,unix_time_ms_i64()]).map_err(|e| e.to_string())?;
    transaction
        .execute(
            "DELETE FROM connector_project_bindings WHERE connector_id=?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
    transaction
        .execute(
            "DELETE FROM connector_account_metadata WHERE connector_id=?1",
            params![id],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

pub(super) fn setup_state(engine: &PersistenceEngine) -> Result<SetupState, String> {
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    connection.execute("INSERT OR IGNORE INTO setup_progress (singleton,current_step,updated_at_ms) VALUES (1,'welcome',?1)", params![unix_time_ms_i64()]).map_err(|e| e.to_string())?;
    connection.execute("UPDATE setup_progress SET current_step='finished',sample_project_id=(SELECT project_id FROM activation_receipts ORDER BY verified_at_ms DESC LIMIT 1),completed_at_ms=COALESCE(completed_at_ms,(SELECT verified_at_ms FROM activation_receipts ORDER BY verified_at_ms DESC LIMIT 1)),updated_at_ms=MAX(updated_at_ms,(SELECT verified_at_ms FROM activation_receipts ORDER BY verified_at_ms DESC LIMIT 1)) WHERE singleton=1 AND current_step!='finished' AND sample_project_id IS NULL AND EXISTS (SELECT 1 FROM activation_receipts)", []).map_err(|e| e.to_string())?;
    connection.query_row("SELECT current_step,model_path,completion_channel,sample_project_id,completed_at_ms FROM setup_progress WHERE singleton=1", [], |row| Ok(SetupState { current_step: row.get(0)?, model_path: row.get(1)?, completion_channel: row.get(2)?, sample_project_id: row.get(3)?, completed_at_ms: row.get(4)? })).map_err(|e| e.to_string())
}

pub(super) fn save_setup(
    engine: &PersistenceEngine,
    step: &str,
    model_path: Option<&str>,
    channel: Option<&str>,
) -> Result<SetupState, String> {
    const STEPS: [&str; 8] = [
        "welcome",
        "model",
        "permissions",
        "connectors",
        "channel",
        "sample",
        "complete",
        "finished",
    ];
    if !STEPS.contains(&step) {
        return Err("Unknown setup step.".to_string());
    }
    let current = setup_state(engine)?;
    if current.current_step == "finished" {
        return Ok(current);
    }
    engine.open_connection().map_err(|e| e.to_string())?.execute("UPDATE setup_progress SET current_step=?1,model_path=COALESCE(?2,model_path),completion_channel=?3,updated_at_ms=?4 WHERE singleton=1 AND current_step!='finished' AND (sample_project_id IS NOT NULL OR NOT EXISTS (SELECT 1 FROM activation_receipts))", params![step,model_path,channel,unix_time_ms_i64()]).map_err(|e| e.to_string())?;
    setup_state(engine)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completed_setup_fixture(engine: &PersistenceEngine) -> (String, i64) {
        let project = crate::projects::repository::create(
            engine,
            crate::projects::CreateProjectRequest {
                name: "Verified setup".into(),
                description: String::new(),
                data_policy: crate::projects::ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let task_id = crate::p0_contracts::TaskId::new().to_string();
        let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
        let verified_at = unix_time_ms_i64();
        engine.open_connection().unwrap().execute("INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,completed_at_ms,recovery_state) VALUES (?1,?2,?3,'taskflow',?4,'completed','test',?2,'Verified setup',?5,?5,?5,'reconciled')", params![task_run_id,task_id,project.project_id,format!("setup-{task_run_id}"),verified_at]).unwrap();
        engine.open_connection().unwrap().execute("INSERT INTO activation_receipts (receipt_id,project_id,task_run_id,model_route,capability_snapshot_json,verified_at_ms) VALUES (?1,?2,?3,'local','{}',?4)", params![format!("receipt-{task_run_id}"),project.project_id,task_run_id,verified_at]).unwrap();
        (project.project_id, verified_at)
    }

    #[test]
    fn microsoft_identity_grants_and_routing_round_trip_without_keychain_reads() {
        let root =
            std::env::temp_dir().join(format!("oomu-microsoft-metadata-{}", unix_time_ms_i64()));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let connector_id =
            create_account(&engine, super::super::microsoft365::MANIFEST_ID, 1).unwrap();
        record_oauth_attempt(
            &engine,
            "oauth_microsoft_metadata",
            &connector_id,
            "state-hash",
            "http://127.0.0.1:4100/oauth/callback",
            unix_time_ms_i64() + 60_000,
        )
        .unwrap();
        let subject = "microsoft_365\0tenant-a\0account-a";
        let metadata = ConnectorIdentityMetadata {
            tenant_id: "tenant-a".to_string(),
            tenant_label: "Example Tenant".to_string(),
            account_id: "account-a".to_string(),
            account_principal: "person@example.com".to_string(),
            account_kind: "work".to_string(),
            identity_binding_hash: sha256_hex(subject.as_bytes()),
            data_routing: vec!["https://graph.microsoft.com".to_string()],
            consent_reviewed_at_ms: unix_time_ms_i64(),
            identity_verified_at_ms: unix_time_ms_i64(),
        };
        finish_oauth(
            &engine,
            "oauth_microsoft_metadata",
            &connector_id,
            "person@example.com",
            subject,
            &["User.Read".to_string(), "Mail.Read".to_string()],
            Some(unix_time_ms_i64() + 3_600_000),
            None,
            Some(&metadata),
        )
        .unwrap();

        let account = list_accounts(&engine)
            .unwrap()
            .into_iter()
            .find(|account| account.connector_id == connector_id)
            .unwrap();
        assert_eq!(account.tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(account.account_id.as_deref(), Some("account-a"));
        assert_eq!(
            account.account_principal.as_deref(),
            Some("person@example.com")
        );
        assert_eq!(account.data_routing, metadata.data_routing);
        let mail_read = account
            .capability_grants
            .iter()
            .find(|grant| grant.capability_id == super::super::microsoft365::OUTLOOK_MAIL_READ)
            .unwrap();
        let mail_draft = account
            .capability_grants
            .iter()
            .find(|grant| grant.capability_id == super::super::microsoft365::OUTLOOK_MAIL_DRAFT)
            .unwrap();
        assert!(mail_read.granted);
        assert!(!mail_draft.granted);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_incremental_consent_preserves_an_authorized_account() {
        let root =
            std::env::temp_dir().join(format!("oomu-microsoft-preserve-{}", unix_time_ms_i64()));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let connector_id =
            create_account(&engine, super::super::microsoft365::MANIFEST_ID, 1).unwrap();
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE connector_accounts SET connection_state='authorized' WHERE connector_id=?1",
                params![connector_id],
            )
            .unwrap();
        record_oauth_attempt(
            &engine,
            "oauth_incremental_failure",
            &connector_id,
            "state-hash",
            "http://127.0.0.1:4200/oauth/callback",
            unix_time_ms_i64() + 60_000,
        )
        .unwrap();
        fail_oauth(
            &engine,
            "oauth_incremental_failure",
            "microsoft_token_access_denied",
            true,
        )
        .unwrap();
        let state: String = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT connection_state FROM connector_accounts WHERE connector_id=?1",
                params![connector_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "authorized");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn account_listing_hides_orphaned_oauth_shells_but_keeps_live_attempts() {
        let root = std::env::temp_dir().join(format!("oomu-oauth-shells-{}", unix_time_ms_i64()));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let orphaned = create_account(&engine, "google_workspace", 1).unwrap();
        let pending = create_account(&engine, "google_workspace", 1).unwrap();
        let recoverable = create_account(&engine, "google_workspace", 1).unwrap();
        record_oauth_attempt(
            &engine,
            "oauth_expired_attempt",
            &orphaned,
            "expired-state-hash",
            "http://127.0.0.1:4000/oauth/callback",
            unix_time_ms_i64() - 1,
        )
        .unwrap();
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE connector_accounts SET connection_state='degraded' WHERE connector_id=?1",
                params![orphaned],
            )
            .unwrap();
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE connector_accounts SET connection_state='degraded', account_subject_hash='known-identity', account_label='person@example.com' WHERE connector_id=?1",
                params![recoverable],
            )
            .unwrap();
        record_oauth_attempt(
            &engine,
            "oauth_live_attempt",
            &pending,
            "state-hash",
            "http://127.0.0.1:4000/oauth/callback",
            unix_time_ms_i64() + 60_000,
        )
        .unwrap();

        let accounts = list_accounts(&engine).unwrap();

        assert!(accounts
            .iter()
            .any(|account| account.connector_id == pending));
        assert!(accounts
            .iter()
            .any(|account| account.connector_id == recoverable));
        assert!(!accounts
            .iter()
            .any(|account| account.connector_id == orphaned));
        let orphaned_state: String = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT connection_state FROM connector_accounts WHERE connector_id=?1",
                params![orphaned],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(orphaned_state, "disconnected");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn task_run_binding_requires_the_exact_task_and_project_relation() {
        let root = std::env::temp_dir().join(format!(
            "oomu-connector-task-binding-{}",
            unix_time_ms_i64()
        ));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let first = crate::projects::repository::create(
            &engine,
            crate::projects::CreateProjectRequest {
                name: "First binding project".into(),
                description: String::new(),
                data_policy: crate::projects::ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let second = crate::projects::repository::create(
            &engine,
            crate::projects::CreateProjectRequest {
                name: "Second binding project".into(),
                description: String::new(),
                data_policy: crate::projects::ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let task_id = TaskId::new().to_string();
        let other_task_id = TaskId::new().to_string();
        let run_id = TaskRunId::new().to_string();
        let now = unix_time_ms_i64();
        engine.open_connection().unwrap().execute(
            "INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'taskflow',?4,'running','test',?2,'Binding test',?5,?5,'not_required')",
            params![run_id, task_id, first.project_id, format!("binding-{run_id}"), now],
        ).unwrap();

        assert_eq!(
            validate_task_binding(&engine, &first.project_id, Some(&task_id), Some(&run_id))
                .unwrap(),
            (Some(task_id.clone()), Some(run_id.clone()))
        );
        assert_eq!(
            validate_task_binding(&engine, &second.project_id, Some(&task_id), Some(&run_id))
                .unwrap_err(),
            "connector_task_binding_mismatch"
        );
        assert_eq!(
            validate_task_binding(
                &engine,
                &first.project_id,
                Some(&other_task_id),
                Some(&run_id)
            )
            .unwrap_err(),
            "connector_task_binding_mismatch"
        );
        assert_eq!(
            validate_task_binding(&engine, &first.project_id, None, Some(&run_id)).unwrap_err(),
            "connector_task_id_required_for_run"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn finished_setup_cannot_be_regressed_by_a_late_step_write() {
        let root =
            std::env::temp_dir().join(format!("oomu-setup-monotonic-{}", unix_time_ms_i64()));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        save_setup(&engine, "finished", Some("local"), Some("local")).unwrap();

        let state = save_setup(&engine, "model", Some("replacement"), None).unwrap();

        assert_eq!(state.current_step, "finished");
        assert_eq!(state.model_path.as_deref(), Some("local"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn activation_receipt_repairs_stale_progress_and_survives_reset_state() {
        let root = std::env::temp_dir().join(format!("oomu-setup-receipt-{}", unix_time_ms_i64()));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        save_setup(&engine, "connectors", Some("local"), Some("local")).unwrap();
        let (project_id, verified_at) = completed_setup_fixture(&engine);

        let repaired = setup_state(&engine).unwrap();
        assert_eq!(repaired.current_step, "finished");
        assert_eq!(
            repaired.sample_project_id.as_deref(),
            Some(project_id.as_str())
        );
        assert_eq!(repaired.completed_at_ms, Some(verified_at));

        let connection = engine.open_connection().unwrap();
        crate::db::purge_transient_sqlite_cache_on_connection(&connection).unwrap();
        drop(connection);
        assert_eq!(setup_state(&engine).unwrap().current_step, "finished");
        let receipt_count: i64 = engine
            .open_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM activation_receipts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(receipt_count, 1);
        let _ = std::fs::remove_dir_all(root);
    }
}
