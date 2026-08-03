use super::{opaque_id, BrowserDownloadView};
use crate::{foundation::digest::sha256_reader_bounded, native_browser::NativeBrowserDownload};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const UPLOAD_TTL_MS: i64 = 5 * 60 * 1_000;
const MAX_UPLOAD_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct BrowserTransferManager {
    uploads: Arc<Mutex<HashMap<String, UploadGrant>>>,
}

struct UploadGrant {
    session_id: String,
    task_run_id: String,
    file_name: String,
    mime_type: String,
    file: fs::File,
    byte_count: u64,
    expires_at_ms: i64,
}

pub(super) struct UploadPayload {
    pub file_name: String,
    pub mime_type: String,
    pub byte_count: u64,
    pub base64_bytes: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadGrantView {
    pub upload_grant_id: String,
    pub file_name: String,
    pub mime_type: String,
    pub byte_count: u64,
    pub expires_at_ms: i64,
}

impl BrowserTransferManager {
    pub(super) fn issue_upload(
        &self,
        session_id: &str,
        task_run_id: &str,
        path: &Path,
    ) -> Result<UploadGrantView, String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|_| "Selected upload is unavailable.".to_string())?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > MAX_UPLOAD_BYTES
        {
            return Err(
                "Selected upload must be a non-empty regular file no larger than 8 MB.".to_string(),
            );
        }
        let file =
            fs::File::open(path).map_err(|_| "Selected upload could not be opened.".to_string())?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(sanitize_name)
            .ok_or_else(|| "Selected upload filename is invalid.".to_string())?;
        let mime_type = sniff_mime(path, &file_name);
        let upload_grant_id = opaque_id("upload");
        let expires_at_ms = crate::foundation::clock::unix_time_ms_i64() + UPLOAD_TTL_MS;
        self.uploads
            .lock()
            .map_err(|_| "Upload grant store is unavailable.".to_string())?
            .insert(
                upload_grant_id.clone(),
                UploadGrant {
                    session_id: session_id.to_string(),
                    task_run_id: task_run_id.to_string(),
                    file_name: file_name.clone(),
                    mime_type: mime_type.clone(),
                    file,
                    byte_count: metadata.len(),
                    expires_at_ms,
                },
            );
        Ok(UploadGrantView {
            upload_grant_id,
            file_name,
            mime_type,
            byte_count: metadata.len(),
            expires_at_ms,
        })
    }

    pub(super) fn consume_upload(
        &self,
        grant_id: &str,
        session_id: &str,
        task_run_id: &str,
    ) -> Result<UploadPayload, String> {
        let grant = self
            .uploads
            .lock()
            .map_err(|_| "Upload grant store is unavailable.".to_string())?
            .remove(grant_id)
            .ok_or_else(|| "Upload grant is missing, expired, or already consumed.".to_string())?;
        if grant.session_id != session_id
            || grant.task_run_id != task_run_id
            || grant.expires_at_ms < crate::foundation::clock::unix_time_ms_i64()
        {
            return Err("Upload grant scope is invalid or expired.".to_string());
        }
        let mut file = grant.file;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| "Upload file cannot be reread.".to_string())?;
        let mut bytes = Vec::with_capacity(grant.byte_count as usize);
        file.take(MAX_UPLOAD_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Upload file read failed.".to_string())?;
        if bytes.len() as u64 != grant.byte_count || bytes.len() as u64 > MAX_UPLOAD_BYTES {
            return Err("Upload file changed after approval.".to_string());
        }
        Ok(UploadPayload {
            file_name: grant.file_name,
            mime_type: grant.mime_type,
            byte_count: grant.byte_count,
            base64_bytes: STANDARD.encode(bytes),
        })
    }
}

pub(super) fn validate_download(
    record: NativeBrowserDownload,
    quarantine_root: &Path,
) -> Result<(BrowserDownloadView, PathBuf), String> {
    if !record.completed || !record.success {
        let _ = fs::remove_file(&record.private_path);
        return Err("Browser download did not complete successfully.".to_string());
    }
    let canonical_root = fs::canonicalize(quarantine_root)
        .map_err(|_| "Browser quarantine is unavailable.".to_string())?;
    let metadata = fs::symlink_metadata(&record.private_path)
        .map_err(|_| "Downloaded file is unavailable.".to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_DOWNLOAD_BYTES
    {
        let _ = fs::remove_file(&record.private_path);
        return Err("Downloaded file failed quarantine size or type validation.".to_string());
    }
    let canonical = fs::canonicalize(&record.private_path)
        .map_err(|_| "Downloaded file cannot be canonicalized.".to_string())?;
    if !canonical.starts_with(&canonical_root) {
        return Err("Downloaded file escaped quarantine.".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&canonical, fs::Permissions::from_mode(0o600))
            .map_err(|_| "Downloaded file permissions could not be restricted.".to_string())?;
    }
    let file =
        fs::File::open(&canonical).map_err(|_| "Downloaded file cannot be opened.".to_string())?;
    let digest = sha256_reader_bounded(file, MAX_DOWNLOAD_BYTES)
        .map_err(|_| "Downloaded file digest validation failed.".to_string())?
        .ok_or_else(|| "Downloaded file exceeded the digest limit.".to_string())?;
    let mime_type = sniff_mime(&canonical, &record.file_name);
    Ok((
        BrowserDownloadView {
            download_id: record.download_id,
            file_name: record.file_name,
            mime_type,
            byte_count: metadata.len(),
            sha256: digest.to_hex(),
            state: "quarantined".to_string(),
        },
        canonical,
    ))
}

fn sniff_mime(path: &Path, file_name: &str) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .or_else(|| {
            Path::new(file_name)
                .extension()
                .and_then(|value| value.to_str())
        })
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "txt" | "md" | "csv" => "text/plain",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn sanitize_name(value: &str) -> String {
    let value = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .take(120)
        .collect::<String>();
    if value.trim_matches(['.', '_']).is_empty() {
        "upload.bin".to_string()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upload_names_never_preserve_path_material() {
        assert_eq!(
            sanitize_name("../../private report.pdf"),
            ".._.._private_report.pdf"
        );
    }
}
