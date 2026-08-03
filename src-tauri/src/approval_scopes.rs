use crate::{
    db::PersistenceEngine,
    foundation::{clock::unix_time_ms_i64, digest::sha256_hex},
    shield_gate::{ScopeTrustApprovalRequest, ShieldApprovalRequest},
};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewedApprovalGrant {
    pub grant_id: String,
    pub scope_kind: String,
    pub principal: String,
    pub project_id: Option<String>,
    pub task_run_id: Option<String>,
    pub action_class: String,
    pub canonical_resource: String,
    pub argument_class: String,
    pub expires_at_ms: i64,
    pub max_uses: u32,
    pub used_count: u32,
    pub reviewed_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
    pub active: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalScopeAuditEvent {
    pub id: i64,
    pub grant_id: Option<String>,
    pub task_run_id: Option<String>,
    pub event_type: String,
    pub action_class: String,
    pub canonical_resource_hash: String,
    pub detail: serde_json::Value,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalScopeDashboard {
    pub grants: Vec<ReviewedApprovalGrant>,
    pub audit_events: Vec<ApprovalScopeAuditEvent>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalScopeFilter {
    pub project_id: Option<String>,
    pub task_run_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevokeApprovalScopeRequest {
    pub grant_id: String,
    pub reason: Option<String>,
}

fn grant_id() -> String {
    let mut bytes = [0u8; 18];
    OsRng.fill_bytes(&mut bytes);
    format!("trustgrant_{}", hex::encode(bytes))
}

pub(crate) fn mandatory_reconfirmation(action: &str) -> bool {
    matches!(
        action.trim().to_ascii_lowercase().as_str(),
        "terminal_execute"
            | "shell_command"
            | "execute_command"
            | "delete_file"
            | "trash"
            | "credential_disclosure"
            | "message_send"
            | "message_post"
            | "create_decision_pack"
            | "draft_decision_pack_email"
            | "create_conflict_free_calendar_event"
            | "calendar_mutation_with_others"
            | "external_private_export"
            | "airlock_export"
            | "telemetry_archive"
            | "artifact_export"
            | "browser_submit"
            | "connector_write"
    )
}

pub(crate) fn canonical_resource(path: Option<&str>, fallback: &str) -> String {
    path.and_then(|value| Path::new(value).canonicalize().ok())
        .map(|value| value.to_string_lossy().to_string())
        .or_else(|| path.map(str::to_string))
        .unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn argument_class(action: &str, preview: &str) -> String {
    let normalized = action.trim().to_ascii_lowercase();
    // Folder permissions describe an operation and canonical folder. Their
    // validity must not change merely because a later file has a different size.
    if matches!(normalized.as_str(), "filesystem_read" | "filesystem_write") {
        return normalized;
    }
    let size = match preview.len() {
        0..=255 => "small",
        256..=4095 => "medium",
        _ => "large",
    };
    format!("{normalized}:{size}")
}

pub(crate) fn grant(
    engine: &PersistenceEngine,
    approval: &ShieldApprovalRequest,
    selection: &ScopeTrustApprovalRequest,
) -> Result<Option<String>, String> {
    let kind = selection.kind.as_deref().unwrap_or(if selection.enabled {
        "project_path"
    } else {
        "once"
    });
    if kind == "once" || !selection.enabled {
        return Ok(None);
    }
    if approval.mandatory_reconfirm
        || (approval.action_class != "filesystem_read"
            && mandatory_reconfirmation(&approval.action_type))
    {
        return Err("This high-impact action always requires one-time reconfirmation.".into());
    }
    if !approval
        .approval_scope_kinds
        .iter()
        .any(|allowed| allowed == kind)
    {
        return Err("The selected approval scope is not available for this action.".into());
    }
    let now = unix_time_ms_i64();
    let requested = selection.duration_ms.unwrap_or(15 * 60 * 1000) as i64;
    let max_duration = match kind {
        "task" => 24 * 60 * 60 * 1000,
        "project_path" => 7 * 24 * 60 * 60 * 1000,
        // SQLite requires a concrete timestamp. Year 9999 represents
        // "until revoked" without pretending this grant expires in 30 days.
        "persistent" => 253_402_300_799_000_i64.saturating_sub(now),
        _ => return Err("Unsupported approval scope.".into()),
    };
    let expires = if kind == "persistent" {
        253_402_300_799_000_i64
    } else {
        now + requested.clamp(60_000, max_duration)
    };
    let max_uses = selection
        .max_uses
        .unwrap_or(match kind {
            "task" => 25,
            "project_path" => 100,
            "persistent" => 500,
            _ => 1,
        })
        .clamp(1, 10_000);
    if kind == "task" && approval.task_run_id.is_none() {
        return Err("Task trust requires a bound Task.".into());
    }
    if kind == "project_path"
        && (approval.project_id.is_none() || approval.canonical_resource.is_none())
    {
        return Err("Project/path trust requires a bound Project and canonical resource.".into());
    }
    let id = grant_id();
    let filesystem_scope = matches!(
        approval.action_class.as_str(),
        "filesystem_read" | "filesystem_write"
    );
    let resource = if filesystem_scope {
        approval
            .scope_trust_prefix
            .clone()
            .or_else(|| approval.canonical_resource.clone())
    } else {
        approval
            .canonical_resource
            .clone()
            .or_else(|| approval.scope_trust_prefix.clone())
    }
    .unwrap_or_else(|| approval.action_type.clone());
    let principal = approval
        .principal
        .clone()
        .unwrap_or_else(|| "local_principal".into());
    let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    transaction.execute("INSERT INTO reviewed_approval_scopes (grant_id,scope_kind,principal,project_id,task_run_id,action_class,canonical_resource,argument_class,expires_at_ms,max_uses,resource_budget_json,reviewed_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![id,kind,principal,approval.project_id,approval.task_run_id,approval.action_class,resource,approval.argument_class,expires,max_uses,json!({"maxUses":max_uses,"expiresAtMs":expires,"untilRevoked":kind == "persistent"}).to_string(),now]).map_err(|e|e.to_string())?;
    audit(
        &transaction,
        Some(&id),
        approval.task_run_id.as_deref(),
        "granted",
        &approval.action_class,
        &resource,
        json!({"scopeKind":kind,"expiresAtMs":expires,"maxUses":max_uses}),
    )?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(Some(id))
}

pub(crate) fn authorize(
    engine: &PersistenceEngine,
    principal: &str,
    project_id: Option<&str>,
    task_run_id: Option<&str>,
    action_class: &str,
    resource: &str,
    argument_class: &str,
    estimated_uses: u32,
) -> Result<bool, String> {
    if mandatory_reconfirmation(action_class) {
        let connection = engine.open_connection().map_err(|e| e.to_string())?;
        audit(
            &connection,
            None,
            task_run_id,
            "mandatory_reconfirm",
            action_class,
            resource,
            json!({"reason":"irreversible_or_high_impact"}),
        )?;
        return Ok(false);
    }
    let now = unix_time_ms_i64();
    let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
    let mut statement=connection.prepare("SELECT grant_id,scope_kind,project_id,task_run_id,canonical_resource,max_uses,used_count,resource_budget_json FROM reviewed_approval_scopes WHERE principal=?1 AND action_class=?2 AND argument_class=?3 AND revoked_at_ms IS NULL AND expires_at_ms>?4 ORDER BY reviewed_at_ms DESC").map_err(|e|e.to_string())?;
    let rows = statement
        .query_map(
            params![principal, action_class, argument_class, now],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    for (id, kind, project, task, prefix, max_uses, used, resource_budget_json) in rows {
        let scope_matches = match kind.as_str() {
            "task" => task.as_deref() == task_run_id,
            "project_path" => {
                project.as_deref() == project_id
                    && Path::new(resource).starts_with(Path::new(&prefix))
            }
            "persistent" => {
                resource == prefix || Path::new(resource).starts_with(Path::new(&prefix))
            }
            _ => false,
        };
        let until_revoked = kind == "persistent"
            && serde_json::from_str::<serde_json::Value>(&resource_budget_json)
                .ok()
                .and_then(|budget| budget.get("untilRevoked").and_then(|value| value.as_bool()))
                .unwrap_or(false);
        // Historical `persistent` rows were duration- and use-bounded. Only
        // grants explicitly recorded with the new until-revoked contract may
        // ignore max_uses.
        let within_use_budget =
            until_revoked || used.saturating_add(estimated_uses as i64) <= max_uses;
        if scope_matches && within_use_budget {
            let transaction = connection.transaction().map_err(|e| e.to_string())?;
            let updated = if until_revoked {
                transaction.execute(
                    "UPDATE reviewed_approval_scopes SET used_count=used_count+?2,last_used_at_ms=?3 WHERE grant_id=?1 AND revoked_at_ms IS NULL AND expires_at_ms>?3",
                    params![id, estimated_uses, now],
                )
            } else {
                transaction.execute(
                    "UPDATE reviewed_approval_scopes SET used_count=used_count+?2,last_used_at_ms=?3 WHERE grant_id=?1 AND revoked_at_ms IS NULL AND expires_at_ms>?3 AND used_count+?2<=max_uses",
                    params![id, estimated_uses, now],
                )
            }
            .map_err(|e| e.to_string())?;
            if updated == 0 {
                transaction.rollback().map_err(|e| e.to_string())?;
                continue;
            }
            audit(
                &transaction,
                Some(&id),
                task_run_id,
                "used",
                action_class,
                resource,
                json!({"scopeKind":kind,"uses":estimated_uses}),
            )?;
            transaction.commit().map_err(|e| e.to_string())?;
            return Ok(true);
        }
    }
    let expired:Option<String>=connection.query_row("SELECT grant_id FROM reviewed_approval_scopes WHERE principal=?1 AND action_class=?2 AND argument_class=?3 AND revoked_at_ms IS NULL AND expires_at_ms<=?4 ORDER BY expires_at_ms DESC LIMIT 1",params![principal,action_class,argument_class,now],|row|row.get(0)).optional().map_err(|e|e.to_string())?;
    audit(
        &connection,
        expired.as_deref(),
        task_run_id,
        if expired.is_some() {
            "expired"
        } else {
            "denied"
        },
        action_class,
        resource,
        json!({"reason":if expired.is_some(){"scope_expired"}else{"no_matching_scope"}}),
    )?;
    if let Some(task) = task_run_id {
        let _ = crate::tools::task_runtime::record_event(
            engine,
            task,
            "trust.scope_unavailable",
            crate::p0_contracts::EvidenceClass::ObservedResult,
            json!({"actionClass":action_class,"reason":if expired.is_some(){"expired"}else{"not_granted"}}),
        );
    }
    Ok(false)
}

fn audit(
    connection: &rusqlite::Connection,
    grant_id: Option<&str>,
    task: Option<&str>,
    event: &str,
    action: &str,
    resource: &str,
    detail: serde_json::Value,
) -> Result<(), String> {
    connection.execute("INSERT INTO approval_scope_audit (grant_id,task_run_id,event_type,action_class,canonical_resource_hash,detail_json,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![grant_id,task,event,action,sha256_hex(resource.as_bytes()),detail.to_string(),unix_time_ms_i64()]).map_err(|e|e.to_string())?;
    Ok(())
}

fn dashboard(
    engine: &PersistenceEngine,
    filter: ApprovalScopeFilter,
) -> Result<ApprovalScopeDashboard, String> {
    let now = unix_time_ms_i64();
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let mut grants_statement=connection.prepare("SELECT grant_id,scope_kind,principal,project_id,task_run_id,action_class,canonical_resource,argument_class,expires_at_ms,max_uses,used_count,reviewed_at_ms,revoked_at_ms FROM reviewed_approval_scopes ORDER BY reviewed_at_ms DESC LIMIT 250").map_err(|e|e.to_string())?;
    let grants = grants_statement
        .query_map([], |row| {
            let expires: i64 = row.get(8)?;
            let revoked: Option<i64> = row.get(12)?;
            Ok(ReviewedApprovalGrant {
                grant_id: row.get(0)?,
                scope_kind: row.get(1)?,
                principal: row.get(2)?,
                project_id: row.get(3)?,
                task_run_id: row.get(4)?,
                action_class: row.get(5)?,
                canonical_resource: row.get(6)?,
                argument_class: row.get(7)?,
                expires_at_ms: expires,
                max_uses: row.get::<_, i64>(9)? as u32,
                used_count: row.get::<_, i64>(10)? as u32,
                reviewed_at_ms: row.get(11)?,
                revoked_at_ms: revoked,
                active: revoked.is_none() && expires > now,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|grant| {
            filter
                .project_id
                .as_ref()
                .is_none_or(|id| grant.project_id.as_ref() == Some(id))
                && filter
                    .task_run_id
                    .as_ref()
                    .is_none_or(|id| grant.task_run_id.as_ref() == Some(id))
        })
        .collect();
    drop(grants_statement);
    let mut audit_statement=connection.prepare("SELECT id,grant_id,task_run_id,event_type,action_class,canonical_resource_hash,detail_json,created_at_ms FROM approval_scope_audit ORDER BY created_at_ms DESC LIMIT 250").map_err(|e|e.to_string())?;
    let audit_events = audit_statement
        .query_map([], |row| {
            let raw: String = row.get(6)?;
            Ok(ApprovalScopeAuditEvent {
                id: row.get(0)?,
                grant_id: row.get(1)?,
                task_run_id: row.get(2)?,
                event_type: row.get(3)?,
                action_class: row.get(4)?,
                canonical_resource_hash: row.get(5)?,
                detail: serde_json::from_str(&raw).unwrap_or_else(|_| json!({"invalid":true})),
                created_at_ms: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(ApprovalScopeDashboard {
        grants,
        audit_events,
    })
}

#[tauri::command]
pub async fn get_reviewed_approval_scopes(
    filter: ApprovalScopeFilter,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ApprovalScopeDashboard, String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || dashboard(&engine, filter))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn revoke_reviewed_approval_scope(
    request: RevokeApprovalScopeRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<(), String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || revoke(&engine, request))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
pub(crate) fn revoke_grant_for_compensation(
    engine: &PersistenceEngine,
    grant_id: &str,
    reason: &str,
) -> Result<(), String> {
    let revoke_result = revoke(
        engine,
        RevokeApprovalScopeRequest {
            grant_id: grant_id.to_string(),
            reason: Some(reason.to_string()),
        },
    );
    if revoke_result.is_ok() {
        return Ok(());
    }

    // The waiting action is already gone, so reusable authority must fail
    // closed even if the revocation audit table itself is unavailable. The
    // original grant audit remains as forensic evidence; deleting the exact
    // new row ensures no orphaned permission can be exercised.
    let removed = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "DELETE FROM reviewed_approval_scopes WHERE grant_id=?1",
            params![grant_id],
        )
        .map_err(|error| error.to_string())?;
    if removed == 1 {
        Ok(())
    } else {
        Err(revoke_result.expect_err("failed compensation revoke has an error"))
    }
}

fn revoke(engine: &PersistenceEngine, request: RevokeApprovalScopeRequest) -> Result<(), String> {
    let now = unix_time_ms_i64();
    let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    let binding: Option<(Option<String>, String, String)> = transaction
        .query_row(
            "SELECT task_run_id,action_class,canonical_resource FROM reviewed_approval_scopes WHERE grant_id=?1 AND revoked_at_ms IS NULL",
            params![request.grant_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((task, action, resource)) = binding else {
        return Err("Approval scope was not found or is already revoked.".into());
    };
    transaction
        .execute(
            "UPDATE reviewed_approval_scopes SET revoked_at_ms=?2 WHERE grant_id=?1",
            params![request.grant_id, now],
        )
        .map_err(|e| e.to_string())?;
    audit(
        &transaction,
        Some(&request.grant_id),
        task.as_deref(),
        "revoked",
        &action,
        &resource,
        json!({"reason":request.reason.unwrap_or_else(||"manual_revocation".into())}),
    )?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atomicity_test_root(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "oomu-approval-scope-{label}-{}-{}-{}",
            std::process::id(),
            unix_time_ms_i64(),
            grant_id()
        ))
    }

    fn persistent_filesystem_approval(
        trusted_folder: &Path,
        target: &Path,
    ) -> ShieldApprovalRequest {
        ShieldApprovalRequest {
            approval_token: grant_id(),
            session_id: None,
            turn_id: None,
            generation_token: None,
            action_type: "file_read".into(),
            action_label: "View local files".into(),
            target_path: Some(target.display().to_string()),
            principal: Some("local_principal".into()),
            risk_tier: "medium".into(),
            reason: "atomicity test".into(),
            estimated_token_costs: Some(1),
            requested_at_ms: unix_time_ms_i64() as u64,
            preview: "one name".into(),
            semantic_summary: "View this folder".into(),
            semantic_detail: "atomicity test".into(),
            approval_tier: "visual_consent".into(),
            approval_mode: "visual".into(),
            diff_preview: None,
            scope_trust_available: true,
            scope_trust_prefix: Some(trusted_folder.display().to_string()),
            scope_trust_duration_ms: 60_000,
            project_id: None,
            task_run_id: None,
            action_class: "filesystem_read".into(),
            argument_class: argument_class("filesystem_read", "one name"),
            canonical_resource: Some(target.display().to_string()),
            mandatory_reconfirm: false,
            approval_scope_kinds: vec!["once".into(), "persistent".into()],
        }
    }

    fn persistent_selection() -> ScopeTrustApprovalRequest {
        ScopeTrustApprovalRequest {
            enabled: true,
            duration_ms: Some(60_000),
            kind: Some("persistent".into()),
            max_uses: Some(5),
        }
    }

    fn install_failing_audit_trigger(engine: &PersistenceEngine) {
        engine
            .open_connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_approval_scope_audit
                 BEFORE INSERT ON approval_scope_audit
                 BEGIN
                   SELECT RAISE(ABORT, 'forced approval audit failure');
                 END;",
            )
            .unwrap();
    }

    #[test]
    fn irreversible_actions_always_reconfirm() {
        for action in [
            "delete_file",
            "credential_disclosure",
            "message_send",
            "create_conflict_free_calendar_event",
            "calendar_mutation_with_others",
            "external_private_export",
            "artifact_export",
        ] {
            assert!(mandatory_reconfirmation(action));
        }
        assert!(!mandatory_reconfirmation("file_write"));
    }

    #[test]
    fn grant_rolls_back_when_its_audit_event_cannot_be_recorded() {
        let root = atomicity_test_root("atomic-grant");
        let trusted = root.join("trusted");
        std::fs::create_dir_all(&trusted).unwrap();
        let target = trusted.join("note.txt");
        std::fs::write(&target, "content").unwrap();
        let trusted = std::fs::canonicalize(trusted).unwrap();
        let target = std::fs::canonicalize(target).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        install_failing_audit_trigger(&engine);

        let result = grant(
            &engine,
            &persistent_filesystem_approval(&trusted, &target),
            &persistent_selection(),
        );

        assert!(result.is_err(), "an unaudited grant must not succeed");
        let connection = engine.open_connection().unwrap();
        let grant_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM reviewed_approval_scopes", [], |row| {
                row.get(0)
            })
            .unwrap();
        let audit_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM approval_scope_audit", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(grant_count, 0, "the hidden grant insert must roll back");
        assert_eq!(audit_count, 0);
        drop(connection);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn authorize_rolls_back_use_count_when_its_audit_event_cannot_be_recorded() {
        let root = atomicity_test_root("atomic-authorize");
        let trusted = root.join("trusted");
        std::fs::create_dir_all(&trusted).unwrap();
        let target = trusted.join("note.txt");
        std::fs::write(&target, "content").unwrap();
        let trusted = std::fs::canonicalize(trusted).unwrap();
        let target = std::fs::canonicalize(target).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let approval = persistent_filesystem_approval(&trusted, &target);
        let id = grant(&engine, &approval, &persistent_selection())
            .unwrap()
            .expect("the test grant should be stored");
        install_failing_audit_trigger(&engine);

        let result = authorize(
            &engine,
            "local_principal",
            None,
            None,
            "filesystem_read",
            &target.display().to_string(),
            &approval.argument_class,
            1,
        );

        assert!(result.is_err(), "an unaudited use must not succeed");
        let connection = engine.open_connection().unwrap();
        let (used_count, last_used_at_ms): (i64, Option<i64>) = connection
            .query_row(
                "SELECT used_count,last_used_at_ms FROM reviewed_approval_scopes WHERE grant_id=?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let used_audit_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM approval_scope_audit WHERE grant_id=?1 AND event_type='used'",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(used_count, 0, "the hidden use increment must roll back");
        assert_eq!(last_used_at_ms, None);
        assert_eq!(used_audit_count, 0);
        drop(connection);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_authorization_cannot_overspend_a_single_use_grant() {
        let root = atomicity_test_root("concurrent-use-budget");
        let trusted = root.join("trusted");
        std::fs::create_dir_all(&trusted).unwrap();
        let target = trusted.join("note.txt");
        std::fs::write(&target, "content").unwrap();
        let trusted = std::fs::canonicalize(trusted).unwrap();
        let target = std::fs::canonicalize(target).unwrap();
        let engine = std::sync::Arc::new(
            PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap(),
        );
        let id = grant_id();
        let now = unix_time_ms_i64();
        engine
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO reviewed_approval_scopes (grant_id,scope_kind,principal,action_class,canonical_resource,argument_class,expires_at_ms,max_uses,resource_budget_json,reviewed_at_ms) VALUES (?1,'persistent','local_principal','filesystem_read',?2,'filesystem_read',?3,1,?4,?5)",
                params![
                    id,
                    trusted.display().to_string(),
                    now + 60_000,
                    json!({"maxUses":1,"expiresAtMs":now + 60_000}).to_string(),
                    now,
                ],
            )
            .unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let engine = engine.clone();
                let barrier = barrier.clone();
                let resource = target.display().to_string();
                std::thread::spawn(move || {
                    barrier.wait();
                    authorize(
                        &engine,
                        "local_principal",
                        None,
                        None,
                        "filesystem_read",
                        &resource,
                        "filesystem_read",
                        1,
                    )
                })
            })
            .collect::<Vec<_>>();
        let outcomes = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(outcomes.iter().filter(|approved| **approved).count(), 1);
        let connection = engine.open_connection().unwrap();
        let used_count: i64 = connection
            .query_row(
                "SELECT used_count FROM reviewed_approval_scopes WHERE grant_id=?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        let used_audits: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM approval_scope_audit WHERE grant_id=?1 AND event_type='used'",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(used_count, 1);
        assert_eq!(used_audits, 1);
        drop(connection);
        drop(engine);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn revoke_rolls_back_when_its_audit_event_cannot_be_recorded() {
        let root = atomicity_test_root("atomic-revoke");
        let trusted = root.join("trusted");
        std::fs::create_dir_all(&trusted).unwrap();
        let target = trusted.join("note.txt");
        std::fs::write(&target, "content").unwrap();
        let trusted = std::fs::canonicalize(trusted).unwrap();
        let target = std::fs::canonicalize(target).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let id = grant(
            &engine,
            &persistent_filesystem_approval(&trusted, &target),
            &persistent_selection(),
        )
        .unwrap()
        .expect("the test grant should be stored");
        install_failing_audit_trigger(&engine);

        let result = revoke(
            &engine,
            RevokeApprovalScopeRequest {
                grant_id: id.clone(),
                reason: Some("atomicity test".into()),
            },
        );

        assert!(result.is_err(), "an unaudited revocation must not succeed");
        let connection = engine.open_connection().unwrap();
        let revoked_at_ms: Option<i64> = connection
            .query_row(
                "SELECT revoked_at_ms FROM reviewed_approval_scopes WHERE grant_id=?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        let revoked_audit_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM approval_scope_audit WHERE grant_id=?1 AND event_type='revoked'",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(revoked_at_ms, None, "the hidden revocation must roll back");
        assert_eq!(revoked_audit_count, 0);
        drop(connection);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compensation_revoke_is_transactional_and_records_its_reason() {
        let root = atomicity_test_root("compensation-revoke");
        let trusted = root.join("trusted");
        std::fs::create_dir_all(&trusted).unwrap();
        let target = trusted.join("note.txt");
        std::fs::write(&target, "content").unwrap();
        let trusted = std::fs::canonicalize(trusted).unwrap();
        let target = std::fs::canonicalize(target).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let id = grant(
            &engine,
            &persistent_filesystem_approval(&trusted, &target),
            &persistent_selection(),
        )
        .unwrap()
        .expect("the test grant should be stored");

        revoke_grant_for_compensation(&engine, &id, "approval_receiver_closed").unwrap();

        let connection = engine.open_connection().unwrap();
        let revoked_at_ms: Option<i64> = connection
            .query_row(
                "SELECT revoked_at_ms FROM reviewed_approval_scopes WHERE grant_id=?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        let audit_detail: String = connection
            .query_row(
                "SELECT detail_json FROM approval_scope_audit WHERE grant_id=?1 AND event_type='revoked'",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(revoked_at_ms.is_some());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&audit_detail).unwrap()["reason"],
            "approval_receiver_closed"
        );
        drop(connection);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compensation_fails_closed_when_revocation_audit_is_unavailable() {
        let root = atomicity_test_root("compensation-fail-closed");
        let trusted = root.join("trusted");
        std::fs::create_dir_all(&trusted).unwrap();
        let target = trusted.join("note.txt");
        std::fs::write(&target, "content").unwrap();
        let trusted = std::fs::canonicalize(trusted).unwrap();
        let target = std::fs::canonicalize(target).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let id = grant(
            &engine,
            &persistent_filesystem_approval(&trusted, &target),
            &persistent_selection(),
        )
        .unwrap()
        .expect("the test grant should be stored");
        install_failing_audit_trigger(&engine);

        revoke_grant_for_compensation(&engine, &id, "approval_receiver_closed").unwrap();

        let active_count: i64 = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM reviewed_approval_scopes WHERE grant_id=?1 AND revoked_at_ms IS NULL",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            active_count, 0,
            "compensation must never leave authority active"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn task_scope_is_argument_bound_expiring_and_revocable() {
        let root = std::env::temp_dir().join(format!("oomu-approval-scope-{}", unix_time_ms_i64()));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let project = crate::projects::repository::create(
            &engine,
            crate::projects::CreateProjectRequest {
                name: "Trust".into(),
                description: String::new(),
                data_policy: crate::projects::ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let task_id = crate::p0_contracts::TaskId::new().to_string();
        let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
        let now = unix_time_ms_i64();
        engine.open_connection().unwrap().execute("INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'taskflow','trust-test','running','test',?2,'Trust',?4,?4,'not_required')",params![task_run_id,task_id,project.project_id,now]).unwrap();
        let approval = ShieldApprovalRequest {
            approval_token: "approval-test".into(),
            session_id: None,
            turn_id: None,
            generation_token: None,
            action_type: "file_write".into(),
            action_label: "Write".into(),
            target_path: Some(root.join("note.md").to_string_lossy().to_string()),
            principal: Some("local_principal".into()),
            risk_tier: "medium".into(),
            reason: "test".into(),
            estimated_token_costs: Some(1),
            requested_at_ms: now as u64,
            preview: "small".into(),
            semantic_summary: "Write".into(),
            semantic_detail: "test".into(),
            approval_tier: "visual".into(),
            approval_mode: "visual".into(),
            diff_preview: None,
            scope_trust_available: true,
            scope_trust_prefix: Some(root.to_string_lossy().to_string()),
            scope_trust_duration_ms: 60_000,
            project_id: Some(project.project_id.clone()),
            task_run_id: Some(task_run_id.clone()),
            action_class: "file_write".into(),
            argument_class: "file_write:small".into(),
            canonical_resource: Some(root.to_string_lossy().to_string()),
            mandatory_reconfirm: false,
            approval_scope_kinds: vec!["once".into(), "task".into()],
        };
        let id = grant(
            &engine,
            &approval,
            &ScopeTrustApprovalRequest {
                enabled: true,
                duration_ms: Some(60_000),
                kind: Some("task".into()),
                max_uses: Some(1),
            },
        )
        .unwrap()
        .unwrap();
        assert!(authorize(
            &engine,
            "local_principal",
            Some(&project.project_id),
            Some(&task_run_id),
            "file_write",
            &root.to_string_lossy(),
            "file_write:small",
            1
        )
        .unwrap());
        assert!(!authorize(
            &engine,
            "local_principal",
            Some(&project.project_id),
            Some(&task_run_id),
            "file_write",
            &root.to_string_lossy(),
            "file_write:small",
            1
        )
        .unwrap());
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE reviewed_approval_scopes SET revoked_at_ms=?2 WHERE grant_id=?1",
                params![id, unix_time_ms_i64()],
            )
            .unwrap();
        assert!(!authorize(
            &engine,
            "local_principal",
            Some(&project.project_id),
            Some(&task_run_id),
            "file_write",
            &root.to_string_lossy(),
            "file_write:small",
            1
        )
        .unwrap());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn persistent_filesystem_grants_bind_folder_and_operation_until_revoked() {
        let root = std::env::temp_dir().join(format!(
            "oomu-persistent-filesystem-scope-{}-{}",
            std::process::id(),
            unix_time_ms_i64()
        ));
        let trusted = root.join("trusted");
        let sibling = root.join("sibling");
        std::fs::create_dir_all(&trusted).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let first = trusted.join("first.txt");
        let second = trusted.join("second.txt");
        let outside = sibling.join("outside.txt");
        for path in [&first, &second, &outside] {
            std::fs::write(path, "content").unwrap();
        }
        let trusted = std::fs::canonicalize(&trusted).unwrap();
        let first = std::fs::canonicalize(&first).unwrap();
        let second = std::fs::canonicalize(&second).unwrap();
        let outside = std::fs::canonicalize(&outside).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let now = unix_time_ms_i64();
        let small_arguments = argument_class("filesystem_read", "one name");
        let large_arguments = argument_class("filesystem_read", &"x".repeat(16 * 1024));
        assert_eq!(small_arguments, large_arguments);

        let approval = ShieldApprovalRequest {
            approval_token: "persistent-filesystem-approval".into(),
            session_id: None,
            turn_id: None,
            generation_token: None,
            action_type: "file_read".into(),
            action_label: "View local files".into(),
            target_path: Some(first.display().to_string()),
            principal: Some("local_principal".into()),
            risk_tier: "medium".into(),
            reason: "test".into(),
            estimated_token_costs: Some(1),
            requested_at_ms: now as u64,
            preview: "one name".into(),
            semantic_summary: "View this folder".into(),
            semantic_detail: "test".into(),
            approval_tier: "visual_consent".into(),
            approval_mode: "visual".into(),
            diff_preview: None,
            scope_trust_available: true,
            scope_trust_prefix: Some(trusted.display().to_string()),
            scope_trust_duration_ms: 60_000,
            project_id: None,
            task_run_id: None,
            action_class: "filesystem_read".into(),
            argument_class: small_arguments.clone(),
            canonical_resource: Some(first.display().to_string()),
            mandatory_reconfirm: false,
            approval_scope_kinds: vec!["once".into(), "persistent".into()],
        };
        let grant_id = grant(
            &engine,
            &approval,
            &ScopeTrustApprovalRequest {
                enabled: true,
                duration_ms: Some(60_000),
                kind: Some("persistent".into()),
                max_uses: Some(1),
            },
        )
        .unwrap()
        .expect("persistent approval should create a reviewed grant");

        assert!(authorize(
            &engine,
            "local_principal",
            None,
            None,
            "filesystem_read",
            &first.display().to_string(),
            &small_arguments,
            1,
        )
        .unwrap());
        assert!(authorize(
            &engine,
            "local_principal",
            None,
            None,
            "filesystem_read",
            &second.display().to_string(),
            &large_arguments,
            100,
        )
        .unwrap());
        assert!(!authorize(
            &engine,
            "local_principal",
            None,
            None,
            "filesystem_read",
            &outside.display().to_string(),
            &small_arguments,
            1,
        )
        .unwrap());
        assert!(!authorize(
            &engine,
            "local_principal",
            None,
            None,
            "filesystem_write",
            &second.display().to_string(),
            &argument_class("filesystem_write", "replacement"),
            1,
        )
        .unwrap());
        assert!(!authorize(
            &engine,
            "local_principal",
            None,
            None,
            "delete_file",
            &second.display().to_string(),
            &argument_class("delete_file", ""),
            1,
        )
        .unwrap());

        // Simulate a grant written by the previous contract. Historical
        // persistent grants had a real use budget and must not silently become
        // unlimited after an upgrade.
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE reviewed_approval_scopes SET max_uses=1,used_count=0,resource_budget_json=?2 WHERE grant_id=?1",
                params![
                    grant_id,
                    json!({"maxUses":1,"expiresAtMs":253_402_300_799_000_i64}).to_string()
                ],
            )
            .unwrap();
        assert!(authorize(
            &engine,
            "local_principal",
            None,
            None,
            "filesystem_read",
            &first.display().to_string(),
            &small_arguments,
            1,
        )
        .unwrap());
        assert!(!authorize(
            &engine,
            "local_principal",
            None,
            None,
            "filesystem_read",
            &second.display().to_string(),
            &small_arguments,
            1,
        )
        .unwrap());

        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE reviewed_approval_scopes SET revoked_at_ms=?2 WHERE grant_id=?1",
                params![grant_id, unix_time_ms_i64()],
            )
            .unwrap();
        assert!(!authorize(
            &engine,
            "local_principal",
            None,
            None,
            "filesystem_read",
            &first.display().to_string(),
            &small_arguments,
            1,
        )
        .unwrap());

        let _ = std::fs::remove_dir_all(root);
    }
}
