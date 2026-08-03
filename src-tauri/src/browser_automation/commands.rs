use super::*;
use crate::{
    db::PersistenceEngine,
    native_browser::NativeBrowserManager,
    p0_contracts::EvidenceClass,
    projects::{evaluate_project_policy, ProjectTransmissionRequest},
    shield_gate::{request_user_approval, ShieldApprovalManager, ShieldApprovalRequest},
    tasks,
};
use serde::Deserialize;
use serde_json::json;
use std::{fs, path::PathBuf};
use tauri::Manager;

fn load_session(
    manager: &BrowserAutomationManager,
    session_id: &str,
    task_run_id: &str,
) -> Result<BrowserSession, String> {
    let state = manager
        .state
        .lock()
        .map_err(|_| "Browser automation state is unavailable.".to_string())?;
    let session = state
        .sessions
        .get(session_id)
        .filter(|session| session.task_run_id == task_run_id)
        .ok_or_else(|| "Browser automation session was not found.".to_string())?;
    Ok(session.clone())
}

fn save_session(manager: &BrowserAutomationManager, session: BrowserSession) -> Result<(), String> {
    let mut state = manager
        .state
        .lock()
        .map_err(|_| "Browser automation state is unavailable.".to_string())?;
    let current = state
        .sessions
        .get(&session.session_id)
        .ok_or_else(|| "Browser automation session was closed.".to_string())?;
    if current.native_epoch != session.native_epoch {
        return Err("Native browser binding changed; automation was revoked.".to_string());
    }
    state.sessions.insert(session.session_id.clone(), session);
    Ok(())
}

#[tauri::command]
pub async fn start_browser_automation(
    request: StartBrowserAutomationRequest,
    manager: tauri::State<'_, BrowserAutomationManager>,
    native: tauri::State<'_, NativeBrowserManager>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<BrowserSessionView, String> {
    tasks::require_bound_task(
        persistence.inner(),
        &request.task_run_id,
        &request.project_id,
    )?;
    let binding = native.automation_binding()?;
    let policy = evaluate_project_policy(
        persistence.inner(),
        ProjectTransmissionRequest {
            project_id: request.project_id.clone(),
            task_id: None,
            destination_kind: "browser".to_string(),
            destination_origin: binding.canonical_origin.clone(),
            data_classes: vec![
                "task_instructions".to_string(),
                "browser_interaction".to_string(),
            ],
            consent: request.project_policy_consent,
        },
    )?;
    if !policy.allowed {
        return Err(if policy.consent_required {
            "Project policy requires explicit browser transmission consent."
        } else {
            "Project policy blocks browser automation."
        }
        .to_string());
    }
    let session = BrowserSession {
        session_id: opaque_id("browser"),
        task_run_id: request.task_run_id.clone(),
        project_id: request.project_id,
        canonical_origin: binding.canonical_origin,
        destination_binding: binding.destination_binding,
        native_epoch: binding.epoch,
        state: AutomationState::Automating,
        document_generation: 0,
        document_marker_key: opaque_id("dockey"),
        element_marker_key: opaque_id("elementkey"),
        current_document_marker: None,
        references: HashMap::new(),
        current_step: "Ready for first snapshot".to_string(),
        last_snapshot_at_ms: None,
        last_snapshot: None,
    };
    repository::insert_session(persistence.inner(), &session)?;
    {
        let mut state = manager
            .state
            .lock()
            .map_err(|_| "Browser automation state is unavailable.".to_string())?;
        if let Some(previous) = state.active_session.take() {
            if let Some(old) = state.sessions.get_mut(&previous) {
                old.state = AutomationState::Stopped;
            }
        }
        state.active_session = Some(session.session_id.clone());
        state
            .sessions
            .insert(session.session_id.clone(), session.clone());
    }
    tasks::record_domain_event(
        persistence.inner(),
        &request.task_run_id,
        "browser.session_started",
        EvidenceClass::ExecutedMutation,
        json!({"sessionId":session.session_id,"origin":session.canonical_origin,"destinationBinding":session.destination_binding}),
    )?;
    Ok(session.view())
}

#[tauri::command]
pub fn get_browser_automation_session(
    request: BrowserSessionRequest,
    manager: tauri::State<'_, BrowserAutomationManager>,
) -> Result<BrowserSessionView, String> {
    Ok(load_session(manager.inner(), &request.session_id, &request.task_run_id)?.view())
}

#[tauri::command]
pub fn control_browser_automation(
    request: BrowserControlRequest,
    manager: tauri::State<'_, BrowserAutomationManager>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<BrowserSessionView, String> {
    let mut session = load_session(manager.inner(), &request.session_id, &request.task_run_id)?;
    session.state = match request.control.trim() {
        "pause" if session.state == AutomationState::Automating => AutomationState::Paused,
        "takeover"
            if matches!(
                session.state,
                AutomationState::Automating | AutomationState::Paused
            ) =>
        {
            AutomationState::Takeover
        }
        "return" if session.state == AutomationState::Takeover => {
            session.references.clear();
            session.current_document_marker = None;
            AutomationState::ReturnPending
        }
        "stop"
            if !matches!(
                session.state,
                AutomationState::Closed | AutomationState::Stopped
            ) =>
        {
            AutomationState::Stopped
        }
        _ => return Err("Unsupported browser automation control transition.".to_string()),
    };
    session.current_step = format!("Control: {}", request.control.trim());
    repository::update_session(persistence.inner(), &session)?;
    save_session(manager.inner(), session.clone())?;
    tasks::record_domain_event(
        persistence.inner(),
        &request.task_run_id,
        "browser.control_changed",
        EvidenceClass::ExecutedMutation,
        json!({"sessionId":session.session_id,"control":request.control,"state":session.state}),
    )?;
    Ok(session.view())
}

#[tauri::command]
pub async fn choose_browser_upload(
    request: BrowserSessionRequest,
    manager: tauri::State<'_, BrowserAutomationManager>,
    transfers: tauri::State<'_, transfer::BrowserTransferManager>,
) -> Result<Option<transfer::UploadGrantView>, String> {
    let session = load_session(manager.inner(), &request.session_id, &request.task_run_id)?;
    if !matches!(
        session.state,
        AutomationState::Automating | AutomationState::Paused
    ) {
        return Err("Upload selection is unavailable in the current browser state.".to_string());
    }
    let Some(handle) = rfd::AsyncFileDialog::new()
        .set_title("Choose file for guarded browser upload")
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    transfers
        .issue_upload(&request.session_id, &request.task_run_id, handle.path())
        .map(Some)
}

#[tauri::command]
pub async fn execute_browser_action(
    request: ExecuteBrowserActionRequest,
    manager: tauri::State<'_, BrowserAutomationManager>,
    transfers: tauri::State<'_, transfer::BrowserTransferManager>,
    native: tauri::State<'_, NativeBrowserManager>,
    persistence: tauri::State<'_, PersistenceEngine>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    app: tauri::AppHandle,
) -> Result<BrowserActionResult, String> {
    tasks::require_bound_task(
        persistence.inner(),
        &request.task_run_id,
        &request.project_id,
    )?;
    let mut session = load_session(manager.inner(), &request.session_id, &request.task_run_id)?;
    if session.project_id != request.project_id {
        return Err("Browser action Project scope does not match the session.".to_string());
    }
    let binding = native.automation_binding()?;
    if binding.epoch != session.native_epoch
        || binding.destination_binding != session.destination_binding
    {
        session.state = AutomationState::Stopped;
        save_session(manager.inner(), session)?;
        return Err("Native browser destination binding changed; automation stopped.".to_string());
    }
    if request.step.trim().is_empty() || request.step.chars().count() > 240 {
        return Err("Browser action step must be a bounded description.".to_string());
    }
    if request.action.kind() == "status" {
        return Ok(BrowserActionResult {
            action_id: opaque_id("action"),
            action_kind: "status".to_string(),
            state: "observed".to_string(),
            observation: session.last_snapshot.clone(),
            screenshot_path: None,
            downloads: repository::list_downloads(persistence.inner(), &session.session_id)?,
            message: "Browser automation status observed.".to_string(),
        });
    }
    let snapshot_allowed = matches!(request.action, BrowserAction::Snapshot)
        && session.state == AutomationState::ReturnPending;
    if session.state != AutomationState::Automating && !snapshot_allowed {
        return Err("Browser automation is paused, in takeover, or stopped.".to_string());
    }
    if session
        .last_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.protected_interruption.as_deref())
        .is_some()
        && !matches!(
            request.action,
            BrowserAction::Snapshot | BrowserAction::Screenshot | BrowserAction::Close
        )
    {
        return Err("Protected browser interruption requires human takeover.".to_string());
    }
    let action_id = opaque_id("action");
    let action_kind = request.action.kind().to_string();
    session.current_step = request.step.trim().to_string();
    repository::record_action(
        persistence.inner(),
        &session,
        &action_id,
        &action_kind,
        request.action.reference(),
        "previewed",
        &json!({"step":session.current_step,"origin":session.canonical_origin}),
        None,
    )?;

    let target = request
        .action
        .reference()
        .map(|reference| {
            session.references.get(reference).cloned().ok_or_else(|| {
                "Browser reference is stale or unknown; take a fresh snapshot.".to_string()
            })
        })
        .transpose()?;
    if let Some(target) = target.as_ref() {
        let label = target.name.to_ascii_lowercase();
        if [
            "delete permanently",
            "erase all",
            "make payment",
            "place order",
            "buy now",
        ]
        .iter()
        .any(|needle| label.contains(needle))
        {
            return block_action(
                persistence.inner(),
                &session,
                &action_id,
                &action_kind,
                "Protected or destructive browser action requires human takeover.",
            );
        }
        if matches!(request.action, BrowserAction::Click { .. })
            && ["submit", "send", "publish", "confirm", "save"]
                .iter()
                .any(|needle| label.contains(needle))
        {
            approve(
                &app,
                approvals.inner(),
                &session,
                &action_kind,
                &format!(
                    "Submit browser action to {} ({})",
                    session.canonical_origin, target.name
                ),
            )
            .await?;
        }
    }
    if matches!(
        request.action,
        BrowserAction::UploadApprovedFile { .. } | BrowserAction::DownloadToQuarantine { .. }
    ) {
        approve(
            &app,
            approvals.inner(),
            &session,
            &action_kind,
            &format!("{} at {}", request.step, session.canonical_origin),
        )
        .await?;
    }

    match &request.action {
        BrowserAction::Navigate { url } => navigate_same_origin(&app, &session, url)?,
        BrowserAction::Snapshot => {}
        BrowserAction::Screenshot => {}
        BrowserAction::Click { .. } => {
            driver::click(&app, &session, target.as_ref().unwrap()).await?
        }
        BrowserAction::Type { text, .. } => {
            driver::type_text(&app, &session, target.as_ref().unwrap(), text).await?
        }
        BrowserAction::Select { value, .. } => {
            driver::select(&app, &session, target.as_ref().unwrap(), value).await?
        }
        BrowserAction::PressKey { key } => driver::press_key(&app, key).await?,
        BrowserAction::Scroll { delta_y } => driver::scroll(&app, *delta_y).await?,
        BrowserAction::UploadApprovedFile {
            upload_grant_id, ..
        } => {
            let payload = transfers.consume_upload(
                upload_grant_id,
                &session.session_id,
                &session.task_run_id,
            )?;
            driver::upload(
                &app,
                &session,
                target.as_ref().unwrap(),
                &payload.file_name,
                &payload.mime_type,
                &payload.base64_bytes,
            )
            .await?;
            tasks::record_domain_event(
                persistence.inner(),
                &session.task_run_id,
                "browser.upload_executed",
                EvidenceClass::ExecutedMutation,
                json!({"sessionId":session.session_id,"fileName":payload.file_name,"byteCount":payload.byte_count,"destination":session.canonical_origin}),
            )?;
        }
        BrowserAction::DownloadToQuarantine { .. } => {
            driver::click(&app, &session, target.as_ref().unwrap()).await?
        }
        BrowserAction::Wait { milliseconds } => {
            tokio::time::sleep(std::time::Duration::from_millis(
                (*milliseconds).clamp(50, 10_000),
            ))
            .await
        }
        BrowserAction::Close => {
            crate::native_browser::close_native_browser(native.clone(), app.clone())?;
            session.state = AutomationState::Closed;
        }
        BrowserAction::Status => unreachable!(),
    }

    if !matches!(
        request.action,
        BrowserAction::Snapshot | BrowserAction::Screenshot | BrowserAction::Close
    ) {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let observation = if matches!(request.action, BrowserAction::Close) {
        None
    } else {
        Some(driver::snapshot(&app, &mut session).await?)
    };
    if let Some(snapshot) = observation.as_ref() {
        session.last_snapshot_at_ms = Some(snapshot.captured_at_ms);
        session.last_snapshot = Some(snapshot.clone());
        if session.state == AutomationState::ReturnPending {
            session.state = AutomationState::Automating;
        }
    }
    let screenshot_path = if matches!(request.action, BrowserAction::Close) {
        None
    } else {
        Some(capture_evidence_screenshot(&app, &session, &action_id).await?)
    };
    let downloads = sync_downloads(&app, native.inner(), persistence.inner(), &session)?;
    let verified = request
        .expected_postcondition
        .as_deref()
        .is_some_and(|expected| {
            observation
                .as_ref()
                .is_some_and(|snapshot| snapshot_matches(snapshot, expected))
        });
    let final_state = if verified { "verified" } else { "observed" };
    let evidence_class = if verified {
        EvidenceClass::VerifiedPostcondition
    } else {
        EvidenceClass::ObservedResult
    };
    repository::record_action(
        persistence.inner(),
        &session,
        &action_id,
        &action_kind,
        request.action.reference(),
        final_state,
        &json!({"step":session.current_step,"documentGeneration":session.document_generation,"postcondition":request.expected_postcondition,"possiblePromptInjection":observation.as_ref().is_some_and(|value|value.possible_prompt_injection)}),
        screenshot_path.as_deref(),
    )?;
    repository::update_session(persistence.inner(), &session)?;
    save_session(manager.inner(), session.clone())?;
    tasks::record_domain_event(
        persistence.inner(),
        &session.task_run_id,
        "browser.action_observed",
        evidence_class,
        json!({"sessionId":session.session_id,"actionId":action_id,"actionKind":action_kind,"state":final_state,"documentGeneration":session.document_generation,"screenshotPath":screenshot_path,"downloadCount":downloads.len()}),
    )?;
    Ok(BrowserActionResult {
        action_id,
        action_kind,
        state: final_state.to_string(),
        observation,
        screenshot_path,
        downloads,
        message: if verified {
            "Browser postcondition verified."
        } else {
            "Browser action completed with an observed post-action state."
        }
        .to_string(),
    })
}

fn block_action<T>(
    engine: &PersistenceEngine,
    session: &BrowserSession,
    action_id: &str,
    kind: &str,
    message: &str,
) -> Result<T, String> {
    repository::record_action(
        engine,
        session,
        action_id,
        kind,
        None,
        "blocked",
        &json!({"reason":message}),
        None,
    )?;
    tasks::record_domain_event(
        engine,
        &session.task_run_id,
        "browser.action_blocked",
        EvidenceClass::ObservedResult,
        json!({"sessionId":session.session_id,"actionId":action_id,"reason":message}),
    )?;
    Err(message.to_string())
}

fn navigate_same_origin(
    app: &tauri::AppHandle,
    session: &BrowserSession,
    raw: &str,
) -> Result<(), String> {
    let url = url::Url::parse(raw).map_err(|_| "Browser navigation URL is invalid.".to_string())?;
    let origin = url.origin().ascii_serialization();
    if origin != session.canonical_origin {
        return Err(
            "Cross-origin navigation requires a new explicit normalized native-browser approval."
                .to_string(),
        );
    }
    app.get_webview("oomu-browser-mod")
        .ok_or_else(|| "The controlled browser view is not open.".to_string())?
        .navigate(url)
        .map_err(|error| format!("Browser navigation failed: {error}"))
}

async fn capture_evidence_screenshot(
    app: &tauri::AppHandle,
    session: &BrowserSession,
    action_id: &str,
) -> Result<String, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Unable to resolve browser evidence directory: {error}"))?
        .join("browser-evidence")
        .join(&session.session_id);
    let path = root.join(format!("{action_id}.png"));
    screenshot::capture(app, path)
        .await
        .map(|path| path.to_string_lossy().to_string())
}

fn sync_downloads(
    app: &tauri::AppHandle,
    native: &NativeBrowserManager,
    engine: &PersistenceEngine,
    session: &BrowserSession,
) -> Result<Vec<BrowserDownloadView>, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("browser-quarantine");
    for record in native.take_completed_downloads() {
        if let Ok((view, path)) = transfer::validate_download(record, &root) {
            repository::insert_download(engine, session, &view, &path.to_string_lossy())?;
            tasks::record_domain_event(
                engine,
                &session.task_run_id,
                "browser.download_quarantined",
                EvidenceClass::VerifiedPostcondition,
                json!({"sessionId":session.session_id,"downloadId":view.download_id,"fileName":view.file_name,"byteCount":view.byte_count,"sha256":view.sha256}),
            )?;
        }
    }
    repository::list_downloads(engine, &session.session_id)
}

fn snapshot_matches(snapshot: &BrowserSnapshot, expected: &str) -> bool {
    let expected = expected.trim().to_ascii_lowercase();
    !expected.is_empty()
        && (snapshot.title.to_ascii_lowercase().contains(&expected)
            || snapshot.url.to_ascii_lowercase().contains(&expected)
            || snapshot
                .nodes
                .iter()
                .any(|node| node.name.to_ascii_lowercase().contains(&expected)))
}

async fn approve(
    app: &tauri::AppHandle,
    approvals: &ShieldApprovalManager,
    session: &BrowserSession,
    action_kind: &str,
    preview: &str,
) -> Result<(), String> {
    let now = crate::foundation::clock::unix_time_ms_u64();
    request_user_approval(
        app,
        approvals,
        ShieldApprovalRequest {
            approval_token: opaque_id("approval"),
            session_id: Some(session.session_id.clone()),
            turn_id: Some(session.task_run_id.clone()),
            generation_token: Some(session.document_generation.to_string()),
            action_type: format!("browser_{action_kind}"),
            action_label: "Guarded browser action".to_string(),
            target_path: None,
            principal: Some(session.project_id.clone()),
            risk_tier: "consequential".to_string(),
            reason: "This browser action can submit data or transfer a file.".to_string(),
            estimated_token_costs: None,
            requested_at_ms: now,
            preview: preview.to_string(),
            semantic_summary: "Approve one exact guarded browser action".to_string(),
            semantic_detail: format!(
                "Bound to task {}, session {}, origin {}, and document generation {}.",
                session.task_run_id,
                session.session_id,
                session.canonical_origin,
                session.document_generation
            ),
            approval_tier: "effectful".to_string(),
            approval_mode: "single_exact_action".to_string(),
            diff_preview: None,
            scope_trust_available: false,
            scope_trust_prefix: None,
            scope_trust_duration_ms: 0,
            project_id: Some(session.project_id.clone()),
            task_run_id: Some(session.task_run_id.clone()),
            action_class: format!("browser_{action_kind}"),
            argument_class: crate::approval_scopes::argument_class(
                &format!("browser_{action_kind}"),
                preview,
            ),
            canonical_resource: Some(session.canonical_origin.clone()),
            mandatory_reconfirm: true,
            approval_scope_kinds: vec!["once".to_string()],
        },
    )
    .await
    .map_err(|error| error.message)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportBrowserDownloadRequest {
    pub session_id: String,
    pub task_run_id: String,
    pub download_id: String,
}

#[tauri::command]
pub async fn export_browser_download(
    request: ExportBrowserDownloadRequest,
    manager: tauri::State<'_, BrowserAutomationManager>,
    persistence: tauri::State<'_, PersistenceEngine>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    app: tauri::AppHandle,
) -> Result<Option<String>, String> {
    let session = load_session(manager.inner(), &request.session_id, &request.task_run_id)?;
    let (private_path, file_name) = repository::download_path(
        persistence.inner(),
        &request.download_id,
        &request.session_id,
    )?;
    let Some(destination) = rfd::AsyncFileDialog::new()
        .set_title("Export verified browser download")
        .set_file_name(&file_name)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    approve(
        &app,
        approvals.inner(),
        &session,
        "download_export",
        &format!("Export {} to the selected destination", file_name),
    )
    .await?;
    let source = PathBuf::from(private_path);
    let bytes = fs::read(&source)
        .map_err(|_| "Quarantined download is no longer available.".to_string())?;
    fs::write(destination.path(), &bytes)
        .map_err(|error| format!("Download export failed: {error}"))?;
    let exported = fs::read(destination.path())
        .map_err(|_| "Download export could not be verified.".to_string())?;
    if crate::foundation::digest::sha256_hex(&bytes)
        != crate::foundation::digest::sha256_hex(&exported)
    {
        return Err("Download export digest verification failed.".to_string());
    }
    repository::mark_download_exported(persistence.inner(), &request.download_id)?;
    tasks::record_domain_event(
        persistence.inner(),
        &session.task_run_id,
        "browser.download_exported",
        EvidenceClass::VerifiedPostcondition,
        json!({"sessionId":session.session_id,"downloadId":request.download_id,"destinationFileName":file_name,"sha256":crate::foundation::digest::sha256_hex(&exported)}),
    )?;
    Ok(Some(destination.path().to_string_lossy().to_string()))
}
