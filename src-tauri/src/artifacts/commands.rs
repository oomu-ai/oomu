use super::*;
use crate::{
    db::PersistenceEngine,
    foundation::digest::{sha256_file_hex, sha256_hex},
    p0_contracts::EvidenceClass,
    shield_gate::{request_user_approval, ShieldApprovalManager, ShieldApprovalRequest},
    sovereign_identity::SovereignIdentity,
    tasks,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use serde_json::json;
use std::{collections::HashMap, fs, io::Write, path::Path};

#[tauri::command]
pub async fn create_artifact(
    request: CreateArtifactRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<ArtifactRecord, String> {
    create_artifact_internal(request, persistence.inner(), identity.inner()).await
}

pub(crate) async fn create_artifact_internal(
    request: CreateArtifactRequest,
    persistence: &PersistenceEngine,
    identity: &SovereignIdentity,
) -> Result<ArtifactRecord, String> {
    tasks::require_bound_task(persistence, &request.task_run_id, &request.project_id)?;
    validation::validate(&request.document)?;
    let (artifact_id, version) = repository::create_record(
        persistence,
        &request.project_id,
        &request.task_run_id,
        &request.document,
    )?;
    tasks::record_domain_event(
        persistence,
        &request.task_run_id,
        "artifact.build_started",
        EvidenceClass::ExecutedMutation,
        json!({"artifactId":artifact_id,"version":version,"title":request.document.metadata.title}),
    )?;
    let engine = persistence.clone();
    let identity = identity.clone();
    let id = artifact_id.clone();
    let document = request.document;
    let result = tauri::async_runtime::spawn_blocking(move || {
        build_version(&engine, &identity, &id, version, &document)
    })
    .await
    .map_err(|error| error.to_string())?;
    if let Err(error) = result {
        repository::fail(persistence, &artifact_id, version, &error)?;
        tasks::record_domain_event(
            persistence,
            &request.task_run_id,
            "artifact.build_failed",
            EvidenceClass::ObservedResult,
            json!({"artifactId":artifact_id,"version":version,"error":error}),
        )?;
        return Err(error);
    }
    repository::get(persistence, &artifact_id)
}

pub(crate) fn export_verified_artifact_to_approved_path(
    artifact_id: &str,
    version: u32,
    format: &str,
    destination_path: &str,
    persistence: &PersistenceEngine,
    identity: &SovereignIdentity,
) -> Result<ArtifactExportResult, String> {
    if !matches!(format, "docx" | "pdf") {
        return Err("Document format is not available for direct export.".to_string());
    }
    let destination = crate::shield_gate::validate_approved_external_write_target(destination_path)
        .map_err(|error| error.message)?;
    if destination
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
        != Some(format)
    {
        return Err("The file name no longer matches the requested document format.".to_string());
    }

    let files = repository::version_paths(persistence, artifact_id, version)?;
    let payload = serde_json::to_string(&files.manifest).map_err(|error| error.to_string())?;
    identity
        .verify_payload(&payload, &files.signature)
        .map_err(|error| error.message)?;
    let (source, expected) = if format == "pdf" {
        (&files.pdf, &files.pdf_sha256)
    } else {
        (&files.docx, &files.docx_sha256)
    };
    let mut exported = Vec::new();
    let mut hashes = HashMap::new();
    copy_verified(source, &destination, expected, &mut exported, &mut hashes)?;
    let destination_hash = sha256_hex(destination.to_string_lossy().as_bytes());
    repository::record_export(
        persistence,
        artifact_id,
        version,
        format,
        &destination_hash,
        &hashes,
    )?;
    tasks::record_domain_event(
        persistence,
        &files.task_run_id,
        "artifact.exported",
        EvidenceClass::VerifiedPostcondition,
        json!({"artifactId":artifact_id,"version":version,"format":format,"hashes":hashes}),
    )?;
    Ok(ArtifactExportResult {
        exported_files: exported,
        hashes,
    })
}

#[tauri::command]
pub async fn create_decision_brief_from_delegation(
    request: CreateDecisionBriefRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<ArtifactRecord, String> {
    let plan = crate::delegation::load_plan(persistence.inner(), &request.delegation_plan_id)?;
    if !matches!(plan.state.as_str(), "completed" | "partial") {
        return Err(
            "Decision brief generation requires a completed or explicitly partial delegation plan."
                .to_string(),
        );
    }
    let title = request.title.trim();
    if title.is_empty() || title.chars().count() > 240 {
        return Err("Decision brief title is invalid.".to_string());
    }
    let mut sections = Vec::new();
    for child in &plan.children {
        let Some(result) = child.result.as_ref() else {
            continue;
        };
        let mut blocks = Vec::new();
        for finding in &result.findings {
            let sources = finding
                .source_refs
                .iter()
                .filter_map(|source_ref| {
                    result
                        .sources
                        .iter()
                        .find(|source| &source.source_ref == source_ref)
                        .map(|source| ArtifactSourceReference {
                            source_ref: source.source_ref.clone(),
                            evidence_ref: source.digest.clone(),
                            url: None,
                        })
                })
                .collect::<Vec<_>>();
            blocks.push(ArtifactBlock::Paragraph {
                text: finding.statement.clone(),
                style: ParagraphStyle::Body,
                factual: true,
                sources,
            });
        }
        if !result.uncertainties.is_empty() {
            blocks.push(ArtifactBlock::Callout {
                label: "Uncertainties".to_string(),
                text: result.uncertainties.join("\n"),
                factual: false,
                sources: Vec::new(),
            });
        }
        if !result.complete {
            blocks.push(ArtifactBlock::Callout { label: "Incomplete evidence".to_string(), text: "This contribution was incomplete and is not presented as a verified conclusion.".to_string(), factual: false, sources: Vec::new() });
        }
        if !blocks.is_empty() {
            sections.push(ArtifactSection {
                heading: child.goal.clone(),
                page_break_before: !sections.is_empty(),
                blocks,
            });
        }
    }
    if sections.is_empty() {
        return Err("Delegation plan has no schema-valid findings to synthesize.".to_string());
    }
    let document = ArtifactDocument {
        schema_version: ARTIFACT_DOCUMENT_SCHEMA_VERSION,
        metadata: ArtifactMetadata {
            title: title.to_string(),
            subtitle: request.subtitle.trim().chars().take(500).collect(),
            author: "OOMU parent workflow".to_string(),
            subject: "Weekly decision brief".to_string(),
            keywords: vec![
                "decision brief".to_string(),
                "delegated research".to_string(),
            ],
            language: "en".to_string(),
        },
        theme: ThemeTokens::default(),
        page: PageControls::default(),
        header: Some(title.to_string()),
        footer: Some("Evidence-bound decision brief".to_string()),
        sections,
    };
    tasks::record_domain_event(
        persistence.inner(),
        &plan.task_run_id,
        "hero.parent_artifact_requested",
        EvidenceClass::ExecutedMutation,
        json!({"planId":plan.plan_id,"parentOwned":true,"childMutationAuthority":false}),
    )?;
    create_artifact_internal(
        CreateArtifactRequest {
            project_id: plan.project_id,
            task_run_id: plan.task_run_id,
            document,
        },
        persistence.inner(),
        identity.inner(),
    )
    .await
}

#[tauri::command]
pub async fn revise_artifact(
    request: ReviseArtifactRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<ArtifactRecord, String> {
    tasks::require_bound_task(
        persistence.inner(),
        &request.task_run_id,
        &request.project_id,
    )?;
    validation::validate(&request.document)?;
    let version = repository::create_revision(
        persistence.inner(),
        &request.artifact_id,
        &request.project_id,
        &request.task_run_id,
        &request.instruction,
        &request.document,
    )?;
    tasks::record_domain_event(
        persistence.inner(),
        &request.task_run_id,
        "artifact.revision_started",
        EvidenceClass::ExecutedMutation,
        json!({"artifactId":request.artifact_id,"version":version,"instruction":request.instruction.chars().take(500).collect::<String>()}),
    )?;
    let engine = persistence.inner().clone();
    let identity = identity.inner().clone();
    let id = request.artifact_id.clone();
    let document = request.document;
    let result = tauri::async_runtime::spawn_blocking(move || {
        build_version(&engine, &identity, &id, version, &document)
    })
    .await
    .map_err(|error| error.to_string())?;
    if let Err(error) = result {
        repository::fail(persistence.inner(), &request.artifact_id, version, &error)?;
        tasks::record_domain_event(
            persistence.inner(),
            &request.task_run_id,
            "artifact.revision_failed",
            EvidenceClass::ObservedResult,
            json!({"artifactId":request.artifact_id,"version":version,"error":error}),
        )?;
        return Err(error);
    }
    repository::get(persistence.inner(), &request.artifact_id)
}

fn build_version(
    engine: &PersistenceEngine,
    identity: &SovereignIdentity,
    artifact_id: &str,
    version: u32,
    document: &ArtifactDocument,
) -> Result<(), String> {
    let root = crate::settings::app_data_root()
        .join("artifacts")
        .join("staging")
        .join(artifact_id)
        .join(format!("v{version}"));
    if root.exists() {
        fs::remove_dir_all(&root)
            .map_err(|error| format!("Unable to reset private artifact staging: {error}"))?;
    }
    let built = runtime::build_contained(document, &root)?;
    let pdf_builder_digest =
        runtime::rebuild_pdf_with_packaged_renderer(document, &built.pdf_path, &root)?;
    repository::mark_verifying(engine, artifact_id, version)?;
    let render_dir = root.join("rendered-pages");
    let (verification, previews) =
        verifier::verify_all(document, &built.docx_path, &built.pdf_path, &render_dir)?;
    if !verification.structurally_verified_docx
        || !verification.structurally_verified_pdf
        || !verification.visually_verified_pdf
    {
        return Err("Artifact verification did not satisfy every required gate.".to_string());
    }
    let docx_sha256 = sha256_file_hex(&built.docx_path).map_err(|error| error.to_string())?;
    let pdf_sha256 = sha256_file_hex(&built.pdf_path).map_err(|error| error.to_string())?;
    let docx_bytes = fs::metadata(&built.docx_path)
        .map_err(|error| error.to_string())?
        .len();
    let pdf_bytes = fs::metadata(&built.pdf_path)
        .map_err(|error| error.to_string())?
        .len();
    let sources=document.sections.iter().flat_map(|section|section.blocks.iter()).flat_map(|block|block.sources().iter()).map(|source|json!({"sourceRef":source.source_ref,"evidenceRef":source.evidence_ref,"url":source.url})).collect::<Vec<_>>();
    let provenance = json!({"schemaVersion":1,"sourceTask":repository::get(engine,artifact_id)?.task_run_id,"sources":sources,"builderHelperSha256":built.helper_digest,"pdfBuilderHelperSha256":pdf_builder_digest});
    let manifest = json!({"schemaVersion":1,"artifactId":artifact_id,"version":version,"title":document.metadata.title,"docx":{"sha256":docx_sha256,"bytes":docx_bytes},"pdf":{"sha256":pdf_sha256,"bytes":pdf_bytes,"pages":verification.page_count},"builderIdentity":ARTIFACT_BUILDER_IDENTITY,"rendererIdentity":ARTIFACT_RENDERER_IDENTITY,"verification":verification,"provenance":provenance});
    let manifest_payload = serde_json::to_string(&manifest).map_err(|error| error.to_string())?;
    let signature = identity
        .sign_payload(&manifest_payload)
        .map_err(|error| error.message)?;
    identity
        .verify_payload(&manifest_payload, &signature)
        .map_err(|error| error.message)?;
    repository::complete(
        engine,
        repository::CompletedVersion {
            artifact_id,
            version,
            docx: &built.docx_path,
            pdf: &built.pdf_path,
            previews: &previews,
            verification: &verification,
            provenance: &provenance,
            manifest: &manifest,
            signature: &signature,
            docx_sha256: &docx_sha256,
            pdf_sha256: &pdf_sha256,
            docx_bytes,
            pdf_bytes,
        },
    )?;
    let record = repository::get(engine, artifact_id)?;
    tasks::record_domain_event(
        engine,
        &record.task_run_id,
        "artifact.verified",
        EvidenceClass::SignedArtifact,
        json!({"artifactId":artifact_id,"version":version,"docxSha256":docx_sha256,"pdfSha256":pdf_sha256,"pageCount":verification.page_count,"manifestSignature":signature}),
    )?;
    Ok(())
}

#[tauri::command]
pub async fn list_artifacts(
    request: ArtifactListRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<ArtifactRecord>, String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || repository::list(&engine, request))
        .await
        .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn get_artifact(
    request: ArtifactIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ArtifactRecord, String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || repository::get(&engine, &request.artifact_id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn get_artifact_preview_page(
    request: ArtifactPreviewRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    _app: tauri::AppHandle,
) -> Result<String, String> {
    let path = repository::preview_path(
        persistence.inner(),
        &request.artifact_id,
        request.version,
        request.page,
    )?;
    let root = crate::settings::app_data_root().join("artifacts");
    let canonical_root = fs::canonicalize(root)
        .map_err(|_| "Private artifact staging is unavailable.".to_string())?;
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| "Artifact preview is unavailable.".to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > 16 * 1024 * 1024
    {
        return Err("Artifact preview failed validation.".to_string());
    }
    let canonical = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !canonical.starts_with(canonical_root) {
        return Err("Artifact preview escaped private staging.".to_string());
    }
    let bytes = fs::read(canonical).map_err(|error| error.to_string())?;
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
}

#[tauri::command]
pub async fn choose_artifact_export_destination(
    request: ChooseArtifactExportRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    runtime: tauri::State<'_, ArtifactRuntimeManager>,
) -> Result<Option<ArtifactExportGrantView>, String> {
    repository::version_paths(persistence.inner(), &request.artifact_id, request.version)?;
    let Some(handle) = rfd::AsyncFileDialog::new().pick_folder().await else {
        return Ok(None);
    };
    let path = fs::canonicalize(handle.path())
        .map_err(|_| "Selected export folder is unavailable.".to_string())?;
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Selected export destination must be a regular folder.".to_string());
    }
    let grant_id = format!("artifact_export_{}", hex::encode(random_bytes()));
    let expires = crate::foundation::clock::unix_time_ms_i64() + 5 * 60 * 1000;
    runtime
        .grants
        .lock()
        .map_err(|_| "Artifact export grant store is unavailable.".to_string())?
        .insert(
            grant_id.clone(),
            ExportGrant {
                artifact_id: request.artifact_id,
                version: request.version,
                path: path.clone(),
                expires_at_ms: expires,
            },
        );
    Ok(Some(ArtifactExportGrantView {
        export_grant_id: grant_id,
        directory_name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("selected folder")
            .to_string(),
        expires_at_ms: expires,
    }))
}

#[tauri::command]
pub async fn export_artifact(
    request: ExportArtifactRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    runtime: tauri::State<'_, ArtifactRuntimeManager>,
    identity: tauri::State<'_, SovereignIdentity>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    app: tauri::AppHandle,
) -> Result<ArtifactExportResult, String> {
    if !matches!(request.format.as_str(), "docx" | "pdf" | "both") {
        return Err("Artifact export format is invalid.".to_string());
    }
    let grant = runtime
        .grants
        .lock()
        .map_err(|_| "Artifact export grant store is unavailable.".to_string())?
        .remove(&request.export_grant_id)
        .ok_or_else(|| "Artifact export grant is missing, expired, or consumed.".to_string())?;
    if grant.artifact_id != request.artifact_id
        || grant.version != request.version
        || grant.expires_at_ms < crate::foundation::clock::unix_time_ms_i64()
    {
        return Err("Artifact export grant scope is invalid or expired.".to_string());
    }
    let files =
        repository::version_paths(persistence.inner(), &request.artifact_id, request.version)?;
    let payload = serde_json::to_string(&files.manifest).map_err(|error| error.to_string())?;
    identity
        .verify_payload(&payload, &files.signature)
        .map_err(|error| error.message)?;
    approve_export(
        &app,
        approvals.inner(),
        &files,
        &grant.path,
        &request.format,
    )
    .await?;
    let stem = safe_name(&files.title);
    let mut exported = Vec::new();
    let mut hashes = HashMap::new();
    if matches!(request.format.as_str(), "docx" | "both") {
        copy_verified(
            &files.docx,
            &grant.path.join(format!("{stem}-v{}.docx", request.version)),
            &files.docx_sha256,
            &mut exported,
            &mut hashes,
        )?;
    }
    if matches!(request.format.as_str(), "pdf" | "both") {
        copy_verified(
            &files.pdf,
            &grant.path.join(format!("{stem}-v{}.pdf", request.version)),
            &files.pdf_sha256,
            &mut exported,
            &mut hashes,
        )?;
    }
    let manifest_path = grant
        .path
        .join(format!("{stem}-v{}.manifest.json", request.version));
    let manifest_export = json!({"manifest":files.manifest,"signature":files.signature});
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest_export).map_err(|error| error.to_string())?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&manifest_path)
        .and_then(|mut file| file.write_all(&manifest_bytes))
        .map_err(|error| {
            format!("Artifact manifest export failed without overwriting existing files: {error}")
        })?;
    hashes.insert("manifest".to_string(), sha256_hex(&manifest_bytes));
    exported.push(manifest_path.to_string_lossy().to_string());
    let destination_hash = sha256_hex(grant.path.to_string_lossy().as_bytes());
    repository::record_export(
        persistence.inner(),
        &request.artifact_id,
        request.version,
        &request.format,
        &destination_hash,
        &hashes,
    )?;
    tasks::record_domain_event(
        persistence.inner(),
        &files.task_run_id,
        "artifact.exported",
        EvidenceClass::VerifiedPostcondition,
        json!({"artifactId":request.artifact_id,"version":request.version,"format":request.format,"hashes":hashes}),
    )?;
    Ok(ArtifactExportResult {
        exported_files: exported,
        hashes,
    })
}

fn copy_verified(
    source: &Path,
    destination: &Path,
    expected: &str,
    exported: &mut Vec<String>,
    hashes: &mut HashMap<String, String>,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|_| "Private artifact output is unavailable.".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Private artifact output failed validation.".to_string());
    }
    let mut input = fs::File::open(source)
        .map_err(|error| format!("Artifact export source failed: {error}"))?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| {
            format!("Artifact export refused to overwrite an existing file: {error}")
        })?;
    std::io::copy(&mut input, &mut output)
        .map_err(|error| format!("Artifact export failed: {error}"))?;
    output
        .sync_all()
        .map_err(|error| format!("Artifact export durability check failed: {error}"))?;
    let digest = sha256_file_hex(destination).map_err(|error| error.to_string())?;
    if digest != expected {
        let _ = fs::remove_file(destination);
        return Err("Artifact export digest verification failed.".to_string());
    }
    let key = destination
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("file")
        .to_string();
    hashes.insert(key, digest);
    exported.push(destination.to_string_lossy().to_string());
    Ok(())
}
async fn approve_export(
    app: &tauri::AppHandle,
    approvals: &ShieldApprovalManager,
    files: &repository::VersionPaths,
    destination: &Path,
    format: &str,
) -> Result<(), String> {
    request_user_approval(
        app,
        approvals,
        ShieldApprovalRequest {
            approval_token: format!("approval_{}", hex::encode(random_bytes())),
            session_id: Some(files.artifact_id.clone()),
            turn_id: Some(files.task_run_id.clone()),
            generation_token: None,
            action_type: "artifact_export".to_string(),
            action_label: "Export verified artifact".to_string(),
            target_path: Some(destination.to_string_lossy().to_string()),
            principal: Some(files.project_id.clone()),
            risk_tier: "consequential".to_string(),
            reason: "Verified DOCX/PDF files will leave private staging.".to_string(),
            estimated_token_costs: None,
            requested_at_ms: crate::foundation::clock::unix_time_ms_u64(),
            preview: format!("Export {} ({format}) to the selected folder.", files.title),
            semantic_summary: "Export signed verified artifact".to_string(),
            semantic_detail: format!(
                "DOCX {} and PDF {} remain digest-bound to the signed manifest.",
                files.docx_sha256, files.pdf_sha256
            ),
            approval_tier: "effectful".to_string(),
            approval_mode: "single_exact_destination".to_string(),
            diff_preview: None,
            scope_trust_available: false,
            scope_trust_prefix: None,
            scope_trust_duration_ms: 0,
            project_id: Some(files.project_id.clone()),
            task_run_id: Some(files.task_run_id.clone()),
            action_class: "artifact_export".to_string(),
            argument_class: crate::approval_scopes::argument_class(
                "artifact_export",
                format.as_ref(),
            ),
            canonical_resource: Some(destination.to_string_lossy().to_string()),
            mandatory_reconfirm: true,
            approval_scope_kinds: vec!["once".to_string()],
        },
    )
    .await
    .map_err(|error| error.message)
}
fn random_bytes() -> [u8; 18] {
    use rand_core::{OsRng, RngCore};
    let mut bytes = [0u8; 18];
    OsRng.fill_bytes(&mut bytes);
    bytes
}
fn safe_name(value: &str) -> String {
    let name = value
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
    if name.trim_matches('_').is_empty() {
        "oomu-artifact".to_string()
    } else {
        name
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactPipelineHealth {
    pub builder_available: bool,
    pub renderer_available: bool,
    pub builder_identity: String,
    pub renderer_identity: String,
}
pub(crate) fn probe_pipeline_runtime() -> Result<(), String> {
    runtime::probe_builder()?;
    verifier::probe_renderer()
}
#[tauri::command]
pub async fn get_artifact_pipeline_health() -> Result<ArtifactPipelineHealth, String> {
    let builder = runtime::probe_builder().is_ok();
    let renderer = verifier::probe_renderer().is_ok();
    Ok(ArtifactPipelineHealth {
        builder_available: builder,
        renderer_available: renderer,
        builder_identity: ARTIFACT_BUILDER_IDENTITY.to_string(),
        renderer_identity: ARTIFACT_RENDERER_IDENTITY.to_string(),
    })
}
