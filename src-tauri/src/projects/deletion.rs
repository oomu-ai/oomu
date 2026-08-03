use super::repository::{archive, get, user_managed_project_id};
use super::*;
use crate::db::PersistenceEngine;
use rusqlite::params;
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

fn dependent_count(
    connection: &rusqlite::Connection,
    table: &str,
    project_id: &str,
) -> Result<usize, String> {
    connection
        .query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE project_id=?1"),
            params![project_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count as usize)
        .map_err(|error| error.to_string())
}

#[derive(Debug)]
pub(crate) struct ProjectDeletionPlan {
    preview: ProjectDeletionPreview,
    owned_paths: Vec<PathBuf>,
}

struct StagedProjectFiles {
    trash_root: PathBuf,
    moves: Vec<(PathBuf, PathBuf)>,
}

fn safe_path_component(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!(
            "Unsafe private {label} identity blocked project deletion."
        ));
    }
    Ok(())
}

fn validate_owned_candidate(app_data: &Path, path: &Path) -> Result<(), String> {
    if !app_data.is_absolute() || !path.is_absolute() || path == app_data {
        return Err("Unsafe private project path blocked deletion.".to_string());
    }
    let relative = path
        .strip_prefix(app_data)
        .map_err(|_| "Private project file escaped OOMU storage.".to_string())?;
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
                    | Component::CurDir
            )
        })
    {
        return Err("Unsafe private project path blocked deletion.".to_string());
    }
    let canonical_app_data = fs::canonicalize(app_data)
        .map_err(|_| "OOMU private storage is unavailable.".to_string())?;
    let mut cursor = app_data.to_path_buf();
    for component in relative.components() {
        cursor.push(component.as_os_str());
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("A symbolic link blocked safe project-file deletion.".to_string())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    if path.exists() {
        let canonical = fs::canonicalize(path).map_err(|error| error.to_string())?;
        if !canonical.starts_with(&canonical_app_data) {
            return Err("Private project file escaped OOMU storage.".to_string());
        }
    }
    Ok(())
}

fn validate_recorded_path(app_data: &Path, allowed_root: &Path, raw: &str) -> Result<(), String> {
    let path = PathBuf::from(raw);
    if !path.starts_with(allowed_root) {
        return Err("A recorded project file is outside its OOMU-owned directory.".to_string());
    }
    validate_owned_candidate(app_data, &path)
}

fn validate_preview_paths(
    app_data: &Path,
    allowed_root: &Path,
    raw_json: Option<&str>,
) -> Result<(), String> {
    let Some(raw_json) = raw_json.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };
    let value: serde_json::Value = serde_json::from_str(raw_json)
        .map_err(|_| "A private preview manifest is invalid.".to_string())?;
    let Some(items) = value.as_array() else {
        return Err("A private preview manifest is invalid.".to_string());
    };
    for item in items {
        let raw_path = item
            .as_str()
            .or_else(|| item.get("path").and_then(serde_json::Value::as_str));
        if let Some(raw_path) = raw_path {
            validate_recorded_path(app_data, allowed_root, raw_path)?;
        }
    }
    Ok(())
}

fn collect_project_owned_paths(
    connection: &rusqlite::Connection,
    app_data: &Path,
    project_id: &str,
) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(app_data).map_err(|error| error.to_string())?;
    let mut paths = BTreeSet::new();

    {
        let mut statement = connection.prepare(
            "SELECT r.artifact_id,v.docx_private_path,v.pdf_private_path,v.preview_manifest_json FROM artifact_records r LEFT JOIN artifact_versions v ON v.artifact_id=r.artifact_id WHERE r.project_id=?1",
        ).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![project_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (artifact_id, docx, pdf, previews) = row.map_err(|error| error.to_string())?;
            safe_path_component(&artifact_id, "artifact")?;
            let root = app_data.join("artifacts").join("staging").join(artifact_id);
            validate_owned_candidate(app_data, &root)?;
            for recorded in [docx.as_deref(), pdf.as_deref()].into_iter().flatten() {
                validate_recorded_path(app_data, &root, recorded)?;
            }
            validate_preview_paths(app_data, &root, previews.as_deref())?;
            paths.insert(root);
        }
    }

    {
        let mut statement = connection.prepare(
            "SELECT r.artifact_id,v.xlsx_private_path,v.preview_manifest_json FROM workbook_records r LEFT JOIN workbook_revisions v ON v.artifact_id=r.artifact_id WHERE r.project_id=?1",
        ).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![project_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (artifact_id, workbook, previews) = row.map_err(|error| error.to_string())?;
            safe_path_component(&artifact_id, "workbook")?;
            let root = app_data.join("workbooks").join("staging").join(artifact_id);
            validate_owned_candidate(app_data, &root)?;
            if let Some(recorded) = workbook.as_deref() {
                validate_recorded_path(app_data, &root, recorded)?;
            }
            validate_preview_paths(app_data, &root, previews.as_deref())?;
            paths.insert(root);
        }
    }

    {
        let mut statement = connection.prepare(
            "SELECT r.presentation_id,v.pptx_private_path,v.preview_manifest_json FROM presentation_records r LEFT JOIN presentation_revisions v ON v.presentation_id=r.presentation_id WHERE r.project_id=?1",
        ).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![project_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (presentation_id, presentation, previews) =
                row.map_err(|error| error.to_string())?;
            safe_path_component(&presentation_id, "presentation")?;
            let root = app_data
                .join("presentations")
                .join("private")
                .join(presentation_id);
            validate_owned_candidate(app_data, &root)?;
            if let Some(recorded) = presentation.as_deref() {
                validate_recorded_path(app_data, &root, recorded)?;
            }
            validate_preview_paths(app_data, &root, previews.as_deref())?;
            paths.insert(root);
        }
    }

    {
        let root = app_data.join("workbooks").join("template-imports");
        let mut statement = connection.prepare(
            "SELECT template_token,source_private_path FROM workbook_template_imports WHERE project_id=?1",
        ).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![project_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (token, recorded) = row.map_err(|error| error.to_string())?;
            safe_path_component(&token, "workbook template")?;
            let expected = root.join(format!("{token}.xlsx"));
            validate_owned_candidate(app_data, &expected)?;
            if Path::new(&recorded) != expected {
                return Err("A recorded workbook template path is unsafe.".to_string());
            }
            paths.insert(expected);
        }
    }

    {
        let root = app_data.join("presentations").join("templates");
        let mut statement = connection.prepare(
            "SELECT template_id,private_path FROM presentation_template_imports WHERE project_id=?1",
        ).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![project_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (template_id, recorded) = row.map_err(|error| error.to_string())?;
            safe_path_component(&template_id, "presentation template")?;
            let expected = root.join(format!("{template_id}.pptx"));
            validate_owned_candidate(app_data, &expected)?;
            if Path::new(&recorded) != expected {
                return Err("A recorded presentation template path is unsafe.".to_string());
            }
            paths.insert(expected);
        }
    }

    {
        let evidence_root = app_data.join("browser-evidence");
        let quarantine_root = app_data.join("browser-quarantine");
        let mut statement = connection.prepare(
            "SELECT s.session_id,a.screenshot_path,d.private_path FROM browser_automation_sessions s LEFT JOIN browser_automation_actions a ON a.session_id=s.session_id LEFT JOIN browser_download_quarantine d ON d.session_id=s.session_id WHERE s.project_id=?1",
        ).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![project_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (session_id, screenshot, download) = row.map_err(|error| error.to_string())?;
            safe_path_component(&session_id, "browser session")?;
            let session_root = evidence_root.join(session_id);
            validate_owned_candidate(app_data, &session_root)?;
            if let Some(recorded) = screenshot.as_deref() {
                validate_recorded_path(app_data, &session_root, recorded)?;
            }
            if let Some(recorded) = download.as_deref() {
                validate_recorded_path(app_data, &quarantine_root, recorded)?;
                paths.insert(PathBuf::from(recorded));
            }
            paths.insert(session_root);
        }
    }

    {
        let artifact_root = app_data.join("artifacts").join("staging");
        let mut statement = connection
            .prepare(
                "SELECT artifact_id,private_path FROM remote_artifact_grants WHERE project_id=?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![project_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (artifact_id, recorded) = row.map_err(|error| error.to_string())?;
            safe_path_component(&artifact_id, "remote artifact")?;
            let root = artifact_root.join(artifact_id);
            validate_recorded_path(app_data, &root, &recorded)?;
            paths.insert(root);
        }
    }

    let mut paths = paths.into_iter().collect::<Vec<_>>();
    paths.sort_by_key(|path| path.components().count());
    let mut deduplicated = Vec::<PathBuf>::new();
    for path in paths {
        if deduplicated.iter().any(|parent| path.starts_with(parent)) {
            continue;
        }
        deduplicated.push(path);
    }
    Ok(deduplicated)
}

fn count_owned_files(path: &Path) -> Result<usize, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.file_type().is_symlink() {
        return Err("A symbolic link blocked safe project-file deletion.".to_string());
    }
    if metadata.is_file() {
        return Ok(1);
    }
    if !metadata.is_dir() {
        return Err("An unsupported private filesystem entry blocked deletion.".to_string());
    }
    let mut count = 0usize;
    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        count = count
            .checked_add(count_owned_files(
                &entry.map_err(|error| error.to_string())?.path(),
            )?)
            .ok_or_else(|| "Project file count overflowed.".to_string())?;
    }
    Ok(count)
}

fn stage_project_files(
    app_data: &Path,
    project_id: &str,
    paths: &[PathBuf],
) -> Result<Option<StagedProjectFiles>, String> {
    let existing = paths
        .iter()
        .filter(|path| path.exists())
        .cloned()
        .collect::<Vec<_>>();
    if existing.is_empty() {
        return Ok(None);
    }
    let trash_parent = app_data.join("project-deletion-trash");
    fs::create_dir_all(&trash_parent).map_err(|error| error.to_string())?;
    validate_owned_candidate(app_data, &trash_parent)?;
    let trash_root = trash_parent.join(format!(
        "{}-{}-{}",
        project_id,
        crate::foundation::clock::unix_time_ms_i64(),
        std::process::id()
    ));
    fs::create_dir(&trash_root).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&trash_root, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    let mut staged = StagedProjectFiles {
        trash_root,
        moves: Vec::new(),
    };
    for (index, source) in existing.into_iter().enumerate() {
        validate_owned_candidate(app_data, &source)?;
        let destination = staged.trash_root.join(format!("item-{index:04}"));
        if let Err(error) = fs::rename(&source, &destination) {
            for (original, staged_path) in staged.moves.iter().rev() {
                let _ = fs::rename(staged_path, original);
            }
            let _ = fs::remove_dir_all(&staged.trash_root);
            return Err(error.to_string());
        }
        staged.moves.push((source, destination));
    }
    Ok(Some(staged))
}

fn restore_staged_project_files(staged: &StagedProjectFiles) {
    for (original, staged_path) in staged.moves.iter().rev() {
        let _ = fs::rename(staged_path, original);
    }
    let _ = fs::remove_dir_all(&staged.trash_root);
}

fn remove_project_from_json_assignments(
    tx: &rusqlite::Transaction<'_>,
    project_id: &str,
) -> Result<(), String> {
    let devices = {
        let mut statement = tx
            .prepare("SELECT remote_device_id,allowed_project_ids_json FROM remote_devices")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for (id, raw) in devices {
        let mut projects: Vec<String> = serde_json::from_str(&raw)
            .map_err(|_| "A remote-device Project assignment is invalid.".to_string())?;
        let original_len = projects.len();
        projects.retain(|value| value != project_id);
        if projects.len() != original_len {
            tx.execute(
                "UPDATE remote_devices SET allowed_project_ids_json=?2 WHERE remote_device_id=?1",
                params![
                    id,
                    serde_json::to_string(&projects).map_err(|error| error.to_string())?
                ],
            )
            .map_err(|error| error.to_string())?;
        }
    }

    let challenges = {
        let mut statement = tx
            .prepare("SELECT challenge_id,allowed_project_ids_json FROM remote_pairing_challenges")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for (id, raw) in challenges {
        let projects: Vec<String> = serde_json::from_str(&raw)
            .map_err(|_| "A remote-pairing Project assignment is invalid.".to_string())?;
        if projects.iter().any(|value| value == project_id) {
            tx.execute(
                "DELETE FROM remote_pairing_challenges WHERE challenge_id=?1",
                params![id],
            )
            .map_err(|error| error.to_string())?;
        }
    }

    let bundles = {
        let mut statement = tx
            .prepare(
                "SELECT bundle_id,package_version,project_ids_json FROM capability_bundle_records",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for (bundle_id, version, raw) in bundles {
        let mut projects: Vec<String> = serde_json::from_str(&raw)
            .map_err(|_| "A capability-bundle Project assignment is invalid.".to_string())?;
        let original_len = projects.len();
        projects.retain(|value| value != project_id);
        if projects.len() != original_len {
            tx.execute(
                "UPDATE capability_bundle_records SET project_ids_json=?3,updated_at_ms=?4 WHERE bundle_id=?1 AND package_version=?2",
                params![bundle_id, version, serde_json::to_string(&projects).map_err(|error| error.to_string())?, crate::foundation::clock::unix_time_ms_i64()],
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn purge_project_rows(tx: &rusqlite::Transaction<'_>, project_id: &str) -> Result<(), String> {
    remove_project_from_json_assignments(tx, project_id)?;

    let statements = [
        "UPDATE remote_audit_receipts SET command_id=NULL WHERE command_id IN (SELECT command_id FROM remote_commands WHERE project_id=?1)",
        "DELETE FROM remote_artifact_grants WHERE project_id=?1",
        "DELETE FROM remote_commands WHERE project_id=?1",
        "DELETE FROM capability_runtime_denials WHERE project_id=?1",
        "DELETE FROM media_asset_relationships WHERE media_asset_id IN (SELECT media_asset_id FROM media_assets WHERE project_id=?1) OR related_media_asset_id IN (SELECT media_asset_id FROM media_assets WHERE project_id=?1)",
        "DELETE FROM media_transcripts WHERE media_asset_id IN (SELECT media_asset_id FROM media_assets WHERE project_id=?1)",
        "DELETE FROM media_evidence WHERE project_id=?1 OR media_asset_id IN (SELECT media_asset_id FROM media_assets WHERE project_id=?1)",
        "DELETE FROM media_interpretations WHERE media_asset_id IN (SELECT media_asset_id FROM media_assets WHERE project_id=?1)",
        "DELETE FROM media_assets WHERE project_id=?1",
        "DELETE FROM presentation_source_links WHERE presentation_id IN (SELECT presentation_id FROM presentation_records WHERE project_id=?1)",
        "DELETE FROM presentation_exports WHERE presentation_id IN (SELECT presentation_id FROM presentation_records WHERE project_id=?1)",
        "DELETE FROM presentation_revisions WHERE presentation_id IN (SELECT presentation_id FROM presentation_records WHERE project_id=?1)",
        "DELETE FROM presentation_template_imports WHERE project_id=?1",
        "DELETE FROM presentation_records WHERE project_id=?1",
        "DELETE FROM workbook_source_links WHERE artifact_id IN (SELECT artifact_id FROM workbook_records WHERE project_id=?1)",
        "DELETE FROM workbook_exports WHERE artifact_id IN (SELECT artifact_id FROM workbook_records WHERE project_id=?1)",
        "DELETE FROM workbook_revisions WHERE artifact_id IN (SELECT artifact_id FROM workbook_records WHERE project_id=?1)",
        "DELETE FROM workbook_template_imports WHERE project_id=?1",
        "DELETE FROM workbook_records WHERE project_id=?1",
        "DELETE FROM artifact_source_links WHERE artifact_id IN (SELECT artifact_id FROM artifact_records WHERE project_id=?1)",
        "DELETE FROM artifact_exports WHERE artifact_id IN (SELECT artifact_id FROM artifact_records WHERE project_id=?1)",
        "DELETE FROM artifact_versions WHERE artifact_id IN (SELECT artifact_id FROM artifact_records WHERE project_id=?1)",
        "DELETE FROM artifact_records WHERE project_id=?1",
        "DELETE FROM browser_automation_actions WHERE session_id IN (SELECT session_id FROM browser_automation_sessions WHERE project_id=?1)",
        "DELETE FROM browser_download_quarantine WHERE session_id IN (SELECT session_id FROM browser_automation_sessions WHERE project_id=?1)",
        "DELETE FROM browser_automation_sessions WHERE project_id=?1",
        "DELETE FROM work_graph_suggestions WHERE plan_id IN (SELECT plan_id FROM delegation_plans WHERE project_id=?1) OR task_run_id IN (SELECT task_run_id FROM task_runs WHERE project_id=?1)",
        "DELETE FROM delegation_child_runs WHERE plan_id IN (SELECT plan_id FROM delegation_plans WHERE project_id=?1)",
        "DELETE FROM delegation_plans WHERE project_id=?1",
        "DELETE FROM saved_method_versions WHERE method_id IN (SELECT method_id FROM saved_methods WHERE project_id=?1)",
        "DELETE FROM saved_methods WHERE project_id=?1",
        "DELETE FROM learning_offers WHERE project_id=?1",
        "DELETE FROM analysis_runs WHERE project_id=?1",
        "UPDATE approval_scope_audit SET grant_id=NULL WHERE grant_id IN (SELECT grant_id FROM reviewed_approval_scopes WHERE project_id=?1)",
        "DELETE FROM reviewed_approval_scopes WHERE project_id=?1",
        "DELETE FROM routine_authority_grants WHERE project_id=?1 OR schedule_id IN (SELECT id FROM workflow_schedules WHERE project_id=?1)",
        "DELETE FROM activation_receipts WHERE project_id=?1",
        "DELETE FROM setup_sample_tasks WHERE project_id=?1",
        "UPDATE setup_progress SET sample_project_id=NULL WHERE sample_project_id=?1",
        "DELETE FROM connector_project_bindings WHERE project_id=?1",
        "DELETE FROM project_policy_decisions WHERE project_id=?1",
        "DELETE FROM project_sources WHERE project_id=?1",
        "DELETE FROM project_instructions WHERE project_id=?1",
        "DELETE FROM project_policy WHERE project_id=?1",
        "UPDATE chat_sessions SET project_id=NULL WHERE project_id=?1",
        "UPDATE workflows SET project_id=NULL WHERE project_id=?1",
        "UPDATE workflow_blueprints SET project_id=NULL WHERE project_id=?1",
        "UPDATE workflow_schedules SET project_id=NULL WHERE project_id=?1",
        "UPDATE execution_instances SET project_id=NULL WHERE project_id=?1",
        "UPDATE agent_executions SET project_id=NULL WHERE project_id=?1",
        "UPDATE message_queue SET project_id=NULL WHERE project_id=?1",
        "UPDATE task_runs SET project_id=NULL WHERE project_id=?1",
        "DELETE FROM projects WHERE project_id=?1",
    ];
    for statement in statements {
        tx.execute(statement, params![project_id])
            .map_err(|error| {
                format!("Project deletion failed while clearing private data: {error}")
            })?;
    }
    Ok(())
}

pub(crate) fn prepare_deletion(
    engine: &PersistenceEngine,
    raw_id: &str,
    app_data: &Path,
) -> Result<ProjectDeletionPlan, String> {
    let id = user_managed_project_id(raw_id)?;
    get(engine, &id)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let owned_paths = collect_project_owned_paths(&connection, app_data, &id)?;
    let user_files_to_delete = owned_paths.iter().try_fold(0usize, |total, path| {
        total
            .checked_add(count_owned_files(path)?)
            .ok_or_else(|| "Project file count overflowed.".to_string())
    })?;
    let preview = ProjectDeletionPreview {
        project_id: id.clone(),
        conversations_to_detach: dependent_count(&connection, "chat_sessions", &id)?,
        workflows_to_detach: dependent_count(&connection, "workflow_blueprints", &id)?,
        schedules_to_detach: dependent_count(&connection, "workflow_schedules", &id)?,
        task_runs_to_detach: dependent_count(&connection, "task_runs", &id)?,
        sources_to_remove: dependent_count(&connection, "project_sources", &id)?,
        user_files_to_delete,
        default_action: "permanent_delete".to_string(),
    };
    Ok(ProjectDeletionPlan {
        preview,
        owned_paths,
    })
}

pub(super) fn deletion_preview(
    engine: &PersistenceEngine,
    raw_id: &str,
    app_data: &Path,
) -> Result<ProjectDeletionPreview, String> {
    Ok(prepare_deletion(engine, raw_id, app_data)?.preview)
}

pub(super) fn delete(
    engine: &PersistenceEngine,
    knowledge: &crate::knowledge::KnowledgeStore,
    memory: &crate::memory_ledger::MemoryLedger,
    app_data: &Path,
    request: DeleteProjectRequest,
) -> Result<ProjectDeletionPreview, String> {
    let plan = prepare_deletion(engine, &request.project_id, app_data)?;
    let preview = plan.preview;
    if !request.permanently_remove_project_record {
        archive(engine, &request.project_id)?;
        return Ok(preview);
    }
    if !request.detach_dependents {
        return Err(
            "Permanent project removal requires explicit dependent detachment.".to_string(),
        );
    }
    if !request.delete_project_files {
        return Err(
            "Permanent project removal requires explicit project-file deletion.".to_string(),
        );
    }
    engine.require_durable_store("delete project")?;
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let tx = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    purge_project_rows(&tx, &preview.project_id)?;
    let staged = stage_project_files(app_data, &preview.project_id, &plan.owned_paths)?;
    if let Err(error) = knowledge.purge_project(&preview.project_id) {
        if let Some(staged) = staged.as_ref() {
            restore_staged_project_files(staged);
        }
        return Err(error.message);
    }
    if let Err(error) = memory.purge_project(&preview.project_id) {
        if let Some(staged) = staged.as_ref() {
            restore_staged_project_files(staged);
        }
        return Err(error.message);
    }
    if let Err(error) = tx.commit().map_err(|error| error.to_string()) {
        if let Some(staged) = staged.as_ref() {
            restore_staged_project_files(staged);
        }
        return Err(error);
    }
    if let Some(staged) = staged {
        fs::remove_dir_all(&staged.trash_root).map_err(|error| {
            format!("Project data was removed, but private-file cleanup needs repair: {error}")
        })?;
    }
    Ok(preview)
}
