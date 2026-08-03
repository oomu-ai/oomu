use super::*;
use crate::{
    db::PersistenceEngine,
    foundation::digest::sha256_file_hex,
    p0_contracts::EvidenceClass,
    shield_gate::{request_user_approval, ShieldApprovalManager, ShieldApprovalRequest},
    sovereign_identity::SovereignIdentity,
    tasks,
};
use rusqlite::params;
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};

const GRANT_TTL_MS: i64 = 10 * 60 * 1_000;
type CommandResult<T> = Result<T, PresentationCommandError>;

#[derive(Default)]
pub struct PresentationExportGrantStore {
    grants: Mutex<HashMap<String, DestinationGrant>>,
}

struct DestinationGrant {
    presentation_id: String,
    revision: u32,
    destination: PathBuf,
    display_name: String,
    expires_at_ms: i64,
}

#[tauri::command]
pub async fn choose_presentation_export_destination(
    request: ChoosePresentationExportDestinationRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    grants: tauri::State<'_, PresentationExportGrantStore>,
) -> CommandResult<Option<PresentationExportGrant>> {
    let files = presentation_revision_files(
        persistence.inner(),
        &request.presentation_id,
        request.revision,
    )
    .map_err(|value| command_error("presentation_export_unavailable", value))?;
    verify_export_source(identity.inner(), &files)?;
    if request.suggested_name.chars().count() > 128 || request.suggested_name.contains('\0') {
        return Err(command_error(
            "presentation_export_invalid",
            "Suggested export name is invalid.",
        ));
    }
    let requested = request.suggested_name.trim_end_matches(".pptx");
    let stem = if requested.trim().is_empty() {
        safe_name(&files.title)
    } else {
        safe_name(requested)
    };
    let suggested = format!("{stem}-r{}.pptx", request.revision);
    let Some(handle) = rfd::AsyncFileDialog::new()
        .add_filter("PPTX", &["pptx"])
        .set_file_name(&suggested)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let destination = handle.path().to_path_buf();
    if destination.extension().and_then(|value| value.to_str()) != Some("pptx") {
        return Err(command_error(
            "presentation_export_invalid",
            "Presentation export destination must end in .pptx.",
        ));
    }
    let display_name = destination
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .ok_or_else(|| {
            command_error(
                "presentation_export_invalid",
                "Presentation destination has no file name.",
            )
        })?;
    let grant_token = format!("presentation-export-{}", hex::encode(random_bytes()));
    let expires_at_ms = crate::foundation::clock::unix_time_ms_i64().saturating_add(GRANT_TTL_MS);
    let mut locked = grants.grants.lock().map_err(|_| {
        command_error(
            "presentation_export_unavailable",
            "Export grants are unavailable.",
        )
    })?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    locked.retain(|_, grant| grant.expires_at_ms >= now);
    if locked.len() >= 128 {
        return Err(command_error(
            "presentation_export_grant_limit",
            "Too many presentation export destinations are pending.",
        ));
    }
    locked.insert(
        grant_token.clone(),
        DestinationGrant {
            presentation_id: request.presentation_id,
            revision: request.revision,
            destination,
            display_name: display_name.clone(),
            expires_at_ms,
        },
    );
    Ok(Some(PresentationExportGrant {
        grant_token,
        display_name,
        expires_at_ms,
    }))
}

#[tauri::command]
pub async fn export_presentation_revision(
    request: ExportPresentationRevisionRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    grants: tauri::State<'_, PresentationExportGrantStore>,
    app: tauri::AppHandle,
) -> CommandResult<PresentationExportResult> {
    let grant = take_grant(grants.inner(), &request)?;
    let files = presentation_revision_files(
        persistence.inner(),
        &request.presentation_id,
        request.revision,
    )
    .map_err(|value| command_error("presentation_export_unavailable", value))?;
    verify_export_source(identity.inner(), &files)?;
    approve_export(&app, approvals.inner(), &files, &grant.destination).await?;
    let private_root = crate::settings::app_data_root()
        .join("presentations")
        .join("private");
    let export_id = format!("presentation-export-record-{}", hex::encode(random_bytes()));
    let receipt_id = format!("presentation-receipt-{}", hex::encode(random_bytes()));
    persistence.inner().open_connection().map_err(|value|command_error("presentation_export_failed",value.to_string()))?.execute(
        "INSERT INTO presentation_exports(export_id,presentation_id,revision,destination_name,pptx_sha256,receipt_id,status_code,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,'copying',?7)",
        params![export_id,request.presentation_id,request.revision as i64,grant.display_name,files.sha256,receipt_id,crate::foundation::clock::unix_time_ms_i64()],
    ).map_err(|value|command_error("presentation_export_failed",value.to_string()))?;
    let source = files.pptx.clone();
    let destination = grant.destination.clone();
    let expected = files.sha256.clone();
    let copy = tauri::async_runtime::spawn_blocking(move || {
        copy_immutable_verified(&private_root, &source, &destination, &expected)
    })
    .await
    .map_err(|value| command_error("presentation_export_failed", value.to_string()))?;
    let digest = match copy {
        Ok(value) => value,
        Err(value) => {
            let _ = persistence.inner().open_connection().and_then(|connection| connection.execute(
                "UPDATE presentation_exports SET status_code='failed',last_error=?2,completed_at_ms=?3 WHERE export_id=?1",
                params![export_id,value.chars().take(1000).collect::<String>(),crate::foundation::clock::unix_time_ms_i64()],
            ));
            return Err(command_error("presentation_export_failed", value));
        }
    };
    persistence.inner().open_connection().map_err(|value|command_error("presentation_export_failed",value.to_string()))?.execute(
        "UPDATE presentation_exports SET status_code='completed',pptx_sha256=?2,completed_at_ms=?3,last_error=NULL WHERE export_id=?1 AND status_code='copying'",
        params![export_id,digest,crate::foundation::clock::unix_time_ms_i64()],
    ).map_err(|value|command_error("presentation_export_failed",value.to_string()))?;
    if let Err(value) = tasks::record_domain_event(
        persistence.inner(),
        &files.task_run_id,
        "presentation.exported",
        EvidenceClass::VerifiedPostcondition,
        json!({"presentationId":request.presentation_id,"revision":request.revision,"pptxSha256":digest,"receiptId":receipt_id}),
    ) {
        eprintln!("PRESENTATION_EXPORT_EVENT_PENDING code=presentation_event_failed export_id={} error={}", export_id, value);
    }
    Ok(PresentationExportResult {
        presentation_id: request.presentation_id,
        revision: request.revision,
        display_name: grant.display_name,
        sha256: digest,
        receipt_id,
    })
}

pub(crate) async fn export_presentation_revision_to_approved_path(
    presentation_id: &str,
    revision: u32,
    destination_path: &str,
    persistence: &PersistenceEngine,
    identity: &SovereignIdentity,
    _app: &tauri::AppHandle,
) -> CommandResult<PresentationExportResult> {
    let destination = crate::shield_gate::validate_approved_external_write_target(destination_path)
        .map_err(|error| command_error("presentation_export_failed", error.message))?;
    if destination
        .extension()
        .and_then(|value| value.to_str())
        .is_none_or(|value| !value.eq_ignore_ascii_case("pptx"))
    {
        return Err(command_error(
            "presentation_export_failed",
            "The file name no longer matches the PPTX format.",
        ));
    }
    let files = presentation_revision_files(persistence, presentation_id, revision)
        .map_err(|value| command_error("presentation_export_unavailable", value))?;
    verify_export_source(identity, &files)?;
    let private_root = crate::settings::app_data_root()
        .join("presentations")
        .join("private");
    let export_id = format!("presentation-export-record-{}", hex::encode(random_bytes()));
    let receipt_id = format!("presentation-receipt-{}", hex::encode(random_bytes()));
    let display_name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Presentation.pptx")
        .to_string();
    persistence.open_connection().map_err(|value|command_error("presentation_export_failed",value.to_string()))?.execute(
        "INSERT INTO presentation_exports(export_id,presentation_id,revision,destination_name,pptx_sha256,receipt_id,status_code,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,'copying',?7)",
        params![export_id,presentation_id,revision as i64,display_name,files.sha256,receipt_id,crate::foundation::clock::unix_time_ms_i64()],
    ).map_err(|value|command_error("presentation_export_failed",value.to_string()))?;
    let digest = match copy_immutable_verified(
        &private_root,
        &files.pptx,
        &destination,
        &files.sha256,
    ) {
        Ok(value) => value,
        Err(value) => {
            let _ = persistence.open_connection().and_then(|connection| connection.execute(
                "UPDATE presentation_exports SET status_code='failed',last_error=?2,completed_at_ms=?3 WHERE export_id=?1",
                params![export_id,value.chars().take(1000).collect::<String>(),crate::foundation::clock::unix_time_ms_i64()],
            ));
            return Err(command_error("presentation_export_failed", value));
        }
    };
    persistence.open_connection().map_err(|value|command_error("presentation_export_failed",value.to_string()))?.execute(
        "UPDATE presentation_exports SET status_code='completed',pptx_sha256=?2,completed_at_ms=?3,last_error=NULL WHERE export_id=?1 AND status_code='copying'",
        params![export_id,digest,crate::foundation::clock::unix_time_ms_i64()],
    ).map_err(|value|command_error("presentation_export_failed",value.to_string()))?;
    tasks::record_domain_event(
        persistence,
        &files.task_run_id,
        "presentation.exported",
        EvidenceClass::VerifiedPostcondition,
        json!({"presentationId":presentation_id,"revision":revision,"pptxSha256":digest,"receiptId":receipt_id}),
    )
    .map_err(|value| command_error("presentation_event_failed", value))?;
    Ok(PresentationExportResult {
        presentation_id: presentation_id.to_string(),
        revision,
        display_name,
        sha256: digest,
        receipt_id,
    })
}

fn take_grant(
    store: &PresentationExportGrantStore,
    request: &ExportPresentationRevisionRequest,
) -> CommandResult<DestinationGrant> {
    let grant = store
        .grants
        .lock()
        .map_err(|_| {
            command_error(
                "presentation_export_unavailable",
                "Export grants are unavailable.",
            )
        })?
        .remove(&request.grant_token)
        .ok_or_else(|| {
            command_error(
                "presentation_export_grant_invalid",
                "Export grant is missing or already used.",
            )
        })?;
    if grant.presentation_id != request.presentation_id
        || grant.revision != request.revision
        || grant.expires_at_ms < crate::foundation::clock::unix_time_ms_i64()
    {
        return Err(command_error(
            "presentation_export_grant_invalid",
            "Export grant is expired or bound to another revision.",
        ));
    }
    Ok(grant)
}

fn verify_export_source(
    identity: &SovereignIdentity,
    files: &PresentationRevisionFiles,
) -> CommandResult<()> {
    if !files.verification.structurally_verified
        || !files.verification.visually_verified
        || !files.verification.exportable
    {
        return Err(command_error(
            "presentation_export_not_ready",
            "Presentation structural and visual checks must pass before export.",
        ));
    }
    let payload = serde_json::to_string(&files.manifest)
        .map_err(|value| command_error("presentation_export_unavailable", value.to_string()))?;
    identity
        .verify_payload(&payload, &files.signature)
        .map_err(|value| command_error("presentation_export_unavailable", value.message))
}

fn copy_immutable_verified(
    private_root: &Path,
    source: &Path,
    destination: &Path,
    expected_sha256: &str,
) -> Result<String, String> {
    let source_metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    let canonical_root = fs::canonicalize(private_root).map_err(|error| error.to_string())?;
    let canonical_source = fs::canonicalize(source).map_err(|error| error.to_string())?;
    if source_metadata.file_type().is_symlink()
        || !source_metadata.is_file()
        || !canonical_source.starts_with(canonical_root)
        || sha256_file_hex(&canonical_source).map_err(|error| error.to_string())? != expected_sha256
    {
        return Err(
            "Private presentation package failed containment or digest checks.".to_string(),
        );
    }
    if destination.extension().and_then(|value| value.to_str()) != Some("pptx") {
        return Err("Presentation destination extension changed after selection.".to_string());
    }
    if fs::symlink_metadata(destination).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("Presentation destination cannot be a symbolic link.".to_string());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "Presentation destination has no parent directory.".to_string())?;
    let temporary = parent.join(format!(
        ".oomu-presentation-{}.tmp",
        hex::encode(random_bytes())
    ));
    let input = fs::File::open(canonical_source).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    let mut output = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| error.to_string())?
    };
    #[cfg(not(unix))]
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let copied = std::io::copy(&mut input.take(256 * 1024 * 1024 + 1), &mut output)
        .map_err(|error| error.to_string())?;
    output.sync_all().map_err(|error| error.to_string())?;
    if copied == 0
        || copied > 256 * 1024 * 1024
        || sha256_file_hex(&temporary).map_err(|error| error.to_string())? != expected_sha256
    {
        let _ = fs::remove_file(&temporary);
        return Err("Exported presentation copy failed digest verification.".to_string());
    }
    if let Err(error) = fs::hard_link(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Presentation export refused to overwrite an existing file: {error}"
        ));
    }
    if let Err(error) = fs::remove_file(&temporary) {
        let destination_metadata = fs::symlink_metadata(destination);
        if destination_metadata.is_err()
            || destination_metadata
                .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(format!(
                "Presentation export could not finalize its verified package: {error}"
            ));
        }
    }
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())?;
    let digest = sha256_file_hex(destination).map_err(|error| error.to_string())?;
    if digest != expected_sha256 {
        return Err("Exported presentation changed after atomic placement.".to_string());
    }
    Ok(digest)
}

async fn approve_export(
    app: &tauri::AppHandle,
    approvals: &ShieldApprovalManager,
    files: &PresentationRevisionFiles,
    destination: &Path,
) -> CommandResult<()> {
    request_user_approval(
        app,
        approvals,
        ShieldApprovalRequest {
            approval_token: format!("approval_{}", hex::encode(random_bytes())),
            session_id: Some(files.presentation_id.clone()),
            turn_id: Some(files.task_run_id.clone()),
            generation_token: None,
            action_type: "presentation_export".into(),
            action_label: "presentation_export_action".into(),
            target_path: Some(destination.to_string_lossy().to_string()),
            principal: Some(files.project_id.clone()),
            risk_tier: "consequential".into(),
            reason: "presentation_export_reason".into(),
            estimated_token_costs: None,
            requested_at_ms: crate::foundation::clock::unix_time_ms_u64(),
            preview: String::new(),
            semantic_summary: "presentation_export_title".into(),
            semantic_detail: "presentation_export_detail".into(),
            approval_tier: "effectful".into(),
            approval_mode: "single_exact_destination".into(),
            diff_preview: None,
            scope_trust_available: false,
            scope_trust_prefix: None,
            scope_trust_duration_ms: 0,
            project_id: Some(files.project_id.clone()),
            task_run_id: Some(files.task_run_id.clone()),
            action_class: "presentation_export".into(),
            argument_class: crate::approval_scopes::argument_class("presentation_export", "pptx"),
            canonical_resource: Some(destination.to_string_lossy().to_string()),
            mandatory_reconfirm: true,
            approval_scope_kinds: vec!["once".into()],
        },
    )
    .await
    .map_err(|value| command_error("presentation_export_not_approved", value.message))
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
        "oomu-presentation".to_string()
    } else {
        name
    }
}
fn command_error(code: &str, message: impl Into<String>) -> PresentationCommandError {
    PresentationCommandError::new(code, message)
}
fn random_bytes() -> [u8; 18] {
    use rand_core::{OsRng, RngCore};
    let mut bytes = [0_u8; 18];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_copy_succeeds_once_and_never_overwrites() {
        let root = std::env::temp_dir().join(format!(
            "oomu-presentation-export-test-{}",
            hex::encode(random_bytes())
        ));
        let private_root = root.join("private");
        fs::create_dir_all(&private_root).unwrap();
        let source = private_root.join("source.pptx");
        let destination = root.join("decision.pptx");
        fs::write(&source, b"PK verified presentation").unwrap();
        let digest = sha256_file_hex(&source).unwrap();

        assert_eq!(
            copy_immutable_verified(&private_root, &source, &destination, &digest).unwrap(),
            digest
        );
        assert_eq!(fs::read(&destination).unwrap(), b"PK verified presentation");

        let second_destination = root.join("existing.pptx");
        fs::write(&second_destination, b"user-owned existing file").unwrap();
        let error = copy_immutable_verified(&private_root, &source, &second_destination, &digest)
            .unwrap_err();
        assert!(error.contains("refused to overwrite"));
        assert_eq!(
            fs::read(&second_destination).unwrap(),
            b"user-owned existing file"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn destination_grants_are_bound_expiring_and_single_use() {
        let store = PresentationExportGrantStore::default();
        let presentation_id = crate::p0_contracts::ArtifactId::new().to_string();
        store.grants.lock().unwrap().insert(
            "grant".to_string(),
            DestinationGrant {
                presentation_id: presentation_id.clone(),
                revision: 3,
                destination: PathBuf::from("/tmp/result.pptx"),
                display_name: "result.pptx".to_string(),
                expires_at_ms: crate::foundation::clock::unix_time_ms_i64() + 60_000,
            },
        );
        let request = ExportPresentationRevisionRequest {
            presentation_id: presentation_id.clone(),
            revision: 3,
            grant_token: "grant".to_string(),
        };
        assert!(take_grant(&store, &request).is_ok());
        assert!(take_grant(&store, &request).is_err());

        store.grants.lock().unwrap().insert(
            "expired".to_string(),
            DestinationGrant {
                presentation_id,
                revision: 3,
                destination: PathBuf::from("/tmp/result.pptx"),
                display_name: "result.pptx".to_string(),
                expires_at_ms: 0,
            },
        );
        let expired = ExportPresentationRevisionRequest {
            grant_token: "expired".to_string(),
            ..request
        };
        assert!(take_grant(&store, &expired).is_err());
    }
}
