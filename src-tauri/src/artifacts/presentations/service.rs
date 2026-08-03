use super::*;
use crate::{
    db::PersistenceEngine, foundation::digest::sha256_file_hex, p0_contracts::EvidenceClass,
    sovereign_identity::SovereignIdentity, tasks,
};
use rusqlite::{params, OptionalExtension};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) fn build_presentation_revision(
    engine: &PersistenceEngine,
    identity: &SovereignIdentity,
    _app: &tauri::AppHandle,
    presentation_id: &str,
    revision: u32,
    presentation: &PresentationIr,
) -> Result<(), String> {
    let app_data = crate::settings::app_data_root();
    let private_root = app_data.join("presentations").join("private");
    let artifact_root = private_root.join(presentation_id);
    let revision_root = artifact_root.join(format!("r{revision}"));
    for directory in [&private_root, &artifact_root] {
        create_private_directory(directory)?;
    }
    if revision_root.exists() {
        return Err("Private presentation revision staging already exists.".to_string());
    }
    create_private_directory(&revision_root)?;
    let mut cleanup = CleanupDirectory {
        path: revision_root.clone(),
        committed: false,
    };
    let ownership = get_presentation_record(engine, presentation_id, Some(revision))?;
    let (built, imported_source) = if presentation.template.imported {
        let template = load_registered_template(
            engine,
            &app_data,
            &ownership.summary.project_id,
            &ownership.summary.task_id,
            &ownership.summary.task_run_id,
            &presentation.template,
        )?;
        (
            build_presentation_from_registered_template(&template, presentation)?,
            Some(template),
        )
    } else {
        (build_presentation(presentation)?, None)
    };
    let pptx = revision_root.join("presentation.pptx");
    write_private_file(&pptx, &built.bytes)?;
    if sha256_file_hex(&pptx).map_err(|error| error.to_string())? != built.package_sha256 {
        return Err("Private presentation package digest verification failed.".to_string());
    }
    let verified = if let Some(source_template) = imported_source.as_deref() {
        verify_imported_presentation_bytes(
            &built.bytes,
            source_template,
            &built.normalized,
            &built.policy_notices,
        )?
    } else {
        verify_presentation_bytes(&built.bytes, &built.normalized, &built.policy_notices)?
    };
    let preview_root = revision_root.join("previews");
    create_private_directory(&preview_root)?;
    let mut stored_previews = Vec::new();
    for (index, preview) in verified.previews.iter().enumerate() {
        let path = preview_root.join(format!("slide-{:04}.png", index + 1));
        write_private_file(&path, &preview.bytes)?;
        if sha256_file_hex(&path).map_err(|error| error.to_string())? != preview.sha256 {
            return Err("Private slide preview digest verification failed.".to_string());
        }
        stored_previews.push(StoredPresentationPreview {
            slide_id: preview.slide_id.clone(),
            path: path.to_string_lossy().to_string(),
            media_type: "image/png".to_string(),
            width: preview.width,
            height: preview.height,
            sha256: preview.sha256.clone(),
        });
    }
    let mut evidence_checked = built.normalized.clone();
    let evidence = bind_presentation_provenance(
        engine,
        &ownership.summary.project_id,
        &ownership.summary.task_id,
        &ownership.summary.task_run_id,
        &mut evidence_checked,
    )?;
    let contract = artifact_presentation_contract(
        &ownership.summary.project_id,
        &ownership.summary.task_id,
        &ownership.summary.task_run_id,
        &ownership.summary.artifact_id,
        &built.normalized,
        &evidence,
    )?;
    let manifest = json!({
        "schemaVersion": 1,
        "presentationId": presentation_id,
        "revision": revision,
        "contract": contract,
        "pptx": {"sha256": built.package_sha256, "bytes": built.bytes.len()},
        "previews": stored_previews.iter().map(|preview| json!({"slideId":preview.slide_id,"sha256":preview.sha256,"width":preview.width,"height":preview.height})).collect::<Vec<_>>(),
        "verification": verified.record,
    });
    let payload = serde_json::to_string(&manifest).map_err(|error| error.to_string())?;
    let signature = identity
        .sign_payload(&payload)
        .map_err(|error| error.message)?;
    identity
        .verify_payload(&payload, &signature)
        .map_err(|error| error.message)?;
    complete_presentation_revision(
        engine,
        CompletedPresentationRevision {
            presentation_id,
            revision,
            presentation: &built.normalized,
            pptx: &pptx,
            previews: &stored_previews,
            verification: &verified.record,
            manifest: &manifest,
            signature: &signature,
            pptx_sha256: &built.package_sha256,
            pptx_bytes: built.bytes.len() as u64,
        },
    )?;
    cleanup.committed = true;
    let evidence_class = if verified.record.exportable {
        EvidenceClass::SignedArtifact
    } else {
        EvidenceClass::ObservedResult
    };
    tasks::record_domain_event(
        engine,
        &ownership.summary.task_run_id,
        "presentation.review_ready",
        evidence_class,
        json!({"presentationId":presentation_id,"revision":revision,"pptxSha256":built.package_sha256,"exportable":verified.record.exportable,"manifestSignature":signature}),
    )?;
    Ok(())
}

pub(crate) async fn build_presentation_revision_off_thread(
    engine: PersistenceEngine,
    identity: SovereignIdentity,
    app: tauri::AppHandle,
    presentation_id: String,
    revision: u32,
    presentation: PresentationIr,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        build_presentation_revision(
            &engine,
            &identity,
            &app,
            &presentation_id,
            revision,
            &presentation,
        )
    })
    .await
    .map_err(|error| format!("Presentation build worker failed: {error}"))?
}

fn load_registered_template(
    engine: &PersistenceEngine,
    app_data: &Path,
    project_id: &str,
    task_id: &str,
    task_run_id: &str,
    identity: &PresentationTemplateIdentity,
) -> Result<Vec<u8>, String> {
    let template_id = identity
        .template_id
        .as_deref()
        .ok_or_else(|| "Imported template identity is incomplete.".to_string())?;
    let (path, fingerprint): (String, String) = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT private_path,fingerprint_sha256 FROM presentation_template_imports WHERE template_id=?1 AND project_id=?2 AND task_id=?3 AND task_run_id=?4",
            params![template_id,project_id,task_id,task_run_id],
            |row| Ok((row.get(0)?,row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Registered presentation template is unavailable.".to_string())?;
    if fingerprint != identity.fingerprint_sha256 {
        return Err("Registered presentation template identity changed.".to_string());
    }
    let root = app_data.join("presentations").join("templates");
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let path = PathBuf::from(path);
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    let canonical = fs::canonicalize(&path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || !canonical.starts_with(canonical_root)
        || metadata.len() == 0
        || metadata.len() > 128 * 1024 * 1024
    {
        return Err("Registered presentation template failed containment checks.".to_string());
    }
    let bytes = fs::read(canonical).map_err(|error| error.to_string())?;
    if super::ooxml::hex_digest(&bytes) != fingerprint {
        return Err("Registered presentation template digest changed.".to_string());
    }
    Ok(bytes)
}

pub(crate) fn create_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| error.to_string())?
    };
    #[cfg(not(unix))]
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())
}

struct CleanupDirectory {
    path: PathBuf,
    committed: bool,
}
impl Drop for CleanupDirectory {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
