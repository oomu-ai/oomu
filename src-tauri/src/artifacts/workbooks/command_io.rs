use super::{repository, WorkbookCommandError, WorkbookPreviewRequest, WorkbookPreviewResponse};
use crate::{
    db::PersistenceEngine,
    foundation::digest::{sha256_file_hex, sha256_hex},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::{fs, io::Write, path::Path};

type CommandResult<T> = Result<T, WorkbookCommandError>;

pub(super) fn load_preview_response(
    engine: &PersistenceEngine,
    workbook_root: &Path,
    request: WorkbookPreviewRequest,
) -> CommandResult<WorkbookPreviewResponse> {
    let stored = repository::preview(engine, &request)
        .map_err(|error| command_error("workbook_preview_unavailable", error))?;
    let canonical_root = fs::canonicalize(workbook_root).map_err(|_| {
        command_error(
            "workbook_preview_unavailable",
            "Private workbook staging is unavailable.",
        )
    })?;
    let path = Path::new(&stored.path);
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        command_error(
            "workbook_preview_unavailable",
            "Workbook preview is unavailable.",
        )
    })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > 16 * 1024 * 1024
    {
        return Err(command_error(
            "workbook_preview_unavailable",
            "Workbook preview failed validation.",
        ));
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| command_error("workbook_preview_unavailable", error.to_string()))?;
    if !canonical.starts_with(canonical_root) {
        return Err(command_error(
            "workbook_preview_unavailable",
            "Workbook preview escaped private staging.",
        ));
    }
    let bytes = fs::read(canonical)
        .map_err(|error| command_error("workbook_preview_unavailable", error.to_string()))?;
    if sha256_hex(&bytes) != stored.sha256 {
        return Err(command_error(
            "workbook_preview_unavailable",
            "Workbook preview digest verification failed.",
        ));
    }
    Ok(WorkbookPreviewResponse {
        artifact_id: request.artifact_id,
        revision: request.revision,
        sheet_id: request.sheet_id,
        mime_type: stored.mime_type.clone(),
        data_url: format!(
            "data:{};base64,{}",
            stored.mime_type,
            STANDARD.encode(bytes)
        ),
        width: stored.width,
        height: stored.height,
        sha256: stored.sha256,
    })
}

pub(super) fn atomic_write_bytes(destination: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "Workbook output has no parent directory.".to_string())?;
    let temporary = parent.join(format!(".oomu-{}.tmp", hex::encode(random_bytes())));
    let mut linked = false;
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
        fs::hard_link(&temporary, destination).map_err(|error| {
            format!("Workbook output refused to replace an existing file: {error}")
        })?;
        linked = true;
        fs::remove_file(&temporary).map_err(|error| error.to_string())?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| error.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        if linked {
            let _ = fs::remove_file(destination);
        }
    }
    result
}

pub(super) fn create_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(super) fn atomic_copy_verified(
    source: &Path,
    destination: &Path,
    expected: &str,
) -> CommandResult<String> {
    let parent = destination.parent().ok_or_else(|| {
        command_error(
            "workbook_export_failed",
            "Export destination has no parent directory.",
        )
    })?;
    let temporary = parent.join(format!(".oomu-export-{}.tmp", hex::encode(random_bytes())));
    let mut linked = false;
    let result = (|| {
        let mut input = fs::File::open(source)
            .map_err(|error| command_error("workbook_export_failed", error.to_string()))?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| command_error("workbook_export_failed", error.to_string()))?;
        std::io::copy(&mut input, &mut output)
            .map_err(|error| command_error("workbook_export_failed", error.to_string()))?;
        output
            .sync_all()
            .map_err(|error| command_error("workbook_export_failed", error.to_string()))?;
        let digest = sha256_file_hex(&temporary)
            .map_err(|error| command_error("workbook_export_failed", error.to_string()))?;
        if digest != expected {
            return Err(command_error(
                "workbook_export_failed",
                "Export digest verification failed.",
            ));
        }
        fs::hard_link(&temporary, destination).map_err(|error| {
            command_error(
                "workbook_export_failed",
                format!("Export refused to overwrite an existing file: {error}"),
            )
        })?;
        linked = true;
        fs::remove_file(&temporary)
            .map_err(|error| command_error("workbook_export_failed", error.to_string()))?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| command_error("workbook_export_failed", error.to_string()))?;
        Ok(digest)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        if linked {
            let _ = fs::remove_file(destination);
        }
    }
    result
}

pub(super) fn validate_private_source(
    workbook_root: &Path,
    source: &Path,
    expected: &str,
) -> CommandResult<()> {
    let canonical_root = fs::canonicalize(workbook_root).map_err(|_| {
        command_error(
            "workbook_export_unavailable",
            "Private workbook storage is unavailable.",
        )
    })?;
    let metadata = fs::symlink_metadata(source).map_err(|_| {
        command_error(
            "workbook_export_unavailable",
            "Private workbook output is unavailable.",
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(command_error(
            "workbook_export_unavailable",
            "Private workbook output failed validation.",
        ));
    }
    let canonical = fs::canonicalize(source)
        .map_err(|error| command_error("workbook_export_unavailable", error.to_string()))?;
    if !canonical.starts_with(canonical_root)
        || sha256_file_hex(&canonical)
            .map_err(|error| command_error("workbook_export_unavailable", error.to_string()))?
            != expected
    {
        return Err(command_error(
            "workbook_export_unavailable",
            "Private workbook output digest or path validation failed.",
        ));
    }
    Ok(())
}

pub(super) struct CleanupDirectory {
    pub(super) path: std::path::PathBuf,
    pub(super) committed: bool,
}

impl Drop for CleanupDirectory {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
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
    fn atomic_writes_are_durable_digest_bound_and_never_overwrite() {
        let root = std::env::temp_dir().join(format!(
            "oomu-workbook-atomic-{}",
            hex::encode(random_bytes())
        ));
        fs::create_dir(&root).unwrap();
        let private = root.join("preview.png");
        atomic_write_bytes(&private, b"preview-bytes").unwrap();
        assert_eq!(
            sha256_file_hex(&private).unwrap(),
            sha256_hex(b"preview-bytes")
        );
        assert!(atomic_write_bytes(&private, b"replacement").is_err());
        assert_eq!(fs::read(&private).unwrap(), b"preview-bytes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&private).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let exported = root.join("export.xlsx");
        let digest = sha256_file_hex(&private).unwrap();
        assert_eq!(
            atomic_copy_verified(&private, &exported, &digest).unwrap(),
            digest
        );
        assert!(atomic_copy_verified(&private, &exported, &digest).is_err());
        assert_eq!(fs::read(&exported).unwrap(), b"preview-bytes");
        fs::remove_dir_all(root).unwrap();
    }
}
