use super::{CreateFileBrief, CreatedFile};
use crate::foundation::digest::{sha256_file_hex, sha256_hex};
use serde_json::Value;
use std::{fs, io::Write, path::Path};

pub(super) struct CreatedFileEvidence {
    pub(super) canonical_path: String,
    pub(super) format: String,
    pub(super) file_sha256: String,
    pub(super) verified_content_sha256: String,
    pub(super) byte_length: u64,
    pub(super) verification_method: &'static str,
}

pub(super) fn verify_final_created_file(
    brief: &CreateFileBrief,
    result: &CreatedFile,
) -> Result<CreatedFileEvidence, String> {
    let path = Path::new(&result.path);
    let link_metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("OOMU created the file, but couldn't reopen it: {error}"))?;
    if !link_metadata.file_type().is_file() || link_metadata.file_type().is_symlink() {
        return Err("OOMU created the file, but couldn't verify its final file type.".to_string());
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("OOMU created the file, but couldn't verify its path: {error}"))?;
    let expected =
        crate::shield_gate::validate_approved_external_write_target(&brief.destination_path)
            .map_err(|error| error.message)?;
    let expected = fs::canonicalize(&expected).map_err(|error| {
        format!("OOMU created the file, but couldn't verify its destination: {error}")
    })?;
    if canonical != expected {
        return Err(
            "OOMU created the file, but its final path changed during verification.".to_string(),
        );
    }
    let extension = canonical
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| {
            "OOMU created the file, but its extension could not be verified.".to_string()
        })?;
    if extension != brief.format {
        return Err(
            "OOMU created the file, but its final format did not match the request.".to_string(),
        );
    }
    let bytes = fs::read(&canonical).map_err(|error| {
        format!("OOMU created the file, but couldn't verify its contents: {error}")
    })?;
    let final_sha256 = sha256_hex(&bytes);
    if final_sha256 != result.sha256 {
        return Err(
            "OOMU created the file, but its final digest did not match the writer receipt."
                .to_string(),
        );
    }
    let verification_method = if matches!(
        brief.format.as_str(),
        "txt" | "md" | "csv" | "json" | "html" | "xml" | "rtf" | "xls"
    ) {
        if bytes != serialized_text_bytes(brief)? {
            return Err(
                "OOMU created the file, but couldn't verify its requested contents.".to_string(),
            );
        }
        "exact_serialized_bytes"
    } else {
        "production_structural_content_verifier"
    };
    Ok(CreatedFileEvidence {
        canonical_path: canonical.display().to_string(),
        format: brief.format.clone(),
        file_sha256: final_sha256,
        verified_content_sha256: sha256_hex(brief.content.as_bytes()),
        byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        verification_method,
    })
}

pub(super) fn create_verified_text_file(brief: &CreateFileBrief) -> Result<CreatedFile, String> {
    let destination =
        crate::shield_gate::validate_approved_external_write_target(&brief.destination_path)
            .map_err(|error| error.message)?;
    let bytes = serialized_text_bytes(brief)?;
    #[cfg(unix)]
    let mut output = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&destination)
    }
    .map_err(|error| file_create_error(&destination, error))?;
    #[cfg(not(unix))]
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|error| file_create_error(&destination, error))?;
    output
        .write_all(&bytes)
        .and_then(|_| output.sync_all())
        .map_err(|error| format!("OOMU couldn't finish saving this file: {error}"))?;
    let sha256 = sha256_file_hex(&destination).map_err(|error| error.to_string())?;
    Ok(CreatedFile {
        path: destination.display().to_string(),
        sha256,
    })
}

fn serialized_text_bytes(brief: &CreateFileBrief) -> Result<Vec<u8>, String> {
    match brief.format.as_str() {
        "txt" | "md" | "csv" => Ok(brief.content.as_bytes().to_vec()),
        "xls" => super::super::agent_tool_legacy_xls::legacy_xls_bytes(&brief.title, &brief.content),
        "rtf" => Ok(rtf_document(&brief.content).into_bytes()),
        "json" => {
            let value = serde_json::from_str::<Value>(&brief.content)
                .map_err(|_| "The requested JSON content is invalid JSON.".to_string())?;
            serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())
        }
        "html" => Ok(format!(
            "<!doctype html>\n<html lang=\"{}\"><head><meta charset=\"utf-8\"><title>{}</title></head><body><p>{}</p></body></html>\n",
            html_escape(&brief.locale),
            html_escape(&brief.title),
            html_escape(&brief.content).replace('\n', "<br>\n")
        )
        .into_bytes()),
        "xml" => Ok(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<document lang=\"{}\"><title>{}</title><content>{}</content></document>\n",
            xml_escape(&brief.locale),
            xml_escape(&brief.title),
            xml_escape(&brief.content)
        )
        .into_bytes()),
        _ => Err("This format requires a verified document writer.".to_string()),
    }
}

pub(super) fn rtf_document(content: &str) -> String {
    let mut escaped = String::new();
    for character in content.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '{' => escaped.push_str("\\{"),
            '}' => escaped.push_str("\\}"),
            '\n' => escaped.push_str("\\par\n"),
            value if value.is_ascii() => escaped.push(value),
            value => {
                let mut utf16 = [0; 2];
                for unit in value.encode_utf16(&mut utf16).iter() {
                    escaped.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
    format!("{{\\rtf1\\ansi\\ansicpg1252\\deff0 {escaped}}}")
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn xml_escape(value: &str) -> String {
    html_escape(value)
}

fn file_create_error(path: &Path, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        format!(
            "{} already exists. Choose a different file name.",
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("That file")
        )
    } else {
        format!("OOMU couldn't create this file: {error}")
    }
}
