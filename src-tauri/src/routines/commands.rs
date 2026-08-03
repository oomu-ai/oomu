use super::*;
use crate::{
    db::PersistenceEngine, foundation::clock::unix_time_ms_i64,
    sovereign_identity::SovereignIdentity,
};
use rand_core::{OsRng, RngCore};
use rusqlite::params;
use serde_json::Value;
use tauri::Emitter;
#[cfg(target_os = "macos")]
use tauri_plugin_opener::OpenerExt;

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|e| e.to_string())?
}

fn ensure_background_node_identity(identity: &SovereignIdentity) -> Result<(), String> {
    identity
        .generate_node_identity()
        .map(|_| ())
        .map_err(|error| error.message)
}

#[tauri::command]
pub async fn propose_routine(request: ProposeRoutineRequest) -> Result<RoutineProposal, String> {
    blocking(move || parser::propose(&request.text, &request.timezone, unix_time_ms_i64())).await
}

#[tauri::command]
pub async fn create_routine(
    request: CreateRoutineRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<RoutineRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::create(&engine, request)).await
}

#[tauri::command]
pub async fn list_routines(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<RoutineRecord>, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::list(&engine)).await
}

#[tauri::command]
pub async fn get_routine(
    request: RoutineIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<RoutineRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::get(&engine, &request.routine_id)).await
}

#[tauri::command]
pub async fn update_routine(
    request: UpdateRoutineRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<RoutineRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::update(&engine, request)).await
}

#[tauri::command]
pub async fn pause_routine(
    request: RoutineIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<RoutineRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::set_active(&engine, &request.routine_id, false)).await
}

#[tauri::command]
pub async fn resume_routine(
    request: RoutineIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<RoutineRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::set_active(&engine, &request.routine_id, true)).await
}

#[tauri::command]
pub async fn delete_routine(
    request: DeleteRoutineRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<(), String> {
    if !request.confirmed {
        return Err("routine_delete_confirmation_required".to_string());
    }
    let engine = persistence.inner().clone();
    blocking(move || repository::delete(&engine, &request.routine_id)).await
}

#[tauri::command]
pub async fn duplicate_routine(
    request: RoutineIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<RoutineRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::duplicate(&engine, &request.routine_id)).await
}

#[tauri::command]
pub async fn run_routine_now(
    request: RoutineIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<RoutineRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || queue_routine_run_now_at(&engine, &request.routine_id, unix_time_ms_i64()))
        .await
}

fn queue_routine_run_now_at(
    engine: &PersistenceEngine,
    routine_id: &str,
    now_ms: i64,
) -> Result<RoutineRecord, String> {
    let routine = repository::get(engine, routine_id)?;
    if matches!(
        routine.delivery_state.as_deref(),
        Some("retrying" | "needs_review")
    ) {
        return Err("routine_delivery_in_progress".to_string());
    }

    let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let state = load_run_now_state(&transaction, routine_id)?;
    let Some(run_request) = prepare_run_now_request(&state, now_ms)? else {
        transaction.commit().map_err(|e| e.to_string())?;
        return repository::get(engine, routine_id);
    };
    transaction
        .execute(
            "UPDATE workflow_schedules SET is_active=1,next_run_at_ms=?2,claimed_at_ms=NULL,paused_reason=NULL,run_request_json=?3,updated_at_ms=?2 WHERE id=?1",
            params![routine_id, now_ms, run_request.to_string()],
        )
        .map_err(|e| e.to_string())?;
    transaction.commit().map_err(|e| e.to_string())?;
    repository::get(engine, routine_id)
}

struct RunNowState {
    run_request: Value,
    is_active: bool,
    next_run_at_ms: Option<i64>,
    claimed_at_ms: Option<i64>,
    last_status: Option<String>,
    schedule_kind: String,
    expression: String,
    timezone: String,
}

fn load_run_now_state(
    transaction: &rusqlite::Transaction<'_>,
    routine_id: &str,
) -> Result<RunNowState, String> {
    let raw: (
        String,
        bool,
        Option<i64>,
        Option<i64>,
        Option<String>,
        String,
        String,
        String,
    ) = transaction
        .query_row(
            "SELECT run_request_json,is_active,next_run_at_ms,claimed_at_ms,last_status,schedule_kind,schedule_expression,routine_timezone FROM workflow_schedules WHERE id=?1 AND id LIKE 'routine_%'",
            params![routine_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    Ok(RunNowState {
        run_request: serde_json::from_str(&raw.0).map_err(|e| e.to_string())?,
        is_active: raw.1,
        next_run_at_ms: raw.2,
        claimed_at_ms: raw.3,
        last_status: raw.4,
        schedule_kind: raw.5,
        expression: raw.6,
        timezone: raw.7,
    })
}

fn prepare_run_now_request(state: &RunNowState, now_ms: i64) -> Result<Option<Value>, String> {
    if crate::routines::control::run_now_resume_at_ms(&state.run_request)?.is_some() {
        return Ok(None);
    }
    if state.claimed_at_ms.is_some()
        || matches!(
            state.last_status.as_deref(),
            Some("Running" | "AwaitingApproval")
        )
    {
        return Err("routine_run_in_progress".to_string());
    }
    if state.is_active && state.next_run_at_ms.is_some_and(|next| next <= now_ms) {
        return Ok(None);
    }
    if crate::routines::control::end_at_ms(&state.run_request)?
        .is_some_and(|end_at_ms| now_ms >= end_at_ms)
    {
        return Err("routine_end_time_reached".to_string());
    }

    let run_request = if state.schedule_kind == "recurring" {
        let resume_at_ms = if state.is_active {
            state.next_run_at_ms
        } else {
            None
        }
        .map(Ok)
        .unwrap_or_else(|| {
            crate::schedule_expression::next_run_after_in_timezone(
                &state.expression,
                &state.timezone,
                now_ms,
            )
        })?;
        crate::routines::control::with_run_now_resume_at_ms(&state.run_request, resume_at_ms)?
    } else {
        state.run_request.clone()
    };
    Ok(Some(run_request))
}

#[tauri::command]
pub async fn get_routine_history(
    request: RoutineIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<Value>, String> {
    let engine = persistence.inner().clone();
    blocking(move || history::get(&engine, &request.routine_id)).await
}

#[tauri::command]
pub async fn set_background_service_enabled(
    enabled: bool,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    supervisor: tauri::State<'_, BackgroundRuntimeSupervisor>,
    app: tauri::AppHandle,
) -> Result<BackgroundServiceStatus, String> {
    if enabled {
        if !crate::settings::get_privacy_settings(app.clone())?.license_accepted {
            return Err("Accept the license before enabling background operation.".to_string());
        }
        persistence.require_durable_store("enable background operation")?;
        ensure_background_node_identity(identity.inner())?;
    }
    let engine = persistence.inner().clone();
    let runtime = supervisor.inner().clone();
    let worker_app = app.clone();
    let status =
        blocking(move || background::set_enabled(worker_app, &engine, &runtime, enabled)).await?;
    let tray_visible = background::menu_should_be_visible(persistence.inner())?;
    let tray_sync_result = crate::sync_background_tray(&app, persistence.inner(), tray_visible);
    if let Err(error) = &tray_sync_result {
        eprintln!(
            "OOMU_BACKGROUND_TRAY_SYNC_FAILED {}",
            crate::redaction::redacted_log_text(error)
        );
        background::record_runtime_attention(
            persistence.inner(),
            "background_menu_evidence_failed",
        );
    }
    if tray_sync_result.is_ok() && status.state == "off" {
        background::record_disabled_verified(persistence.inner());
    }
    let engine = persistence.inner().clone();
    let status = blocking(move || background::status(&engine)).await?;
    let _ = app.emit(background::BACKGROUND_RUNTIME_STATUS_EVENT, status.clone());
    Ok(status)
}

#[tauri::command]
pub async fn get_background_service_status(
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<BackgroundServiceStatus, String> {
    let engine = persistence.inner().clone();
    blocking(move || background::status(&engine)).await?;
    let tray_visible = background::menu_should_be_visible(persistence.inner())?;
    let tray_sync_result = crate::sync_background_tray(&app, persistence.inner(), tray_visible);
    if let Err(error) = &tray_sync_result {
        eprintln!(
            "OOMU_BACKGROUND_TRAY_SYNC_FAILED {}",
            crate::redaction::redacted_log_text(error)
        );
        background::record_runtime_attention(
            persistence.inner(),
            "background_menu_evidence_failed",
        );
    }
    let engine = persistence.inner().clone();
    let status = blocking(move || background::status(&engine)).await?;
    let _ = app.emit(background::BACKGROUND_RUNTIME_STATUS_EVENT, status.clone());
    Ok(status)
}

#[tauri::command]
pub fn open_background_login_items_settings(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        app.opener()
            .open_url(
                "x-apple.systempreferences:com.apple.LoginItems-Settings.extension",
                None::<&str>,
            )
            .map_err(|_| "background_settings_open_failed".to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("background_settings_unsupported".to_string())
    }
}

#[tauri::command]
pub async fn grant_routine_authority(
    request: GrantRoutineAuthorityRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    approvals: tauri::State<'_, crate::shield_gate::ShieldApprovalManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let routine = repository::get(persistence.inner(), &request.routine_id)?;
    let project_id = routine
        .project_id
        .ok_or_else(|| "Routine has no Project scope.".to_string())?;
    let action = request.action_name.trim();
    if action.is_empty() || action.len() > 160 {
        return Err("Routine authority action is invalid.".to_string());
    }
    let now = unix_time_ms_i64();
    if request.expires_at_ms <= now || request.expires_at_ms > now + 30 * 24 * 60 * 60 * 1_000 {
        return Err("Routine authority must expire within 30 days.".to_string());
    }
    let mut random = [0_u8; 24];
    OsRng.fill_bytes(&mut random);
    let token = format!(
        "routine_authority_{}",
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, random)
    );
    crate::shield_gate::request_user_approval(&app,approvals.inner(),crate::shield_gate::ShieldApprovalRequest{approval_token:token,session_id:None,turn_id:None,generation_token:None,action_type:"routine_preauthorization".to_string(),action_label:action.to_string(),target_path:None,principal:Some(project_id.clone()),risk_tier:"consequential".to_string(),reason:"This grant lets one Routine execute one exact action and argument set while OOMU is in the background.".to_string(),estimated_token_costs:None,requested_at_ms:now as u64,preview:format!("Authorize {action} for {} until {}. Arguments are bound by digest.",routine.label,request.expires_at_ms),semantic_summary:format!("Preauthorize {action} for {}",routine.label),semantic_detail:"The grant is Project-scoped, Routine-scoped, argument-bound, expiring, and revocable.".to_string(),approval_tier:"effectful".to_string(),approval_mode:"time_bounded_exact_scope".to_string(),diff_preview:None,scope_trust_available:false,scope_trust_prefix:None,scope_trust_duration_ms:0,project_id:Some(project_id.clone()),task_run_id:None,action_class:"approval_grant".to_string(),argument_class:crate::approval_scopes::argument_class("approval_grant",action),canonical_resource:Some(request.routine_id.clone()),mandatory_reconfirm:true,approval_scope_kinds:vec!["once".to_string()]}).await.map_err(|e|e.message)?;
    persistence.open_connection().map_err(|e|e.to_string())?.execute("INSERT INTO routine_authority_grants (grant_id,schedule_id,project_id,action_name,arguments_hash,expires_at_ms,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![format!("grant_{}",crate::p0_contracts::TaskId::new()),request.routine_id,project_id,action,crate::db::hash_arguments(&request.arguments),request.expires_at_ms,now]).map_err(|e|e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sovereign_identity::APP_DATA_ENV_LOCK;
    use serde_json::json;
    use std::{ffi::OsString, path::Path};

    struct AppDataRootGuard(Option<OsString>);

    impl AppDataRootGuard {
        fn set(path: &Path) -> Self {
            let previous = std::env::var_os(crate::settings::APP_DATA_ROOT_ENV);
            std::env::set_var(crate::settings::APP_DATA_ROOT_ENV, path);
            Self(previous)
        }
    }

    impl Drop for AppDataRootGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                std::env::set_var(crate::settings::APP_DATA_ROOT_ENV, previous);
            } else {
                std::env::remove_var(crate::settings::APP_DATA_ROOT_ENV);
            }
        }
    }

    #[test]
    fn background_enable_bootstraps_and_recovers_a_missing_node_identity() {
        let _env_guard = APP_DATA_ENV_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!(
            "oomu-background-identity-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ns_u128()
        ));
        std::fs::create_dir_all(&root).expect("temporary app data root creates");
        let _app_data_root = AppDataRootGuard::set(&root);
        let identity = SovereignIdentity::initialize_ephemeral();

        assert!(identity.node_identity().is_err());
        ensure_background_node_identity(&identity).expect("missing node identity bootstraps");
        let created = identity.node_identity().expect("created identity loads");
        let identity_path = Path::new(&created.identity_dir).join("node_identity.json");
        std::fs::remove_file(&identity_path).expect("identity loss is simulated");

        ensure_background_node_identity(&identity).expect("missing node identity recovers");
        let recovered = identity.node_identity().expect("recovered identity loads");
        assert_eq!(recovered.node_id, created.node_id);
        assert_eq!(recovered.public_key, created.public_key);
        assert!(identity_path.is_file());

        std::fs::remove_dir_all(&root).expect("temporary app data root removes");
    }

    #[test]
    fn run_now_preserves_the_recurring_anchor_and_end_boundary_once() {
        let root = std::env::temp_dir().join(format!(
            "oomu-run-now-anchor-{}",
            crate::p0_contracts::TaskId::new()
        ));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let now = 1_000_000;
        let scheduled = now + 60 * 60 * 1_000;
        let end = now + 12 * 60 * 60 * 1_000;
        let run_request = crate::routines::control::with_end_at_ms(&json!({}), end).unwrap();
        engine
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO workflow_schedules(id,workflow_id,label,schedule_expression,run_request_json,is_active,next_run_at_ms,created_at_ms,updated_at_ms,schedule_kind,routine_timezone) VALUES ('routine_task_run_now','workflow','Run now','every 1 hour',?1,1,?2,1,1,'recurring','UTC')",
                params![run_request.to_string(), scheduled],
            )
            .unwrap();

        let queued = queue_routine_run_now_at(&engine, "routine_task_run_now", now).unwrap();
        assert_eq!(queued.next_run_at_ms, Some(now));
        let saved = engine
            .load_workflow_schedule("routine_task_run_now")
            .unwrap();
        assert_eq!(
            crate::routines::control::run_now_resume_at_ms(&saved.run_request).unwrap(),
            Some(scheduled)
        );
        assert_eq!(
            crate::routines::control::end_at_ms(&saved.run_request).unwrap(),
            Some(end)
        );

        let duplicate = queue_routine_run_now_at(&engine, "routine_task_run_now", now + 1).unwrap();
        assert_eq!(duplicate.next_run_at_ms, Some(now));
        let saved = engine
            .load_workflow_schedule("routine_task_run_now")
            .unwrap();
        assert_eq!(
            crate::routines::control::run_now_resume_at_ms(&saved.run_request).unwrap(),
            Some(scheduled)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_now_coalesces_with_an_occurrence_that_is_already_due() {
        let root = std::env::temp_dir().join(format!(
            "oomu-run-now-due-{}",
            crate::p0_contracts::TaskId::new()
        ));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let now = 1_000_000;
        let due = now - 30_000;
        engine
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO workflow_schedules(id,workflow_id,label,schedule_expression,run_request_json,is_active,next_run_at_ms,created_at_ms,updated_at_ms,schedule_kind,routine_timezone) VALUES ('routine_task_already_due','workflow','Already due','every 1 hour','{}',1,?1,1,1,'recurring','UTC')",
                params![due],
            )
            .unwrap();

        let unchanged = queue_routine_run_now_at(&engine, "routine_task_already_due", now).unwrap();
        assert_eq!(unchanged.next_run_at_ms, Some(due));
        let saved = engine
            .load_workflow_schedule("routine_task_already_due")
            .unwrap();
        assert_eq!(
            crate::routines::control::run_now_resume_at_ms(&saved.run_request).unwrap(),
            None
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
