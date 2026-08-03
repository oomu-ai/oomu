use super::checker_setup::presentation_checker_readiness;
use super::exact_package_preview::{presentation_checker_download_url, render_exact_package};
use super::service::{
    build_presentation_revision_off_thread, create_private_directory, write_private_file,
};
use super::*;
use crate::{
    db::PersistenceEngine, p0_contracts::EvidenceClass, sovereign_identity::SovereignIdentity,
    tasks,
};
use rusqlite::params;
use serde_json::json;
use std::{fs, io::Read, path::Path};
use tauri_plugin_shell::ShellExt;

type CommandResult<T> = Result<T, PresentationCommandError>;

#[tauri::command]
pub async fn create_presentation(
    request: CreatePresentationRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    app: tauri::AppHandle,
) -> CommandResult<PresentationReviewDetail> {
    create_presentation_internal(request, persistence.inner(), identity.inner(), &app).await
}

pub(crate) async fn create_presentation_internal(
    mut request: CreatePresentationRequest,
    persistence: &PersistenceEngine,
    identity: &SovereignIdentity,
    app: &tauri::AppHandle,
) -> CommandResult<PresentationReviewDetail> {
    if request.presentation.revision != 1 || request.title != request.presentation.title {
        return Err(error(
            "presentation_create_invalid",
            "New presentations must begin at revision 1 with one consistent title.",
        ));
    }
    let task = tasks::require_bound_task(persistence, &request.task_run_id, &request.project_id)
        .map_err(|value| error("presentation_context_invalid", value))?;
    if task.task_id != request.task_id {
        return Err(error(
            "presentation_context_invalid",
            "Task ID does not match the bound Task run.",
        ));
    }
    request.presentation = apply_presentation_policies(&request.presentation)
        .map_err(|value| error("presentation_create_invalid", value))?
        .presentation;
    let evidence = bind_presentation_provenance(
        persistence,
        &request.project_id,
        &request.task_id,
        &request.task_run_id,
        &mut request.presentation,
    )
    .map_err(|value| error("presentation_evidence_invalid", value))?;
    let (presentation_id, revision) = create_presentation_record(persistence, &request, &evidence)
        .map_err(|value| error("presentation_create_failed", value))?;
    if let Err(value) = tasks::record_domain_event(
        persistence,
        &request.task_run_id,
        "presentation.create_started",
        EvidenceClass::ExecutedMutation,
        json!({"presentationId":presentation_id,"revision":revision,"title":request.title}),
    ) {
        let _ = fail_presentation_revision(persistence, &presentation_id, revision, &value);
        return Err(error("presentation_event_failed", value));
    }
    if let Err(value) = build_presentation_revision_off_thread(
        persistence.clone(),
        identity.clone(),
        app.clone(),
        presentation_id.clone(),
        revision,
        request.presentation,
    )
    .await
    {
        let _ = fail_presentation_revision(persistence, &presentation_id, revision, &value);
        return Err(error("presentation_create_failed", value));
    }
    get_presentation_record(persistence, &presentation_id, None)
        .map_err(|value| error("presentation_review_unavailable", value))
}

#[tauri::command]
pub async fn list_presentation_reviews(
    request: PresentationListRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> CommandResult<Vec<PresentationReviewSummary>> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || list_presentation_records(&engine, request))
        .await
        .map_err(|value| error("presentation_list_failed", value.to_string()))?
        .map_err(|value| error("presentation_list_failed", value))
}

#[tauri::command]
pub async fn get_presentation_review(
    request: GetPresentationReviewRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> CommandResult<PresentationReviewDetail> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        get_presentation_record(&engine, &request.presentation_id, request.revision)
    })
    .await
    .map_err(|value| error("presentation_review_unavailable", value.to_string()))?
    .map_err(|value| error("presentation_review_unavailable", value))
}

#[tauri::command]
pub async fn get_presentation_preview(
    request: GetPresentationPreviewRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    _app: tauri::AppHandle,
) -> CommandResult<PresentationPreviewResponse> {
    let root = crate::settings::app_data_root()
        .join("presentations")
        .join("private");
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || load_presentation_preview(&engine, &root, request))
        .await
        .map_err(|value| error("presentation_preview_unavailable", value.to_string()))?
}

#[tauri::command]
pub async fn get_presentation_checker_readiness() -> CommandResult<PresentationCheckerReadiness> {
    tauri::async_runtime::spawn_blocking(presentation_checker_readiness)
        .await
        .map_err(|value| error("presentation_checker_probe_failed", value.to_string()))
}

#[tauri::command]
#[allow(deprecated)]
pub fn open_presentation_checker_download(app: tauri::AppHandle) -> CommandResult<()> {
    app.shell()
        .open(presentation_checker_download_url(), None)
        .map_err(|value| error("presentation_checker_download_failed", value.to_string()))
}

#[tauri::command]
pub async fn recheck_presentation_revision(
    request: RecheckPresentationRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    app: tauri::AppHandle,
) -> CommandResult<PresentationReviewDetail> {
    let current = get_presentation_record(persistence.inner(), &request.presentation_id, None)
        .map_err(|value| error("presentation_review_unavailable", value))?;
    if current.summary.current_revision != request.expected_revision
        || current.selected_revision != request.expected_revision
    {
        return Err(error(
            "presentation_revision_conflict",
            "Presentation changed; reload it before checking again.",
        ));
    }
    if !current
        .issues
        .iter()
        .any(|issue| issue.code == "exact_package_preview_unavailable")
    {
        return Err(error(
            "presentation_recheck_not_needed",
            "This presentation does not need the presentation checker setup repair.",
        ));
    }
    if presentation_checker_readiness().status != PresentationCheckerStatus::Ready {
        return Err(error(
            "presentation_checker_not_ready",
            "The presentation checker is not ready on this Mac.",
        ));
    }

    let files = presentation_revision_files(
        persistence.inner(),
        &request.presentation_id,
        request.expected_revision,
    )
    .map_err(|value| error("presentation_recheck_failed", value))?;
    let private_root = crate::settings::app_data_root()
        .join("presentations")
        .join("private");
    let canonical_root = fs::canonicalize(private_root)
        .map_err(|value| error("presentation_recheck_failed", value.to_string()))?;
    let metadata = fs::symlink_metadata(&files.pptx)
        .map_err(|value| error("presentation_recheck_failed", value.to_string()))?;
    let canonical_package = fs::canonicalize(&files.pptx)
        .map_err(|value| error("presentation_recheck_failed", value.to_string()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > 128 * 1024 * 1024
        || !canonical_package.starts_with(&canonical_root)
    {
        return Err(error(
            "presentation_recheck_failed",
            "The saved presentation failed private-file checks.",
        ));
    }
    let package = fs::read(canonical_package)
        .map_err(|value| error("presentation_recheck_failed", value.to_string()))?;
    if super::ooxml::hex_digest(&package) != files.sha256 {
        return Err(error(
            "presentation_recheck_failed",
            "The saved presentation digest changed.",
        ));
    }
    let slide_ids = current
        .presentation
        .slides
        .iter()
        .map(|slide| slide.slide_id.clone())
        .collect::<Vec<_>>();
    tauri::async_runtime::spawn_blocking(move || render_exact_package(&package, &slide_ids))
        .await
        .map_err(|value| error("presentation_recheck_failed", value.to_string()))?
        .map_err(|value| error("presentation_recheck_failed", value))?;

    let mut presentation = current.presentation;
    presentation.revision = request.expected_revision.checked_add(1).ok_or_else(|| {
        error(
            "presentation_recheck_failed",
            "Presentation revision limit reached.",
        )
    })?;
    let evidence = bind_presentation_provenance(
        persistence.inner(),
        &current.summary.project_id,
        &current.summary.task_id,
        &current.summary.task_run_id,
        &mut presentation,
    )
    .map_err(|value| error("presentation_evidence_invalid", value))?;
    let revision = create_presentation_revision(
        persistence.inner(),
        &request.presentation_id,
        request.expected_revision,
        PresentationRevisionScope::WholePresentation,
        "Presentation checks rerun",
        &presentation,
        &evidence,
    )
    .map_err(|value| error("presentation_revision_conflict", value))?;
    tasks::record_domain_event(
        persistence.inner(),
        &current.summary.task_run_id,
        "presentation.recheck_started",
        EvidenceClass::ExecutedMutation,
        json!({"presentationId":request.presentation_id,"revision":revision}),
    )
    .map_err(|value| error("presentation_event_failed", value))?;
    if let Err(value) = build_presentation_revision_off_thread(
        persistence.inner().clone(),
        identity.inner().clone(),
        app,
        request.presentation_id.clone(),
        revision,
        presentation,
    )
    .await
    {
        let _ = fail_presentation_revision(
            persistence.inner(),
            &request.presentation_id,
            revision,
            &value,
        );
        return Err(error("presentation_recheck_failed", value));
    }
    get_presentation_record(persistence.inner(), &request.presentation_id, None)
        .map_err(|value| error("presentation_review_unavailable", value))
}

#[tauri::command]
pub async fn revise_presentation_scope(
    request: RevisePresentationScopeRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    app: tauri::AppHandle,
) -> CommandResult<PresentationReviewDetail> {
    let current = get_presentation_record(persistence.inner(), &request.presentation_id, None)
        .map_err(|value| error("presentation_review_unavailable", value))?;
    if current.summary.current_revision != request.expected_revision {
        return Err(error(
            "presentation_revision_conflict",
            "Presentation changed; reload it before revising.",
        ));
    }
    let base = load_presentation_ir(
        persistence.inner(),
        &request.presentation_id,
        request.expected_revision,
    )
    .map_err(|value| error("presentation_revision_failed", value))?;
    let scope = request.scope;
    let summary = request.change_summary.clone();
    let mut revised = revise_presentation_scope_ir(&base, &request)
        .map_err(|value| error("presentation_revision_invalid", value))?;
    let evidence = bind_presentation_provenance(
        persistence.inner(),
        &current.summary.project_id,
        &current.summary.task_id,
        &current.summary.task_run_id,
        &mut revised,
    )
    .map_err(|value| error("presentation_evidence_invalid", value))?;
    let revision = create_presentation_revision(
        persistence.inner(),
        &request.presentation_id,
        request.expected_revision,
        scope,
        &summary,
        &revised,
        &evidence,
    )
    .map_err(|value| error("presentation_revision_conflict", value))?;
    tasks::record_domain_event(
        persistence.inner(),
        &current.summary.task_run_id,
        "presentation.revision_started",
        EvidenceClass::ExecutedMutation,
        json!({"presentationId":request.presentation_id,"revision":revision,"scope":scope}),
    )
    .map_err(|value| error("presentation_event_failed", value))?;
    if let Err(value) = build_presentation_revision_off_thread(
        persistence.inner().clone(),
        identity.inner().clone(),
        app,
        request.presentation_id.clone(),
        revision,
        revised,
    )
    .await
    {
        let _ = fail_presentation_revision(
            persistence.inner(),
            &request.presentation_id,
            revision,
            &value,
        );
        return Err(error("presentation_revision_failed", value));
    }
    get_presentation_record(persistence.inner(), &request.presentation_id, None)
        .map_err(|value| error("presentation_review_unavailable", value))
}

#[tauri::command]
pub async fn inspect_presentation_template(
    request: InspectPresentationTemplateRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    _app: tauri::AppHandle,
) -> CommandResult<Option<RegisteredPresentationTemplate>> {
    let Some(handle) = rfd::AsyncFileDialog::new()
        .add_filter("PPTX", &["pptx"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let source = handle.path().to_path_buf();
    let app_data = crate::settings::app_data_root();
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        inspect_and_register_template(&engine, &app_data, &source, request)
    })
    .await
    .map_err(|value| error("presentation_template_failed", value.to_string()))?
    .map(Some)
}

fn inspect_and_register_template(
    engine: &PersistenceEngine,
    app_data: &Path,
    source: &Path,
    request: InspectPresentationTemplateRequest,
) -> CommandResult<RegisteredPresentationTemplate> {
    let task = tasks::require_bound_task(engine, &request.task_run_id, &request.project_id)
        .map_err(|value| error("presentation_template_context_invalid", value))?;
    if task.task_id != request.task_id {
        return Err(error(
            "presentation_template_context_invalid",
            "Task ID does not match the bound Task run.",
        ));
    }
    let path_metadata = fs::symlink_metadata(source)
        .map_err(|value| error("presentation_template_invalid", value.to_string()))?;
    if path_metadata.file_type().is_symlink() {
        return Err(error(
            "presentation_template_invalid",
            "Template source failed file identity or size checks.",
        ));
    }
    #[cfg(unix)]
    let source_file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(source)
            .map_err(|value| error("presentation_template_invalid", value.to_string()))?
    };
    #[cfg(not(unix))]
    let source_file = fs::File::open(source)
        .map_err(|value| error("presentation_template_invalid", value.to_string()))?;
    let metadata = source_file
        .metadata()
        .map_err(|value| error("presentation_template_invalid", value.to_string()))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 128 * 1024 * 1024 {
        return Err(error(
            "presentation_template_invalid",
            "Template source failed file identity or size checks.",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    source_file
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|value| error("presentation_template_invalid", value.to_string()))?;
    if bytes.len() as u64 != metadata.len() {
        return Err(error(
            "presentation_template_invalid",
            "Template source changed while it was inspected.",
        ));
    }
    let inspection = inspect_presentation_template_bytes(&bytes)
        .map_err(|value| error("presentation_template_invalid", value))?;
    let existing_count: i64 = engine
        .open_connection()
        .map_err(|value| error("presentation_template_failed", value.to_string()))?
        .query_row(
            "SELECT COUNT(*) FROM presentation_template_imports WHERE project_id=?1",
            params![request.project_id],
            |row| row.get(0),
        )
        .map_err(|value| error("presentation_template_failed", value.to_string()))?;
    if existing_count >= 100 {
        return Err(error(
            "presentation_template_limit_reached",
            "This Project has reached the registered presentation template limit.",
        ));
    }
    let template_id = format!("presentation-template-{}", hex::encode(random_bytes()));
    let root = app_data.join("presentations").join("templates");
    create_private_directory(&root)
        .map_err(|value| error("presentation_template_failed", value))?;
    let destination = root.join(format!("{template_id}.pptx"));
    write_private_file(&destination, &bytes)
        .map_err(|value| error("presentation_template_failed", value))?;
    let name = source
        .file_stem()
        .map(|value| value.to_string_lossy().chars().take(255).collect())
        .unwrap_or_else(|| "Imported presentation".to_string());
    let inspection_json = serde_json::to_string(&inspection)
        .map_err(|value| error("presentation_template_failed", value.to_string()))?;
    let inserted = engine.open_connection().map_err(|value| error("presentation_template_failed",value.to_string()))?.execute(
        "INSERT INTO presentation_template_imports(template_id,project_id,task_id,task_run_id,fingerprint_sha256,private_path,inspection_json,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![template_id,request.project_id,request.task_id,request.task_run_id,inspection.fingerprint_sha256,destination.to_string_lossy(),inspection_json,crate::foundation::clock::unix_time_ms_i64()],
    );
    if let Err(value) = inserted {
        let _ = fs::remove_file(destination);
        return Err(error("presentation_template_failed", value.to_string()));
    }
    tasks::record_domain_event(engine,&request.task_run_id,"presentation.template_registered",EvidenceClass::VerifiedPostcondition,json!({"templateId":template_id,"fingerprintSha256":inspection.fingerprint_sha256,"masterCount":inspection.master_parts.len(),"layoutCount":inspection.layout_parts.len(),"slideCount":inspection.slide_parts.len()})).map_err(|value|error("presentation_event_failed",value))?;
    Ok(RegisteredPresentationTemplate {
        template_id,
        name,
        fingerprint_sha256: inspection.fingerprint_sha256,
        master_parts: inspection.master_parts,
        layout_parts: inspection.layout_parts,
        slide_count: inspection.slide_parts.len(),
        exact_part_preservation_supported: inspection.exact_part_preservation_supported,
        task_summary_compatible: inspection.task_summary_compatible,
    })
}

fn error(code: &str, message: impl Into<String>) -> PresentationCommandError {
    PresentationCommandError::new(code, message)
}

fn random_bytes() -> [u8; 18] {
    use rand_core::{OsRng, RngCore};
    let mut bytes = [0_u8; 18];
    OsRng.fill_bytes(&mut bytes);
    bytes
}
