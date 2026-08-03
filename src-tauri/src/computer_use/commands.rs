use super::{
    contracts::{
        AppControlSessionRequest, AppControlSessionView, ControlAppControlSessionRequest,
        DesktopActionOutcome, DesktopObservation, ExecuteDesktopActionRequest,
        GetAppControlStatusRequest, StartAppControlSession, StartAppControlSessionRequest,
    },
    error::{AppControlError, AppControlErrorCode},
    manager::AppControlManager,
    policy::app_profile,
};

pub(super) fn get_app_control_status_impl(
    request: GetAppControlStatusRequest,
    manager: tauri::State<'_, AppControlManager>,
) -> Result<Option<AppControlSessionView>, AppControlError> {
    manager.get_status(request.task_run_id.as_deref())
}

pub(super) fn control_app_control_session_impl(
    request: ControlAppControlSessionRequest,
    manager: tauri::State<'_, AppControlManager>,
) -> Result<AppControlSessionView, AppControlError> {
    let should_reobserve = request.control == super::contracts::AppControlControl::ReturnToOomu;
    let session_id = request.session_id.clone();
    let task_run_id = request.task_run_id.clone();
    let view = manager.control(request)?;
    if !should_reobserve {
        return Ok(view);
    }
    manager.observe(&session_id, &task_run_id)?;
    manager
        .get_status(Some(&task_run_id))?
        .filter(|current| current.session_id == session_id)
        .ok_or_else(|| {
            AppControlError::new(
                AppControlErrorCode::SessionNotFound,
                "The app control session was not found after handback.",
            )
        })
}

pub(super) fn start_app_control_session_impl(
    request: StartAppControlSessionRequest,
    manager: tauri::State<'_, AppControlManager>,
) -> Result<AppControlSessionView, AppControlError> {
    manager.start_session(StartAppControlSession {
        project_id: request.project_id,
        task_run_id: request.task_run_id,
        approved_bundle_ids: request.approved_bundle_ids,
        // File-backed actions stay unavailable until a picker-issued root grant
        // is supplied; renderer or model paths are never treated as authority.
        scoped_file_roots: Vec::new(),
        file_grant_ids: request.file_grant_ids,
    })
}

pub(super) fn observe_app_control_session_impl(
    request: AppControlSessionRequest,
    manager: tauri::State<'_, AppControlManager>,
) -> Result<DesktopObservation, AppControlError> {
    manager.observe(&request.session_id, &request.task_run_id)
}

pub(super) async fn review_and_execute_app_control_action_impl(
    request: ExecuteDesktopActionRequest,
    manager: tauri::State<'_, AppControlManager>,
    approvals: tauri::State<'_, crate::shield_gate::ShieldApprovalManager>,
    app: tauri::AppHandle,
) -> Result<DesktopActionOutcome, AppControlError> {
    review_and_execute_app_control_action_core(request, manager.inner(), approvals.inner(), &app)
        .await
}

pub(super) async fn review_and_execute_app_control_action_core(
    request: ExecuteDesktopActionRequest,
    manager: &AppControlManager,
    approvals: &crate::shield_gate::ShieldApprovalManager,
    app: &tauri::AppHandle,
) -> Result<DesktopActionOutcome, AppControlError> {
    let authority = manager.authority_request_for(&request)?;
    let binding = AppControlManager::approval_binding(&authority);
    let app_name = app_profile(&authority.bundle_id, "Application").display_name;
    let mutating = authority.will_change_data;
    let preview = serde_json::to_string(&serde_json::json!({
        "schemaVersion": 1,
        "appName": app_name,
        "actionKind": authority.action_kind,
    }))
    .map_err(|_| {
        AppControlError::new(
            AppControlErrorCode::InvalidRequest,
            "app_control.approval_preview_invalid",
        )
    })?;
    crate::shield_gate::request_user_approval(
        app,
        approvals,
        crate::shield_gate::ShieldApprovalRequest {
            approval_token: super::references::opaque_id("appapproval"),
            session_id: Some(authority.session_id.clone()),
            turn_id: None,
            generation_token: None,
            action_type: "app_control".to_string(),
            action_label: "app_control.action_label".to_string(),
            target_path: None,
            principal: Some(binding.principal.clone()),
            risk_tier: if mutating { "consequential" } else { "guarded" }.to_string(),
            reason: "app_control.approval_reason".to_string(),
            estimated_token_costs: None,
            requested_at_ms: crate::foundation::clock::unix_time_ms_u64(),
            preview,
            semantic_summary: "app_control.approval_summary".to_string(),
            semantic_detail: "app_control.approval_detail".to_string(),
            approval_tier: if mutating { "effectful" } else { "guarded" }.to_string(),
            approval_mode: "one_action_or_task_scope".to_string(),
            diff_preview: None,
            scope_trust_available: true,
            scope_trust_prefix: Some(binding.canonical_resource.clone()),
            scope_trust_duration_ms: 15 * 60 * 1_000,
            project_id: Some(authority.project_id.clone()),
            task_run_id: Some(authority.task_run_id.clone()),
            action_class: binding.action_class,
            argument_class: binding.argument_class,
            canonical_resource: Some(binding.canonical_resource),
            mandatory_reconfirm: false,
            approval_scope_kinds: vec!["once".to_string(), "task".to_string()],
        },
    )
    .await
    .map_err(|_| {
        AppControlError::new(
            AppControlErrorCode::Unauthorized,
            "The app action was not approved.",
        )
    })?;
    manager.register_direct_approval(&authority)?;
    manager.execute(request)
}
