use crate::sovereign_identity::{SignatureBlock, SovereignIdentity};
use crate::{foundation::digest::sha256_hex, shield_gate::LogicalCertificate};
use base64::{engine::general_purpose, Engine as _};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

const MAX_VISUAL_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_VISUAL_BASE64_PAYLOAD_BYTES: usize = ((MAX_VISUAL_ARTIFACT_BYTES as usize + 2) / 3) * 4;
const MAX_VISUAL_DATA_URL_PREFIX_BYTES: usize = 512;
const MAX_VISUAL_ENCODED_INPUT_BYTES: usize =
    MAX_VISUAL_BASE64_PAYLOAD_BYTES + MAX_VISUAL_DATA_URL_PREFIX_BYTES;
const MAX_VISUAL_DIMENSION: u32 = 8_192;
const MAX_VISUAL_PIXELS: u64 = 40_000_000;
const MAX_PROMPT_CONTEXT_BYTES: usize = 64 * 1024;
const MAX_EXTRACTED_TEXT_ITEMS: usize = 80;
const MAX_CLASSIFICATION_ITEMS: usize = 8;
const MACOS_VISION_TIMEOUT: Duration = Duration::from_secs(25);
const TESSERACT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Deserialize)]
pub struct VisionArtifactRequest {
    #[serde(default, alias = "imagePath")]
    pub image_path: Option<String>,
    #[serde(default, alias = "dataBase64")]
    pub data_base64: Option<String>,
    #[serde(default, alias = "fileName")]
    pub file_name: Option<String>,
    #[serde(default, alias = "mimeType")]
    pub mime_type: Option<String>,
    #[serde(default, alias = "evidenceHint")]
    pub evidence_hint: Option<String>,
}

impl std::fmt::Debug for VisionArtifactRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VisionArtifactRequest")
            .field("renderer_path_supplied", &self.image_path.is_some())
            .field(
                "encoded_attachment_chars",
                &self
                    .data_base64
                    .as_ref()
                    .map(String::len)
                    .unwrap_or_default(),
            )
            .field("file_name_supplied", &self.file_name.is_some())
            .field("mime_type", &self.mime_type)
            .field("evidence_hint_supplied", &self.evidence_hint.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VisionArtifactAnalysis {
    pub image_path: String,
    pub artifact_name: String,
    pub mime_type: String,
    pub backend: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub page_count: Option<usize>,
    pub extracted_text: Vec<String>,
    pub classifications: Vec<String>,
    pub warnings: Vec<String>,
    pub prompt_context: String,
    pub extracted_facts: Vec<String>,
    pub logical_certificate: LogicalCertificate,
    pub signature: SignatureBlock,
}

#[derive(Debug, Clone)]
pub struct VisualPromptContext {
    pub mime_type: String,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct VisionToolError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
}

struct ResolvedVisualArtifact {
    path: PathBuf,
    display_path: String,
    name: String,
    mime_type: String,
    byte_count: u64,
    bytes: Zeroizing<Vec<u8>>,
    _staging: Option<PrivateVisualStaging>,
}

impl std::fmt::Debug for ResolvedVisualArtifact {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedVisualArtifact")
            .field("source", &self.display_path)
            .field("name", &self.name)
            .field("mime_type", &self.mime_type)
            .field("byte_count", &self.byte_count)
            .finish()
    }
}

struct PrivateVisualStaging {
    directory: PathBuf,
}

impl Drop for PrivateVisualStaging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[derive(Debug)]
struct VisualEvidence {
    backend: String,
    width: Option<u32>,
    height: Option<u32>,
    page_count: Option<usize>,
    extracted_text: Vec<String>,
    classifications: Vec<String>,
    warnings: Vec<String>,
    prompt_context: String,
    prompt_context_truncated: bool,
    extracted_facts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AppleVisionOutput {
    #[serde(default)]
    backend: String,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    page_count: Option<usize>,
    #[serde(default)]
    texts: Vec<AppleVisionText>,
    #[serde(default)]
    classifications: Vec<AppleVisionClassification>,
    #[serde(default)]
    warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AppleVisionText {
    text: String,
    #[serde(default)]
    confidence: Option<f32>,
    #[serde(default)]
    page: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AppleVisionClassification {
    label: String,
    #[serde(default)]
    confidence: Option<f32>,
}

#[tauri::command]
pub async fn analyze_visual_artifact(
    request: VisionArtifactRequest,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<VisionArtifactAnalysis, VisionToolError> {
    let identity = identity.inner().clone();
    tauri::async_runtime::spawn_blocking(move || analyze_visual_artifact_sync(request, &identity))
        .await
        .map_err(|error| VisionToolError::io(error.to_string()))?
}

pub fn analyze_visual_artifact_sync(
    request: VisionArtifactRequest,
    identity: &SovereignIdentity,
) -> Result<VisionArtifactAnalysis, VisionToolError> {
    let evidence_hint = request.evidence_hint.clone();
    let artifact = resolve_visual_artifact(request)?;
    let evidence = analyze_resolved_visual_artifact(&artifact, evidence_hint.as_deref())
        .map_err(VisionToolError::invalid)?;

    let mut certificate = LogicalCertificate::unsigned(
        vec![
            format!("artifact_name={}", artifact.name),
            format!("artifact_source={}", artifact.display_path),
            format!("artifact_mime_type={}", artifact.mime_type),
            format!("artifact_byte_count={}", artifact.byte_count),
            format!(
                "artifact_sha256={}",
                sha256_hex(artifact.bytes.as_slice())
            ),
            format!("vision_backend={}", evidence.backend),
            "remote_transport=false".to_string(),
        ],
        vec![
            "The visual artifact was resolved from an explicit attachment or approved local path."
                .to_string(),
            "The artifact type and size were checked before local analysis.".to_string(),
            "Local OCR, PDF text extraction, image metadata, and Apple Vision classification were used when available.".to_string(),
            "The resulting visual evidence was converted into bounded text context for the selected chat model.".to_string(),
        ],
        format!(
            "Visual artifact analysis produced {} text item(s) and {} visual label(s).",
            evidence.extracted_text.len(),
            evidence.classifications.len()
        ),
    );
    let signature = identity
        .sign_certificate_parts(
            &certificate.premises,
            &certificate.execution_path,
            &certificate.formal_conclusion,
        )
        .map_err(|error| VisionToolError {
            code: error.code,
            boundary: error.boundary,
            message: error.message,
        })?;
    certificate.signature = Some(signature.clone());

    Ok(VisionArtifactAnalysis {
        image_path: artifact.display_path,
        artifact_name: artifact.name,
        mime_type: artifact.mime_type,
        backend: evidence.backend,
        width: evidence.width,
        height: evidence.height,
        page_count: evidence.page_count,
        extracted_text: evidence.extracted_text,
        classifications: evidence.classifications,
        warnings: evidence.warnings,
        prompt_context: evidence.prompt_context,
        extracted_facts: evidence.extracted_facts,
        logical_certificate: certificate,
        signature,
    })
}

pub fn is_supported_visual_artifact_path(path: &Path) -> bool {
    visual_extension_for_path(path).is_some()
}

pub fn visual_mime_type_for_path(path: &Path) -> Option<String> {
    normalized_visual_mime_type(path, None)
}

pub fn analyze_visual_bytes_for_context(
    path: &Path,
    bytes: Vec<u8>,
) -> Result<VisualPromptContext, String> {
    let artifact = resolve_picker_visual_bytes(path, bytes)?;
    let mime_type = artifact.mime_type.clone();
    let evidence = analyze_resolved_visual_artifact(&artifact, None)?;
    Ok(VisualPromptContext {
        mime_type,
        text: evidence.prompt_context,
        truncated: evidence.prompt_context_truncated,
    })
}

fn resolve_picker_visual_bytes(
    selected_path: &Path,
    bytes: Vec<u8>,
) -> Result<ResolvedVisualArtifact, String> {
    let bytes = Zeroizing::new(bytes);
    if bytes.len() as u64 > MAX_VISUAL_ARTIFACT_BYTES {
        return Err(format!(
            "Visual artifact is larger than the {} MB analysis limit.",
            MAX_VISUAL_ARTIFACT_BYTES / 1024 / 1024
        ));
    }
    let mime_type = normalized_visual_mime_type(selected_path, None)
        .ok_or_else(|| "Unsupported visual artifact type.".to_string())?;
    validate_visual_dimensions(bytes.as_slice(), &mime_type)?;
    let extension = visual_extension_for_path(selected_path)
        .ok_or_else(|| "Unsupported visual artifact type.".to_string())?;
    let name = artifact_name_from_path(selected_path);
    let (staged_path, staging) = stage_visual_bytes(bytes.as_slice(), extension)?;
    Ok(ResolvedVisualArtifact {
        path: staged_path,
        display_path: "native-picker-grant".to_string(),
        name,
        mime_type,
        byte_count: bytes.len() as u64,
        bytes,
        _staging: Some(staging),
    })
}

fn resolve_visual_artifact(
    request: VisionArtifactRequest,
) -> Result<ResolvedVisualArtifact, VisionToolError> {
    if request
        .image_path
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return Err(VisionToolError::invalid(
            "Renderer-supplied local paths are not accepted. Local files must be selected through a native picker grant."
                .to_string(),
        ));
    }
    if request
        .data_base64
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return resolve_attached_visual_artifact(request);
    }

    Err(VisionToolError::invalid(
        "Visual artifact analysis requires bounded attachment data. Local files must be selected through a native picker grant."
            .to_string(),
    ))
}

fn resolve_attached_visual_artifact(
    request: VisionArtifactRequest,
) -> Result<ResolvedVisualArtifact, VisionToolError> {
    let encoded_input = request.data_base64.as_deref().unwrap_or_default();
    if encoded_input.len() > MAX_VISUAL_ENCODED_INPUT_BYTES {
        return Err(VisionToolError::invalid(
            "Visual artifact encoded payload exceeds local safety limits.".to_string(),
        ));
    }
    let raw_data = encoded_input.trim();
    let data = if raw_data.starts_with("data:") {
        let (prefix, data) = raw_data.split_once(',').ok_or_else(|| {
            VisionToolError::invalid("Visual artifact data URL is invalid.".to_string())
        })?;
        if prefix.len() > MAX_VISUAL_DATA_URL_PREFIX_BYTES {
            return Err(VisionToolError::invalid(
                "Visual artifact data URL metadata exceeds local safety limits.".to_string(),
            ));
        }
        data
    } else {
        raw_data
    };
    if data.len() > MAX_VISUAL_BASE64_PAYLOAD_BYTES
        || padded_base64_decoded_len(data)
            .is_none_or(|decoded_len| decoded_len > MAX_VISUAL_ARTIFACT_BYTES as usize)
    {
        return Err(VisionToolError::invalid(
            "Visual artifact encoded payload exceeds local safety limits.".to_string(),
        ));
    }
    let bytes = Zeroizing::new(general_purpose::STANDARD.decode(data).map_err(|error| {
        VisionToolError::invalid(format!("Invalid base64 attachment: {error}"))
    })?);
    if bytes.is_empty() {
        return Err(VisionToolError::invalid(
            "Visual artifact attachment is empty.".to_string(),
        ));
    }
    if bytes.len() as u64 > MAX_VISUAL_ARTIFACT_BYTES {
        return Err(VisionToolError::invalid(format!(
            "Visual artifact is larger than the {} MB analysis limit.",
            MAX_VISUAL_ARTIFACT_BYTES / 1024 / 1024
        )));
    }

    let name = clean_artifact_name(
        request
            .file_name
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("attached-visual-artifact"),
    );
    let extension = visual_extension_for_name(&name)
        .or_else(|| visual_extension_for_mime(request.mime_type.as_deref().unwrap_or_default()))
        .ok_or_else(|| VisionToolError::invalid("Unsupported visual artifact type.".to_string()))?;
    let mime_type = normalized_visual_mime_type(Path::new(&name), request.mime_type.as_deref())
        .ok_or_else(|| VisionToolError::invalid("Unsupported visual artifact type.".to_string()))?;
    validate_visual_dimensions(bytes.as_slice(), &mime_type).map_err(VisionToolError::invalid)?;
    let (temp_path, staging) = stage_visual_bytes(bytes.as_slice(), extension)
        .map_err(|error| VisionToolError::io(error.to_string()))?;

    Ok(ResolvedVisualArtifact {
        path: temp_path.clone(),
        display_path: format!("attached://{name}"),
        name,
        mime_type,
        byte_count: bytes.len() as u64,
        bytes,
        _staging: Some(staging),
    })
}

fn padded_base64_decoded_len(encoded: &str) -> Option<usize> {
    if encoded.is_empty() || encoded.len() % 4 != 0 || !encoded.is_ascii() {
        return None;
    }
    let padding = if encoded.ends_with("==") {
        2usize
    } else if encoded.ends_with('=') {
        1usize
    } else {
        0usize
    };
    encoded
        .len()
        .checked_div(4)?
        .checked_mul(3)?
        .checked_sub(padding)
}

fn stage_visual_bytes(
    bytes: &[u8],
    extension: &str,
) -> Result<(PathBuf, PrivateVisualStaging), String> {
    let mut staging = None;
    for _ in 0..8 {
        let mut random = [0_u8; 32];
        OsRng.fill_bytes(&mut random);
        let directory = env::temp_dir().join(format!("oomu-visual-{}", hex::encode(random)));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        match builder.create(&directory) {
            Ok(()) => {
                staging = Some(PrivateVisualStaging { directory });
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("Unable to prepare private visual staging.".to_string()),
        }
    }
    let staging =
        staging.ok_or_else(|| "Unable to allocate private visual staging.".to_string())?;
    let path = staging.directory.join(format!("artifact.{extension}"));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&path)
        .map_err(|_| "Unable to create private visual staging file.".to_string())?;
    file.write_all(bytes)
        .map_err(|_| "Unable to stage visual attachment bytes.".to_string())?;
    file.flush()
        .map_err(|_| "Unable to finalize visual attachment staging.".to_string())?;
    Ok((path, staging))
}

fn analyze_resolved_visual_artifact(
    artifact: &ResolvedVisualArtifact,
    evidence_hint: Option<&str>,
) -> Result<VisualEvidence, String> {
    let mut backend = "local-visual-metadata".to_string();
    let (mut width, mut height) = image_dimensions(artifact.bytes.as_slice());
    let mut page_count = None;
    let mut extracted_text = Vec::new();
    let mut classifications = Vec::new();
    let mut warnings = Vec::new();

    if is_pdf_mime(&artifact.mime_type) {
        let (pages, text_items, truncated) = extract_pdf_text(artifact.bytes.as_slice())?;
        backend = "contained-lopdf-pdf-text".to_string();
        page_count = Some(pages);
        extracted_text = text_items;
        if truncated {
            warnings.push(
                "PDF text was truncated at the contained extraction output limit.".to_string(),
            );
        }
    } else {
        match run_local_vision_engine(&artifact.path) {
            Ok(output) => {
                backend = output.backend;
                width = output.width.or(width);
                height = output.height.or(height);
                page_count = output.page_count;
                extracted_text = normalize_vision_text(output.texts);
                classifications = normalize_classifications(output.classifications);
                warnings.extend(
                    output
                        .warnings
                        .into_iter()
                        .filter(|warning| !warning.trim().is_empty()),
                );
            }
            Err(error) => {
                warnings.push(error);
                match run_tesseract_ocr(&artifact.path) {
                    Ok(text_items) if !text_items.is_empty() => {
                        backend = "tesseract-local-ocr-fallback".to_string();
                        extracted_text = text_items;
                    }
                    Ok(_) => warnings
                        .push("Tesseract OCR fallback did not detect readable text.".to_string()),
                    Err(ocr_error) => warnings.push(ocr_error),
                }
            }
        }
    }
    let staged_path = artifact.path.to_string_lossy();
    warnings = warnings
        .into_iter()
        .take(16)
        .map(|warning| {
            crate::redaction::redacted_log_text(
                &warning.replace(staged_path.as_ref(), "[staged-artifact]"),
            )
        })
        .collect();
    backend = truncate_single_line(&crate::redaction::redacted_log_text(&backend), 80);

    let mut extracted_facts = Vec::new();
    extracted_facts.extend(extract_facts_from_text(&artifact.name));
    for text in extracted_text.iter().take(24) {
        extracted_facts.push(format!("ocr_text={}", truncate_single_line(text, 240)));
    }
    for label in classifications.iter().take(MAX_CLASSIFICATION_ITEMS) {
        extracted_facts.push(format!("visual_label={label}"));
    }
    if let Some(hint) = evidence_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let hint_lower = hint.to_lowercase();
        let keep_all = extracted_facts.len() <= 1;
        extracted_facts.retain(|fact| fact.to_lowercase().contains(&hint_lower) || keep_all);
    }
    if extracted_facts.is_empty() {
        extracted_facts.push(format!(
            "visual_artifact_hash={}",
            sha256_hex(artifact.bytes.as_slice())
                .chars()
                .take(16)
                .collect::<String>()
        ));
    }
    extracted_facts.sort();
    extracted_facts.dedup();

    let (prompt_context, prompt_context_truncated) = build_visual_prompt_context(
        artifact,
        &backend,
        width,
        height,
        page_count,
        &extracted_text,
        &classifications,
        &warnings,
    );

    Ok(VisualEvidence {
        backend,
        width,
        height,
        page_count,
        extracted_text,
        classifications,
        warnings,
        prompt_context,
        prompt_context_truncated,
        extracted_facts,
    })
}

fn normalize_vision_text(texts: Vec<AppleVisionText>) -> Vec<String> {
    let mut normalized = Vec::new();
    for item in texts.into_iter().take(MAX_EXTRACTED_TEXT_ITEMS) {
        let text = item.text.trim();
        if text.is_empty() {
            continue;
        }
        let confidence = item.confidence.unwrap_or_default();
        let line = if let Some(page) = item.page {
            format!(
                "page {page}: {text} (confidence {:.0}%)",
                confidence * 100.0
            )
        } else if confidence > 0.0 {
            format!("{text} (confidence {:.0}%)", confidence * 100.0)
        } else {
            text.to_string()
        };
        if !normalized.iter().any(|existing| existing == &line) {
            normalized.push(line);
        }
    }
    normalized
}

fn normalize_classifications(classifications: Vec<AppleVisionClassification>) -> Vec<String> {
    let mut normalized = Vec::new();
    for item in classifications.into_iter().take(MAX_CLASSIFICATION_ITEMS) {
        let label = item.label.trim();
        if label.is_empty() {
            continue;
        }
        let confidence = item.confidence.unwrap_or_default();
        let line = if confidence > 0.0 {
            format!("{label} ({:.0}% confidence)", confidence * 100.0)
        } else {
            label.to_string()
        };
        if !normalized.iter().any(|existing| existing == &line) {
            normalized.push(line);
        }
    }
    normalized
}

fn build_visual_prompt_context(
    artifact: &ResolvedVisualArtifact,
    backend: &str,
    width: Option<u32>,
    height: Option<u32>,
    page_count: Option<usize>,
    extracted_text: &[String],
    classifications: &[String],
    warnings: &[String],
) -> (String, bool) {
    let mut lines = vec![
        format!("Visual analysis for {}", artifact.name),
        format!("Source: {}", artifact.display_path),
        format!("MIME type: {}", artifact.mime_type),
        format!("Byte count: {}", artifact.byte_count),
        format!("Backend: {backend}"),
        "Transport: local-only; no remote image upload was performed by this analyzer.".to_string(),
    ];
    if let (Some(width), Some(height)) = (width, height) {
        lines.push(format!("Dimensions: {width} x {height} px"));
    }
    if let Some(page_count) = page_count {
        lines.push(format!("PDF pages inspected: {page_count}"));
    }

    lines.push(String::new());
    lines.push("Detected text:".to_string());
    if extracted_text.is_empty() {
        lines.push("- No readable text was detected.".to_string());
    } else {
        for text in extracted_text {
            lines.push(format!("- {}", truncate_single_line(text, 1000)));
        }
    }

    lines.push(String::new());
    lines.push("Detected visual content:".to_string());
    if classifications.is_empty() {
        lines.push("- No visual classification labels were returned.".to_string());
    } else {
        for label in classifications {
            lines.push(format!("- {label}"));
        }
    }

    if !warnings.is_empty() {
        lines.push(String::new());
        lines.push("Analysis warnings:".to_string());
        for warning in warnings.iter().take(6) {
            lines.push(format!("- {}", truncate_single_line(warning, 700)));
        }
    }

    truncate_text_at_boundary(&lines.join("\n"), MAX_PROMPT_CONTEXT_BYTES)
}

fn extract_pdf_text(bytes: &[u8]) -> Result<(usize, Vec<String>, bool), String> {
    let extraction = crate::pdf_containment::extract_pdf_bytes_contained(bytes)
        .map_err(|error| error.message)?;
    let text_items = extraction
        .text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(MAX_EXTRACTED_TEXT_ITEMS)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    Ok((extraction.page_count, text_items, extraction.truncated))
}

#[cfg(target_os = "macos")]
fn run_local_vision_engine(path: &Path) -> Result<AppleVisionOutput, String> {
    let current_exe = env::current_exe().map_err(|error| error.to_string())?;
    let helper_path = current_exe
        .parent()
        .map(|directory| directory.join("oomu-vision-helper"))
        .ok_or_else(|| "Unable to resolve Apple Vision helper path.".to_string())?;

    if !helper_path.exists() {
        return Err(format!(
            "Apple Vision helper is missing at {}. Recompile the desktop client.",
            helper_path.display()
        ));
    }

    let child = Command::new(&helper_path)
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Apple Vision helper failed to start: {error}"))?;

    let output = wait_with_timeout(child, MACOS_VISION_TIMEOUT, "Apple Vision helper")?;
    if !output.status.success() {
        return Err(format!(
            "Apple Vision helper failed: {}",
            truncate_single_line(&String::from_utf8_lossy(&output.stderr), 1000)
        ));
    }
    parse_apple_vision_output(&output.stdout)
}

#[cfg(not(target_os = "macos"))]
fn run_local_vision_engine(_path: &Path) -> Result<AppleVisionOutput, String> {
    Err("Apple Vision OCR is only available in the macOS desktop build.".to_string())
}

fn parse_apple_vision_output(stdout: &[u8]) -> Result<AppleVisionOutput, String> {
    let output = serde_json::from_slice::<AppleVisionOutput>(stdout).map_err(|error| {
        format!(
            "Apple Vision helper returned invalid analysis JSON: {error}; stdout={}",
            truncate_single_line(&String::from_utf8_lossy(stdout), 600)
        )
    })?;
    if output.backend.trim().is_empty() {
        return Err("Apple Vision helper omitted its backend identity.".to_string());
    }
    Ok(output)
}

fn run_tesseract_ocr(path: &Path) -> Result<Vec<String>, String> {
    let binary =
        tesseract_binary().ok_or_else(|| "Tesseract OCR fallback is not installed.".to_string())?;
    let staging_directory = path
        .parent()
        .ok_or_else(|| "Visual staging directory is unavailable.".to_string())?;
    let output_base = staging_directory.join("ocr-output");
    let output_text_path = output_base.with_extension("txt");
    let child = Command::new(&binary)
        .arg(path)
        .arg(&output_base)
        .arg("--psm")
        .arg("6")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            format!(
                "Tesseract OCR fallback could not start at {}: {error}",
                binary.display()
            )
        })?;
    let output = wait_with_timeout(child, TESSERACT_TIMEOUT, "Tesseract OCR fallback")?;
    if !output.status.success() {
        let _ = fs::remove_file(&output_text_path);
        return Err(format!(
            "Tesseract OCR fallback failed: {}",
            truncate_single_line(&String::from_utf8_lossy(&output.stderr), 1000)
        ));
    }
    let text = fs::read_to_string(&output_text_path)
        .map_err(|error| format!("Tesseract OCR fallback output was unavailable: {error}"))?;
    let _ = fs::remove_file(output_text_path);
    Ok(ocr_text_lines(&text))
}

fn tesseract_binary() -> Option<PathBuf> {
    if let Some(configured) = env::var_os("OOMU_TESSERACT_PATH").map(PathBuf::from) {
        if configured.exists() {
            return Some(configured);
        }
    }
    for candidate in [
        "/opt/homebrew/bin/tesseract",
        "/usr/local/bin/tesseract",
        "/usr/bin/tesseract",
    ] {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    Some(PathBuf::from("tesseract"))
}

fn ocr_text_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(MAX_EXTRACTED_TEXT_ITEMS)
        .map(ToString::to_string)
        .collect()
}

fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
    label: &str,
) -> Result<Output, String> {
    let started = Instant::now();
    loop {
        match child
            .try_wait()
            .map_err(|error| format!("{label} status failed: {error}"))?
        {
            Some(_) => break,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{label} timed out during local media analysis."));
            }
            None => thread::sleep(Duration::from_millis(50)),
        }
    }

    child
        .wait_with_output()
        .map_err(|error| format!("{label} output failed: {error}"))
}

fn normalized_visual_mime_type(path: &Path, requested_mime: Option<&str>) -> Option<String> {
    let requested = requested_mime
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    if let Some(mime) = requested.as_deref().and_then(visual_mime_for_mime) {
        return Some(mime);
    }
    visual_extension_for_path(path).map(visual_mime_for_extension)
}

fn visual_extension_for_path(path: &Path) -> Option<&'static str> {
    path.extension()
        .and_then(|value| value.to_str())
        .and_then(visual_extension_for_extension)
}

fn visual_extension_for_name(name: &str) -> Option<&'static str> {
    Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .and_then(visual_extension_for_extension)
}

fn visual_extension_for_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        "gif" => Some("gif"),
        "heic" => Some("heic"),
        "heif" => Some("heif"),
        "webp" => Some("webp"),
        "pdf" => Some("pdf"),
        "tif" | "tiff" => Some("tiff"),
        "bmp" => Some("bmp"),
        _ => None,
    }
}

fn visual_extension_for_mime(mime_type: &str) -> Option<&'static str> {
    match mime_type.trim().to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/heic" => Some("heic"),
        "image/heif" => Some("heif"),
        "image/webp" => Some("webp"),
        "application/pdf" => Some("pdf"),
        "image/tiff" | "image/tif" => Some("tiff"),
        "image/bmp" => Some("bmp"),
        _ => None,
    }
}

fn visual_mime_for_mime(mime_type: &str) -> Option<String> {
    visual_extension_for_mime(mime_type).map(visual_mime_for_extension)
}

fn visual_mime_for_extension(extension: &str) -> String {
    match extension {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "tif" | "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn is_pdf_mime(mime_type: &str) -> bool {
    mime_type.eq_ignore_ascii_case("application/pdf")
}

fn artifact_name_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(clean_artifact_name)
        .unwrap_or_else(|| "visual-artifact".to_string())
}

fn clean_artifact_name(value: &str) -> String {
    let file_name = Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(value)
        .trim();
    let cleaned = file_name
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '\0' => '_',
            _ => character,
        })
        .collect::<String>()
        .trim()
        .to_string();
    if cleaned.is_empty() {
        "visual-artifact".to_string()
    } else {
        cleaned
    }
}

fn image_dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    if bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return (
            Some(u32::from_be_bytes([
                bytes[16], bytes[17], bytes[18], bytes[19],
            ])),
            Some(u32::from_be_bytes([
                bytes[20], bytes[21], bytes[22], bytes[23],
            ])),
        );
    }
    if bytes.len() > 4 && bytes.starts_with(&[0xff, 0xd8]) {
        let mut index = 2;
        while index + 9 < bytes.len() {
            if bytes[index] != 0xff {
                index += 1;
                continue;
            }
            let marker = bytes[index + 1];
            let length = u16::from_be_bytes([bytes[index + 2], bytes[index + 3]]) as usize;
            if matches!(marker, 0xc0 | 0xc2) && index + 8 < bytes.len() {
                let height = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]) as u32;
                let width = u16::from_be_bytes([bytes[index + 7], bytes[index + 8]]) as u32;
                return (Some(width), Some(height));
            }
            index += 2 + length;
        }
    }
    if bytes.len() >= 10 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return (
            Some(u16::from_le_bytes([bytes[6], bytes[7]]) as u32),
            Some(u16::from_le_bytes([bytes[8], bytes[9]]) as u32),
        );
    }
    if bytes.len() >= 26 && bytes.starts_with(b"BM") {
        return (
            Some(u32::from_le_bytes([
                bytes[18], bytes[19], bytes[20], bytes[21],
            ])),
            Some(u32::from_le_bytes([
                bytes[22], bytes[23], bytes[24], bytes[25],
            ])),
        );
    }
    if bytes.len() >= 30 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        if &bytes[12..16] == b"VP8X" {
            return (
                Some(1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0])),
                Some(1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0])),
            );
        }
    }
    (None, None)
}

pub(crate) fn validate_visual_dimensions(bytes: &[u8], mime_type: &str) -> Result<(), String> {
    if is_pdf_mime(mime_type) {
        return Ok(());
    }
    let (Some(width), Some(height)) = image_dimensions(bytes) else {
        return Err("Visual artifact dimensions could not be validated.".to_string());
    };
    if width == 0
        || height == 0
        || width > MAX_VISUAL_DIMENSION
        || height > MAX_VISUAL_DIMENSION
        || u64::from(width).saturating_mul(u64::from(height)) > MAX_VISUAL_PIXELS
    {
        return Err("Visual artifact dimensions exceed local safety limits.".to_string());
    }
    Ok(())
}

fn extract_facts_from_text(text: &str) -> Vec<String> {
    let normalized = text.replace(['_', '-'], " ");
    let mut facts = Vec::new();
    for token in normalized.split_whitespace() {
        let lower = token.to_lowercase();
        if lower.starts_with("badge") || lower.starts_with("id") {
            facts.push(format!("badge_or_identifier={}", token.trim_matches(':')));
        }
        if token.contains(':') && token.chars().any(|character| character.is_ascii_digit()) {
            facts.push(format!("timestamp_candidate={token}"));
        }
    }
    facts.sort();
    facts.dedup();
    facts
}

fn truncate_single_line(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let mut truncated = normalized.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn truncate_text_at_boundary(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_string(), false);
    }

    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = text[..end].to_string();
    truncated.push_str("\n[visual analysis truncated]");
    (truncated, true)
}

impl VisionToolError {
    fn invalid(message: String) -> Self {
        Self {
            code: "vision_artifact_invalid",
            boundary: "VisionArtifactTool",
            message,
        }
    }

    fn io(message: String) -> Self {
        Self {
            code: "vision_artifact_io",
            boundary: "VisionArtifactTool",
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_common_visual_artifact_formats() {
        for name in [
            "sample.jpeg",
            "sample.jpg",
            "sample.png",
            "sample.gif",
            "sample.heic",
            "sample.webp",
            "sample.pdf",
        ] {
            assert!(
                is_supported_visual_artifact_path(Path::new(name)),
                "{name} should be supported"
            );
        }
        assert!(!is_supported_visual_artifact_path(Path::new("sample.exe")));
    }

    #[test]
    fn apple_vision_helper_has_no_pdfkit_parser_fallback() {
        let helper_source = include_str!("vision.swift");
        assert!(!helper_source.contains("import PDFKit"));
        assert!(!helper_source.contains("PDFDocument("));
        assert!(!helper_source.contains("analyzePDF"));
        assert!(helper_source.contains("dedicated contained PDF helper"));
    }

    #[test]
    fn apple_vision_output_requires_reported_backend_identity() {
        let error = parse_apple_vision_output(br#"{"texts":[]}"#)
            .expect_err("missing backend identity must not receive a fabricated default");
        assert!(error.contains("omitted its backend identity"));

        let output = parse_apple_vision_output(br#"{"backend":"vision-framework","texts":[]}"#)
            .expect("reported backend identity is accepted");
        assert_eq!(output.backend, "vision-framework");
    }

    #[test]
    fn visual_prompt_context_includes_text_and_labels() {
        let artifact = ResolvedVisualArtifact {
            path: PathBuf::from("/tmp/sample.png"),
            display_path: "attached://sample.png".to_string(),
            name: "sample.png".to_string(),
            mime_type: "image/png".to_string(),
            byte_count: 12,
            bytes: Zeroizing::new(b"not-an-image".to_vec()),
            _staging: None,
        };
        let (context, truncated) = build_visual_prompt_context(
            &artifact,
            "test-backend",
            Some(640),
            Some(480),
            None,
            &["Kiana Allan <kiana@example.com>".to_string()],
            &["screenshot (91% confidence)".to_string()],
            &[],
        );

        assert!(!truncated);
        assert!(context.contains("Visual analysis for sample.png"));
        assert!(context.contains("Kiana Allan <kiana@example.com>"));
        assert!(context.contains("screenshot (91% confidence)"));
    }

    #[test]
    fn attached_visual_artifact_rejects_unsupported_mime() {
        let error = resolve_attached_visual_artifact(VisionArtifactRequest {
            image_path: None,
            data_base64: Some(general_purpose::STANDARD.encode(b"hello")),
            file_name: Some("payload.bin".to_string()),
            mime_type: Some("application/octet-stream".to_string()),
            evidence_hint: None,
        })
        .expect_err("unsupported attachment should fail");

        assert_eq!(error.code, "vision_artifact_invalid");
    }

    #[test]
    fn renderer_visual_request_cannot_read_an_absolute_path() {
        let error = resolve_visual_artifact(VisionArtifactRequest {
            image_path: Some("/Users/alice/private.png".to_string()),
            data_base64: None,
            file_name: None,
            mime_type: Some("image/png".to_string()),
            evidence_hint: None,
        })
        .unwrap_err();
        assert_eq!(error.code, "vision_artifact_invalid");
        assert!(!error.message.contains("/Users/alice"));
        assert!(error.message.contains("native picker grant"));
    }

    #[test]
    fn picker_bytes_use_private_staging_and_ignore_replaced_paths_and_sidecars() {
        let mut random = [0_u8; 16];
        OsRng.fill_bytes(&mut random);
        let root =
            env::temp_dir().join(format!("oomu-vision-boundary-test-{}", hex::encode(random)));
        fs::create_dir(&root).unwrap();
        let selected_path = root.join("selected.png");
        let sidecar_path = root.join("selected.txt");
        let approved_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01".to_vec();
        fs::write(&selected_path, &approved_bytes).unwrap();
        fs::write(&sidecar_path, "ungranted-sidecar-canary").unwrap();

        let artifact = resolve_picker_visual_bytes(&selected_path, approved_bytes.clone()).unwrap();
        let staging_directory = artifact.path.parent().unwrap().to_path_buf();
        assert_ne!(artifact.path, selected_path);
        assert_ne!(staging_directory, root);
        assert_eq!(fs::read(&artifact.path).unwrap(), approved_bytes);
        assert!(!artifact.path.with_extension("txt").exists());

        fs::write(&selected_path, b"replacement-after-grant").unwrap();
        assert_eq!(fs::read(&artifact.path).unwrap(), approved_bytes);
        assert!(!extract_facts_from_text(&artifact.name)
            .join("\n")
            .contains("ungranted-sidecar-canary"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&staging_directory)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&artifact.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        drop(artifact);
        assert!(!staging_directory.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn renderer_attachment_cannot_bypass_pixel_limits() {
        let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        header.extend_from_slice(&20_000_u32.to_be_bytes());
        header.extend_from_slice(&20_000_u32.to_be_bytes());
        let error = resolve_attached_visual_artifact(VisionArtifactRequest {
            image_path: None,
            data_base64: Some(general_purpose::STANDARD.encode(header)),
            file_name: Some("pixel-bomb.png".to_string()),
            mime_type: Some("image/png".to_string()),
            evidence_hint: None,
        })
        .unwrap_err();
        assert_eq!(error.code, "vision_artifact_invalid");
        assert!(error.message.contains("safety limits"));
    }

    #[test]
    fn renderer_attachment_rejects_oversized_encoded_input_before_decode() {
        let error = resolve_attached_visual_artifact(VisionArtifactRequest {
            image_path: None,
            data_base64: Some("A".repeat(MAX_VISUAL_ENCODED_INPUT_BYTES + 1)),
            file_name: Some("oversized.png".to_string()),
            mime_type: Some("image/png".to_string()),
            evidence_hint: None,
        })
        .expect_err("oversized encoded input must fail before decoding");

        assert_eq!(error.code, "vision_artifact_invalid");
        assert!(error.message.contains("encoded payload"));
    }
}
