use crate::db::PersistenceEngine;
use rfd::FileDialog;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureBlock {
    pub public_key: String,
    pub signature: String,
    pub payload_hash: String,
    pub signed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogicalCertificate {
    pub premises: Vec<String>,
    pub execution_path: Vec<String>,
    pub formal_conclusion: String,
    pub signature: Option<SignatureBlock>,
}

#[derive(Debug, Deserialize)]
pub struct ExportLogicalCertificateRequest {
    pub default_file_name: Option<String>,
    pub logical_certificate: LogicalCertificate,
}

#[derive(Debug, Serialize)]
pub struct ExportLogicalCertificateResponse {
    pub path: String,
}

#[tauri::command]
pub fn export_logical_certificate(
    request: ExportLogicalCertificateRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Option<ExportLogicalCertificateResponse>, String> {
    persistence.require_durable_store("signed certificate export")?;
    if request.logical_certificate.signature.is_none() {
        return Err("Signed Logical Certificate required for export.".to_string());
    }

    let default_file_name = sanitize_file_name(
        request
            .default_file_name
            .as_deref()
            .unwrap_or("oomu-logical-certificate.mlc.json"),
    );

    let Some(selected_path) = FileDialog::new()
        .set_title("Export Signed Logical Certificate")
        .set_file_name(&default_file_name)
        .add_filter("JSON", &["json"])
        .save_file()
    else {
        return Ok(None);
    };

    let export_path = ensure_json_extension(selected_path);
    let certificate_json = serde_json::to_string_pretty(&request.logical_certificate)
        .map_err(|error| format!("Unable to serialize Logical Certificate: {error}"))?;

    fs::write(&export_path, certificate_json)
        .map_err(|error| format!("Unable to write Logical Certificate export: {error}"))?;

    Ok(Some(ExportLogicalCertificateResponse {
        path: export_path.display().to_string(),
    }))
}

fn ensure_json_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        return path;
    }

    path.with_extension("json")
}

fn sanitize_file_name(input: &str) -> String {
    let sanitized = input
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' => character,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    if sanitized.is_empty() {
        "oomu-logical-certificate.mlc.json".to_string()
    } else {
        sanitized
    }
}
