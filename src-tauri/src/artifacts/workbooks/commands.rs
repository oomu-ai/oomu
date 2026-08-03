use super::*;
use crate::{
    db::PersistenceEngine,
    foundation::digest::{sha256_file_hex, sha256_hex},
    p0_contracts::EvidenceClass,
    shield_gate::{request_user_approval, ShieldApprovalManager, ShieldApprovalRequest},
    sovereign_identity::SovereignIdentity,
    tasks,
};
use serde_json::json;
use std::{fs, path::Path};

type CommandResult<T> = Result<T, WorkbookCommandError>;

#[tauri::command]
pub async fn create_workbook(
    request: CreateWorkbookRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    app: tauri::AppHandle,
) -> CommandResult<WorkbookReviewRecord> {
    create_workbook_internal(request, persistence.inner(), identity.inner(), &app).await
}

pub(crate) async fn create_workbook_internal(
    mut request: CreateWorkbookRequest,
    persistence: &PersistenceEngine,
    identity: &SovereignIdentity,
    app: &tauri::AppHandle,
) -> CommandResult<WorkbookReviewRecord> {
    if request.workbook.revision != 1 {
        return Err(command_error(
            "workbook_create_invalid",
            "New workbooks must begin at revision 1.",
        ));
    }
    let task = tasks::require_bound_task(persistence, &request.task_run_id, &request.project_id)
        .map_err(|error| command_error("workbook_context_invalid", error))?;
    if task.task_id != request.task_id {
        return Err(command_error(
            "workbook_context_invalid",
            "Task ID does not match the bound Task run.",
        ));
    }
    validate_workbook(&request.workbook)
        .map_err(|error| command_error("workbook_create_invalid", error))?;
    bind_workbook_provenance(
        persistence,
        &request.project_id,
        &request.task_id,
        &request.task_run_id,
        &mut request.workbook,
    )
    .map_err(|error| command_error("workbook_evidence_invalid", error))?;
    let (artifact_id, revision) = repository::create_record(persistence, &request)
        .map_err(|error| command_error("workbook_create_failed", error))?;
    if let Err(error) = tasks::record_domain_event(
        persistence,
        &request.task_run_id,
        "workbook.create_started",
        EvidenceClass::ExecutedMutation,
        json!({"artifactId":artifact_id,"revision":revision,"title":request.workbook.title}),
    ) {
        let _ = repository::fail(persistence, &artifact_id, revision, "workbook_event_failed");
        return Err(command_error("workbook_event_failed", error));
    }
    if let Err(error) = build_revision_off_thread(
        persistence.clone(),
        identity.clone(),
        app.clone(),
        artifact_id.clone(),
        revision,
        request.workbook.clone(),
    )
    .await
    {
        let _ = repository::fail(persistence, &artifact_id, revision, &error);
        let _ = tasks::record_domain_event(
            persistence,
            &request.task_run_id,
            "workbook.create_failed",
            EvidenceClass::ObservedResult,
            json!({"artifactId":artifact_id,"revision":revision,"errorCode":"workbook_create_failed"}),
        );
        return Err(command_error("workbook_create_failed", error));
    }
    repository::get(persistence, &artifact_id)
        .map_err(|error| command_error("workbook_review_unavailable", error))
}

#[tauri::command]
pub async fn list_workbook_reviews(
    request: WorkbookListRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> CommandResult<Vec<WorkbookReviewSummary>> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = repository::reconcile_review_events(&engine);
        repository::list(&engine, request)
    })
    .await
    .map_err(|error| command_error("workbook_list_failed", error.to_string()))?
    .map_err(|error| command_error("workbook_list_failed", error))
}

#[tauri::command]
pub async fn get_workbook_review(
    request: WorkbookIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> CommandResult<WorkbookReviewRecord> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = repository::reconcile_review_events(&engine);
        repository::get(&engine, &request.artifact_id)
    })
    .await
    .map_err(|error| command_error("workbook_review_unavailable", error.to_string()))?
    .map_err(|error| command_error("workbook_review_unavailable", error))
}

#[tauri::command]
pub async fn get_workbook_preview(
    request: WorkbookPreviewRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    _app: tauri::AppHandle,
) -> CommandResult<WorkbookPreviewResponse> {
    let root = crate::settings::app_data_root().join("workbooks");
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        command_io::load_preview_response(&engine, &root, request)
    })
    .await
    .map_err(|error| command_error("workbook_preview_unavailable", error.to_string()))?
}

#[tauri::command]
pub async fn revise_workbook_range(
    request: ReviseWorkbookRangeRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    app: tauri::AppHandle,
) -> CommandResult<WorkbookReviewRecord> {
    let record = repository::get(persistence.inner(), &request.artifact_id)
        .map_err(|error| command_error("workbook_review_unavailable", error))?;
    if record.current_revision != request.base_revision {
        return Err(command_error(
            "workbook_revision_conflict",
            "Workbook changed; reload it before revising.",
        ));
    }
    let base = repository::load_ir(
        persistence.inner(),
        &request.artifact_id,
        request.base_revision,
    )
    .map_err(|error| command_error("workbook_revision_failed", error))?;
    let revision_request = WorkbookRangeRevision {
        sheet_id: request.sheet_id,
        target_range: request.target_range,
        instruction: request.instruction.clone(),
        replacement_cells: request.replacement_cells,
    };
    let mut revised = revise_range(&base, &revision_request)
        .map_err(|error| command_error(&revision_error_code(error.code), error.message))?;
    bind_workbook_provenance(
        persistence.inner(),
        &record.project_id,
        &record.task_id,
        &record.task_run_id,
        &mut revised,
    )
    .map_err(|error| command_error("workbook_evidence_invalid", error))?;
    let revision = repository::create_revision(
        persistence.inner(),
        &request.artifact_id,
        request.base_revision,
        &request.instruction,
        &revised,
    )
    .map_err(|error| command_error("workbook_revision_conflict", error))?;
    if let Err(error) = tasks::record_domain_event(
        persistence.inner(),
        &record.task_run_id,
        "workbook.revision_started",
        EvidenceClass::ExecutedMutation,
        json!({"artifactId":request.artifact_id,"revision":revision}),
    ) {
        let _ = repository::fail(
            persistence.inner(),
            &request.artifact_id,
            revision,
            "workbook_event_failed",
        );
        return Err(command_error("workbook_event_failed", error));
    }
    if let Err(error) = build_revision_off_thread(
        persistence.inner().clone(),
        identity.inner().clone(),
        app.clone(),
        request.artifact_id.clone(),
        revision,
        revised.clone(),
    )
    .await
    {
        let _ = repository::fail(persistence.inner(), &request.artifact_id, revision, &error);
        return Err(command_error("workbook_revision_failed", error));
    }
    repository::get(persistence.inner(), &request.artifact_id)
        .map_err(|error| command_error("workbook_review_unavailable", error))
}

#[tauri::command]
pub async fn export_workbook_revision(
    request: ExportWorkbookRevisionRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    app: tauri::AppHandle,
) -> CommandResult<ExportWorkbookRevisionResult> {
    let engine = persistence.inner().clone();
    let artifact_id = request.artifact_id.clone();
    let revision = request.revision;
    let files = tauri::async_runtime::spawn_blocking(move || {
        let _ = repository::reconcile_review_events(&engine);
        repository::revision_files(&engine, &artifact_id, revision)
    })
    .await
    .map_err(|error| command_error("workbook_export_unavailable", error.to_string()))?
    .map_err(|error| command_error("workbook_export_unavailable", error))?;
    if !files.verification.exportable {
        return Err(export_not_ready_error(
            &files.verification,
            "Workbook checks must pass and numbers must be up to date before export.",
        ));
    }
    let manifest_payload = serde_json::to_string(&files.manifest)
        .map_err(|error| command_error("workbook_export_unavailable", error.to_string()))?;
    identity
        .verify_payload(&manifest_payload, &files.signature)
        .map_err(|error| command_error("workbook_export_unavailable", error.message))?;
    let workbook_root = crate::settings::app_data_root().join("workbooks");
    let private_source = files.xlsx.clone();
    let private_sha256 = files.sha256.clone();
    tauri::async_runtime::spawn_blocking(move || {
        command_io::validate_private_source(&workbook_root, &private_source, &private_sha256)
    })
    .await
    .map_err(|error| command_error("workbook_export_unavailable", error.to_string()))??;
    let suggested = format!("{}-r{}.xlsx", safe_name(&files.title), request.revision);
    let Some(handle) = rfd::AsyncFileDialog::new()
        .set_file_name(&suggested)
        .save_file()
        .await
    else {
        return Err(command_error(
            "workbook_export_cancelled",
            "Workbook export was cancelled.",
        ));
    };
    let destination = handle.path().to_path_buf();
    approve_export(&app, approvals.inner(), &files, &destination).await?;
    let destination_hash = sha256_hex(destination.to_string_lossy().as_bytes());
    let receipt = repository::begin_export(persistence.inner(), &files, &destination_hash)
        .map_err(|error| command_error("workbook_export_failed", error))?;
    let source = files.xlsx.clone();
    let destination_for_copy = destination.clone();
    let expected = files.sha256.clone();
    let copy = tauri::async_runtime::spawn_blocking(move || {
        command_io::atomic_copy_verified(&source, &destination_for_copy, &expected)
    })
    .await
    .map_err(|error| command_error("workbook_export_failed", error.to_string()))?;
    let digest = match copy {
        Ok(digest) => digest,
        Err(error) => {
            let _ =
                repository::fail_export(persistence.inner(), &receipt, "workbook_export_failed");
            return Err(error);
        }
    };
    let accounting_status_code = match repository::complete_export(persistence.inner(), &receipt) {
        Ok(()) => {
            if let Err(error) = tasks::record_domain_event(
                persistence.inner(),
                &files.task_run_id,
                "workbook.exported",
                EvidenceClass::VerifiedPostcondition,
                json!({"artifactId":files.artifact_id,"revision":files.revision,"xlsxSha256":digest,"receiptId":receipt.export_id}),
            ) {
                eprintln!("WORKBOOK_EXPORT_EVENT_PENDING code=workbook_event_failed receipt_id={} error={}", receipt.export_id, error);
                WorkbookExportAccountingStatusCode::RecordingPending
            } else {
                WorkbookExportAccountingStatusCode::Recorded
            }
        }
        Err(error) => {
            eprintln!("WORKBOOK_EXPORT_RECEIPT_PENDING code=workbook_export_receipt_pending receipt_id={} error={}", receipt.export_id, error);
            WorkbookExportAccountingStatusCode::RecordingPending
        }
    };
    Ok(ExportWorkbookRevisionResult {
        artifact_id: files.artifact_id,
        revision: files.revision,
        path: destination.to_string_lossy().to_string(),
        sha256: digest,
        receipt_id: receipt.export_id,
        accounting_status_code,
    })
}

pub(crate) async fn export_workbook_revision_to_approved_path(
    artifact_id: &str,
    revision: u32,
    destination_path: &str,
    persistence: &PersistenceEngine,
    identity: &SovereignIdentity,
    _app: &tauri::AppHandle,
) -> CommandResult<ExportWorkbookRevisionResult> {
    let destination = crate::shield_gate::validate_approved_external_write_target(destination_path)
        .map_err(|error| command_error("workbook_export_failed", error.message))?;
    if destination
        .extension()
        .and_then(|value| value.to_str())
        .is_none_or(|value| !value.eq_ignore_ascii_case("xlsx"))
    {
        return Err(command_error(
            "workbook_export_failed",
            "The file name no longer matches the XLSX format.",
        ));
    }
    let engine = persistence.clone();
    let artifact_id_owned = artifact_id.to_string();
    let files = tauri::async_runtime::spawn_blocking(move || {
        let _ = repository::reconcile_review_events(&engine);
        repository::revision_files(&engine, &artifact_id_owned, revision)
    })
    .await
    .map_err(|error| command_error("workbook_export_unavailable", error.to_string()))?
    .map_err(|error| command_error("workbook_export_unavailable", error))?;
    if !files.verification.exportable {
        return Err(export_not_ready_error(
            &files.verification,
            "Workbook checks must pass before it can be saved.",
        ));
    }
    let manifest_payload = serde_json::to_string(&files.manifest)
        .map_err(|error| command_error("workbook_export_unavailable", error.to_string()))?;
    identity
        .verify_payload(&manifest_payload, &files.signature)
        .map_err(|error| command_error("workbook_export_unavailable", error.message))?;
    let workbook_root = crate::settings::app_data_root().join("workbooks");
    command_io::validate_private_source(&workbook_root, &files.xlsx, &files.sha256)?;
    let destination_hash = sha256_hex(destination.to_string_lossy().as_bytes());
    let receipt = repository::begin_export(persistence, &files, &destination_hash)
        .map_err(|error| command_error("workbook_export_failed", error))?;
    let digest = match command_io::atomic_copy_verified(&files.xlsx, &destination, &files.sha256) {
        Ok(digest) => digest,
        Err(error) => {
            let _ = repository::fail_export(persistence, &receipt, "workbook_export_failed");
            return Err(error);
        }
    };
    let accounting_status_code = match repository::complete_export(persistence, &receipt) {
        Ok(()) => WorkbookExportAccountingStatusCode::Recorded,
        Err(error) => {
            eprintln!(
                "WORKBOOK_EXPORT_RECEIPT_PENDING code=workbook_export_receipt_pending receipt_id={} error={}",
                receipt.export_id, error
            );
            WorkbookExportAccountingStatusCode::RecordingPending
        }
    };
    tasks::record_domain_event(
        persistence,
        &files.task_run_id,
        "workbook.exported",
        EvidenceClass::VerifiedPostcondition,
        json!({"artifactId":files.artifact_id,"revision":files.revision,"xlsxSha256":digest,"receiptId":receipt.export_id}),
    )
    .map_err(|error| command_error("workbook_event_failed", error))?;
    Ok(ExportWorkbookRevisionResult {
        artifact_id: files.artifact_id,
        revision: files.revision,
        path: destination.display().to_string(),
        sha256: digest,
        receipt_id: receipt.export_id,
        accounting_status_code,
    })
}

fn build_revision(
    engine: &PersistenceEngine,
    identity: &SovereignIdentity,
    _app: &tauri::AppHandle,
    artifact_id: &str,
    revision: u32,
    workbook: &WorkbookIr,
) -> Result<(), String> {
    let workbook_root = crate::settings::app_data_root().join("workbooks");
    let staging = workbook_root.join("staging");
    let artifact_root = staging.join(artifact_id);
    let root = artifact_root.join(format!("r{revision}"));
    if root.exists() {
        fs::remove_dir_all(&root).map_err(|error| error.to_string())?;
    }
    for directory in [&workbook_root, &staging, &artifact_root, &root] {
        command_io::create_private_directory(directory)?;
    }
    let mut cleanup = command_io::CleanupDirectory {
        path: root.clone(),
        committed: false,
    };
    let built = build_workbook(workbook)?;
    let xlsx = root.join("workbook.xlsx");
    command_io::atomic_write_bytes(&xlsx, &built.bytes)?;
    if sha256_file_hex(&xlsx).map_err(|error| error.to_string())? != built.sha256 {
        return Err("Private workbook digest verification failed.".to_string());
    }
    let preview_root = root.join("previews");
    command_io::create_private_directory(&preview_root)?;
    let mut stored_previews = Vec::new();
    for (index, preview) in built.previews.iter().enumerate() {
        let identity_hash = sha256_hex(preview.evidence.sheet_id.as_bytes());
        let path = preview_root.join(format!("{index:04}-{}.png", &identity_hash[..12]));
        command_io::atomic_write_bytes(&path, &preview.bytes)?;
        if sha256_file_hex(&path).map_err(|error| error.to_string())? != preview.evidence.sha256 {
            return Err("Private workbook preview digest verification failed.".to_string());
        }
        stored_previews.push(StoredWorkbookPreview {
            sheet_id: preview.evidence.sheet_id.clone(),
            path: path.to_string_lossy().to_string(),
            mime_type: preview.evidence.mime_type.clone(),
            width: preview.evidence.width,
            height: preview.evidence.height,
            sha256: preview.evidence.sha256.clone(),
        });
    }
    let ownership = repository::get(engine, artifact_id)?;
    let bound_evidence = resolve_workbook_evidence(
        engine,
        &ownership.project_id,
        &ownership.task_id,
        &ownership.task_run_id,
        &built.workbook,
    )?;
    let contract = artifact_workbook_contract(
        &ownership.project_id,
        &ownership.task_id,
        &ownership.task_run_id,
        artifact_id,
        &built.workbook,
        &bound_evidence,
    )?;
    let manifest = json!({"schemaVersion":1,"artifactId":artifact_id,"revision":revision,"title":built.workbook.title,"contract":contract,"xlsx":{"sha256":built.sha256,"bytes":built.bytes.len()},"previews":built.verification.previews,"verification":built.verification});
    let payload = serde_json::to_string(&manifest).map_err(|error| error.to_string())?;
    let signature = identity
        .sign_payload(&payload)
        .map_err(|error| error.message)?;
    identity
        .verify_payload(&payload, &signature)
        .map_err(|error| error.message)?;
    repository::complete(
        engine,
        repository::CompletedRevision {
            artifact_id,
            revision,
            workbook: &built.workbook,
            xlsx: &xlsx,
            previews: &stored_previews,
            verification: &built.verification,
            manifest: &manifest,
            signature: &signature,
            xlsx_sha256: &built.sha256,
            xlsx_bytes: built.bytes.len() as u64,
        },
    )?;
    cleanup.committed = true;
    let evidence = if built.verification.exportable {
        EvidenceClass::SignedArtifact
    } else {
        EvidenceClass::ObservedResult
    };
    if let Err(error) = tasks::record_domain_event(
        engine,
        &ownership.task_run_id,
        "workbook.review_ready",
        evidence,
        json!({"artifactId":artifact_id,"revision":revision,"statusCode":built.verification.status_code,"xlsxSha256":built.sha256,"exportable":built.verification.exportable,"manifestSignature":signature}),
    ) {
        let _ = repository::mark_review_event_pending(engine, artifact_id, revision);
        eprintln!(
            "WORKBOOK_POST_COMPLETION_EVENT_FAILED code=workbook_event_failed artifact_id={} revision={} error={}",
            artifact_id, revision, error
        );
    } else if let Err(error) = repository::mark_review_event_recorded(engine, artifact_id, revision)
    {
        eprintln!(
            "WORKBOOK_REVIEW_EVENT_MARK_PENDING code=workbook_event_marker_failed artifact_id={} revision={} error={}",
            artifact_id, revision, error
        );
    }
    Ok(())
}

async fn build_revision_off_thread(
    engine: PersistenceEngine,
    identity: SovereignIdentity,
    app: tauri::AppHandle,
    artifact_id: String,
    revision: u32,
    workbook: WorkbookIr,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        build_revision(&engine, &identity, &app, &artifact_id, revision, &workbook)
    })
    .await
    .map_err(|error| format!("Workbook build worker failed: {error}"))?
}

async fn approve_export(
    app: &tauri::AppHandle,
    approvals: &ShieldApprovalManager,
    files: &repository::RevisionFiles,
    destination: &Path,
) -> CommandResult<()> {
    request_user_approval(
        app,
        approvals,
        ShieldApprovalRequest {
            approval_token: format!("approval_{}", hex::encode(random_bytes())),
            session_id: Some(files.artifact_id.clone()),
            turn_id: Some(files.task_run_id.clone()),
            generation_token: None,
            action_type: "workbook_export".into(),
            action_label: "workbook_export_action".into(),
            target_path: Some(destination.to_string_lossy().to_string()),
            principal: Some(files.project_id.clone()),
            risk_tier: "consequential".into(),
            reason: "workbook_export_reason".into(),
            estimated_token_costs: None,
            requested_at_ms: crate::foundation::clock::unix_time_ms_u64(),
            preview: String::new(),
            semantic_summary: "workbook_export_title".into(),
            semantic_detail: "workbook_export_detail".into(),
            approval_tier: "effectful".into(),
            approval_mode: "single_exact_destination".into(),
            diff_preview: None,
            scope_trust_available: false,
            scope_trust_prefix: None,
            scope_trust_duration_ms: 0,
            project_id: Some(files.project_id.clone()),
            task_run_id: Some(files.task_run_id.clone()),
            action_class: "workbook_export".into(),
            argument_class: crate::approval_scopes::argument_class("workbook_export", "xlsx"),
            canonical_resource: Some(destination.to_string_lossy().to_string()),
            mandatory_reconfirm: true,
            approval_scope_kinds: vec!["once".into()],
        },
    )
    .await
    .map_err(|error| command_error("workbook_export_not_approved", error.message))
}

fn revision_error_code(code: WorkbookRevisionErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "workbook_revision_failed".to_string())
}
fn command_error(code: &str, message: impl Into<String>) -> WorkbookCommandError {
    WorkbookCommandError::new(code, message)
}
fn export_not_ready_error(
    verification: &WorkbookVerification,
    message: impl Into<String>,
) -> WorkbookCommandError {
    let failed_evidence = verification
        .evidence
        .iter()
        .filter(|check| !check.passed)
        .collect::<Vec<_>>();
    eprintln!(
        "WORKBOOK_EXPORT_NOT_READY code=workbook_export_not_ready detail={}",
        json!({
            "statusCode": verification.status_code,
            "renderer": verification.renderer,
            "exactPackagePageCount": verification.exact_package_page_count,
            "warnings": verification.warnings,
            "failedEvidence": failed_evidence,
        })
    );
    command_error("workbook_export_not_ready", message)
}
fn random_bytes() -> [u8; 18] {
    use rand_core::{OsRng, RngCore};
    let mut bytes = [0_u8; 18];
    OsRng.fill_bytes(&mut bytes);
    bytes
}
fn safe_name(value: &str) -> String {
    let result = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(80)
        .collect::<String>();
    if result.trim_matches('_').is_empty() {
        "oomu-spreadsheet".into()
    } else {
        result
    }
}
