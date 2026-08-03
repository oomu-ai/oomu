use super::{
    template_inspection::inspect_template, CreateWorkbookFromTemplateRequest,
    InspectWorkbookTemplateRequest, WorkbookCommandError, WorkbookTemplateInspection,
};
use crate::{
    db::PersistenceEngine,
    foundation::digest::{sha256_file_hex, sha256_hex},
    tasks,
};
use rusqlite::params;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tauri::Manager;

const MAX_TEMPLATE_BYTES: u64 = 64 * 1024 * 1024;
const TEMPLATE_TTL_MS: i64 = 30 * 60 * 1_000;

type CommandResult<T> = Result<T, WorkbookCommandError>;

#[tauri::command]
pub async fn inspect_workbook_template(
    request: InspectWorkbookTemplateRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> CommandResult<WorkbookTemplateInspection> {
    let Some(source_handle) = rfd::AsyncFileDialog::new()
        .add_filter("XLSX", &["xlsx"])
        .pick_file()
        .await
    else {
        return Err(command_error(
            "workbook_template_cancelled",
            "Template selection was cancelled.",
        ));
    };
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| command_error("workbook_template_inspection_failed", error.to_string()))?;
    let source = source_handle.path().to_path_buf();
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        inspect_and_stage_template(&engine, &app_data, &source, request)
    })
    .await
    .map_err(|error| command_error("workbook_template_inspection_failed", error.to_string()))?
}

#[tauri::command]
pub async fn create_workbook_from_template(
    request: CreateWorkbookFromTemplateRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> CommandResult<()> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| command_error("workbook_template_invalid", error.to_string()))?;
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        verify_staged_template(&engine, &app_data, &request)?;
        Err(command_error(
            "workbook_template_qualification_required",
            "Template creation remains unavailable until the imported package can be faithfully rendered and verified.",
        ))
    })
    .await
    .map_err(|error| command_error("workbook_template_invalid", error.to_string()))?
}

fn inspect_and_stage_template(
    engine: &PersistenceEngine,
    app_data: &Path,
    source: &Path,
    request: InspectWorkbookTemplateRequest,
) -> CommandResult<WorkbookTemplateInspection> {
    let task = tasks::require_bound_task(engine, &request.task_run_id, &request.project_id)
        .map_err(|error| command_error("workbook_template_context_invalid", error))?;
    if task.task_id != request.task_id {
        return Err(command_error(
            "workbook_template_context_invalid",
            "Task ID does not match the bound Task run.",
        ));
    }
    let bytes = read_template_source(source)?;
    let sheets = inspect_template(&bytes)
        .map_err(|error| command_error("workbook_template_invalid", error))?;
    let source_sha256 = sha256_hex(&bytes);
    let template_token = format!("workbook_template_{}", hex::encode(random_bytes()));
    let staged_path = stage_template(app_data, &template_token, &bytes, &source_sha256)?;
    let now = crate::foundation::clock::unix_time_ms_i64();
    let expires_at_ms = now.saturating_add(TEMPLATE_TTL_MS);
    let sheet_manifest = serde_json::to_string(&sheets)
        .map_err(|error| command_error("workbook_template_inspection_failed", error.to_string()))?;
    let inserted = engine
        .open_connection()
        .map_err(|error| command_error("workbook_template_inspection_failed", error.to_string()))?
        .execute(
            "INSERT INTO workbook_template_imports(template_token,project_id,task_id,task_run_id,source_private_path,source_sha256,source_bytes,sheet_manifest_json,status_code,created_at_ms,expires_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'inspected',?9,?10)",
            params![
                template_token,
                request.project_id,
                request.task_id,
                request.task_run_id,
                staged_path.to_string_lossy(),
                source_sha256,
                bytes.len() as i64,
                sheet_manifest,
                now,
                expires_at_ms,
            ],
        );
    if let Err(error) = inserted {
        let _ = fs::remove_file(&staged_path);
        return Err(command_error(
            "workbook_template_inspection_failed",
            error.to_string(),
        ));
    }
    let source_name = source
        .file_name()
        .map(|value| value.to_string_lossy().chars().take(255).collect())
        .unwrap_or_else(|| "workbook.xlsx".to_string());
    Ok(WorkbookTemplateInspection {
        template_token,
        task_run_id: request.task_run_id,
        source_name,
        source_sha256,
        sheets,
        preview_qualified: false,
        expires_at_ms,
    })
}

fn verify_staged_template(
    engine: &PersistenceEngine,
    app_data: &Path,
    request: &CreateWorkbookFromTemplateRequest,
) -> CommandResult<()> {
    let task = tasks::require_bound_task(engine, &request.task_run_id, &request.project_id)
        .map_err(|error| command_error("workbook_template_context_invalid", error))?;
    if task.task_id != request.task_id {
        return Err(command_error(
            "workbook_template_context_invalid",
            "Task ID does not match the bound Task run.",
        ));
    }
    let now = crate::foundation::clock::unix_time_ms_i64();
    let connection = engine
        .open_connection()
        .map_err(|error| command_error("workbook_template_invalid", error.to_string()))?;
    let (stored_path, stored_sha256, stored_bytes): (String, String, i64) = connection
        .query_row(
            "SELECT source_private_path,source_sha256,source_bytes FROM workbook_template_imports WHERE template_token=?1 AND project_id=?2 AND task_id=?3 AND task_run_id=?4 AND status_code='inspected' AND expires_at_ms>=?5",
            params![request.template_token, request.project_id, request.task_id, request.task_run_id, now],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| {
            command_error(
                "workbook_template_invalid",
                "The inspected template token is unavailable or expired.",
            )
        })?;
    verify_private_template_path(
        app_data,
        Path::new(&stored_path),
        &stored_sha256,
        stored_bytes,
    )?;
    let _ = (
        &request.title,
        &request.locale,
        &request.sheet_name,
        &request.target_range,
        &request.instruction,
        &request.replacement_cells,
    );
    Ok(())
}

fn read_template_source(path: &Path) -> CommandResult<Vec<u8>> {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        != Some("xlsx".to_string())
    {
        return Err(command_error(
            "workbook_template_invalid",
            "Choose an Excel .xlsx template.",
        ));
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| command_error("workbook_template_invalid", error.to_string()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_TEMPLATE_BYTES
    {
        return Err(command_error(
            "workbook_template_invalid",
            "The selected template failed file safety checks.",
        ));
    }
    let input = fs::File::open(path)
        .map_err(|error| command_error("workbook_template_invalid", error.to_string()))?;
    let mut bytes = Vec::with_capacity((metadata.len() as usize).min(1024 * 1024));
    input
        .take(MAX_TEMPLATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| command_error("workbook_template_invalid", error.to_string()))?;
    if bytes.len() as u64 != metadata.len() || bytes.len() as u64 > MAX_TEMPLATE_BYTES {
        return Err(command_error(
            "workbook_template_invalid",
            "The selected template changed or exceeded the size limit while reading.",
        ));
    }
    Ok(bytes)
}

fn stage_template(
    app_data: &Path,
    token: &str,
    bytes: &[u8],
    expected_sha256: &str,
) -> CommandResult<PathBuf> {
    let root = template_import_root(app_data);
    fs::create_dir_all(&root)
        .map_err(|error| command_error("workbook_template_inspection_failed", error.to_string()))?;
    set_directory_private(&root)?;
    let metadata = fs::symlink_metadata(&root)
        .map_err(|error| command_error("workbook_template_inspection_failed", error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(command_error(
            "workbook_template_inspection_failed",
            "Private template staging failed safety checks.",
        ));
    }
    let path = root.join(format!("{token}.xlsx"));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options
        .open(&path)
        .map_err(|error| command_error("workbook_template_inspection_failed", error.to_string()))?;
    let result = output
        .write_all(bytes)
        .and_then(|_| output.sync_all())
        .map_err(|error| command_error("workbook_template_inspection_failed", error.to_string()));
    if let Err(error) = result {
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    if sha256_file_hex(&path)
        .map_err(|error| command_error("workbook_template_inspection_failed", error.to_string()))?
        != expected_sha256
    {
        let _ = fs::remove_file(&path);
        return Err(command_error(
            "workbook_template_inspection_failed",
            "Staged template digest verification failed.",
        ));
    }
    Ok(path)
}

fn verify_private_template_path(
    app_data: &Path,
    path: &Path,
    expected_sha256: &str,
    expected_bytes: i64,
) -> CommandResult<()> {
    let canonical_root = fs::canonicalize(template_import_root(app_data))
        .map_err(|error| command_error("workbook_template_invalid", error.to_string()))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| command_error("workbook_template_invalid", error.to_string()))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_TEMPLATE_BYTES
        || metadata.len() as i64 != expected_bytes
    {
        return Err(command_error(
            "workbook_template_invalid",
            "The staged template failed file safety checks.",
        ));
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| command_error("workbook_template_invalid", error.to_string()))?;
    if !canonical.starts_with(canonical_root)
        || sha256_file_hex(&canonical)
            .map_err(|error| command_error("workbook_template_invalid", error.to_string()))?
            != expected_sha256
    {
        return Err(command_error(
            "workbook_template_invalid",
            "The staged template failed integrity checks.",
        ));
    }
    Ok(())
}

fn template_import_root(app_data: &Path) -> PathBuf {
    app_data.join("workbooks").join("template-imports")
}

#[cfg(unix)]
fn set_directory_private(path: &Path) -> CommandResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| command_error("workbook_template_inspection_failed", error.to_string()))
}

#[cfg(not(unix))]
fn set_directory_private(_path: &Path) -> CommandResult<()> {
    Ok(())
}

fn command_error(code: &str, message: impl Into<String>) -> WorkbookCommandError {
    WorkbookCommandError::new(code, message)
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
    fn template_source_reader_rejects_non_xlsx() {
        let root = std::env::temp_dir().join(format!(
            "oomu-template-source-{}",
            hex::encode(random_bytes())
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("template.xlsm");
        fs::write(&source, b"not-an-xlsx").unwrap();
        assert_eq!(
            read_template_source(&source).unwrap_err().code,
            "workbook_template_invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_staging_is_digest_bound() {
        let root = std::env::temp_dir().join(format!(
            "oomu-template-stage-{}",
            hex::encode(random_bytes())
        ));
        let bytes = b"bounded-template";
        let digest = sha256_hex(bytes);
        let path = stage_template(&root, "workbook_template_test", bytes, &digest).unwrap();
        verify_private_template_path(&root, &path, &digest, bytes.len() as i64).unwrap();
        fs::write(&path, b"tampered-template").unwrap();
        assert_eq!(
            verify_private_template_path(&root, &path, &digest, 17)
                .unwrap_err()
                .code,
            "workbook_template_invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
