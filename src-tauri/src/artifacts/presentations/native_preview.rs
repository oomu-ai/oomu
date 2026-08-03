use super::{
    ooxml::hex_digest, PresentationIr, PresentationIssueSeverity, PresentationReviewIssue,
};
use serde::Deserialize;
use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(crate) const NATIVE_RENDERER_IDENTITY: &str =
    "oomu-artifact-pdf-helper/apple-pdfkit-v1+appkit-presentation-v1";

#[derive(Clone, Debug)]
pub(crate) struct PresentationPreviewImage {
    pub slide_id: String,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
}

#[derive(Deserialize)]
struct NativeRenderOutput {
    backend: String,
    slide_previews: Vec<NativeSlidePreview>,
    warnings: Vec<NativeWarning>,
}

#[derive(Deserialize)]
struct NativeSlidePreview {
    slide_id: String,
    file: String,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct NativeWarning {
    code: String,
    slide_id: Option<String>,
    object_id: Option<String>,
    technical_detail: String,
}

pub(crate) fn render_native_previews(
    presentation: &PresentationIr,
) -> Result<(Vec<PresentationPreviewImage>, Vec<PresentationReviewIssue>), String> {
    let helper = resolve_helper()
        .ok_or_else(|| "Packaged presentation renderer is unavailable.".to_string())?;
    let root = env::temp_dir().join(format!(
        "oomu-presentation-preview-{}",
        hex::encode(random_bytes())
    ));
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    set_private_directory(&root)?;
    let _cleanup = Cleanup(root.clone());
    let input = root.join("presentation.json");
    let output_root = root.join("rendered");
    write_private_file(
        &input,
        &serde_json::to_vec(presentation).map_err(|error| error.to_string())?,
    )?;
    let output = run_bounded(
        &helper,
        &[
            "--render-presentation-preview",
            input
                .to_str()
                .ok_or_else(|| "Private presentation path is invalid.".to_string())?,
            output_root
                .to_str()
                .ok_or_else(|| "Private presentation path is invalid.".to_string())?,
        ],
        Duration::from_secs(20),
    )?;
    if !output.status.success() {
        return Err(format!(
            "Packaged presentation renderer failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(500)
                .collect::<String>()
        ));
    }
    let parsed: NativeRenderOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Presentation renderer returned invalid JSON: {error}"))?;
    if parsed.backend != NATIVE_RENDERER_IDENTITY
        || parsed.slide_previews.len() != presentation.slides.len()
    {
        return Err("Presentation renderer identity or slide count is invalid.".to_string());
    }
    validate_manifest_claims(presentation, &parsed.slide_previews)?;
    let canonical_root = fs::canonicalize(&output_root).map_err(|error| error.to_string())?;
    let mut images = Vec::new();
    let mut rendered_paths = std::collections::HashSet::new();
    for (index, slide) in presentation.slides.iter().enumerate() {
        let rendered = &parsed.slide_previews[index];
        let path = Path::new(&rendered.file);
        let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > 16 * 1024 * 1024
        {
            return Err("Presentation preview file failed validation.".to_string());
        }
        let canonical = fs::canonicalize(path).map_err(|error| error.to_string())?;
        let expected_path =
            fs::canonicalize(canonical_root.join(format!("slide-{:03}.png", index + 1)))
                .map_err(|_| "Presentation renderer omitted a canonical slide file.".to_string())?;
        if canonical != expected_path
            || !canonical.starts_with(&canonical_root)
            || !rendered_paths.insert(canonical.clone())
        {
            return Err("Presentation preview escaped private staging.".to_string());
        }
        let bytes = fs::read(canonical).map_err(|error| error.to_string())?;
        let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|error| format!("Presentation preview is invalid: {error}"))?;
        if decoded.width() != rendered.width
            || decoded.height() != rendered.height
            || !(300..=4_000).contains(&decoded.width())
            || !(200..=4_000).contains(&decoded.height())
        {
            return Err("Presentation preview dimensions are invalid.".to_string());
        }
        let non_blank = decoded
            .to_luma8()
            .pixels()
            .filter(|pixel| pixel[0] < 248)
            .take(51)
            .count();
        if non_blank < 50 {
            return Err("Presentation preview appears blank.".to_string());
        }
        images.push(PresentationPreviewImage {
            slide_id: slide.slide_id.clone(),
            sha256: hex_digest(&bytes),
            bytes,
            width: rendered.width,
            height: rendered.height,
        });
    }
    let warnings = parsed
        .warnings
        .into_iter()
        .enumerate()
        .map(|(index, warning)| map_warning(presentation.revision, index, warning))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((images, warnings))
}

fn validate_manifest_claims(
    presentation: &PresentationIr,
    previews: &[NativeSlidePreview],
) -> Result<(), String> {
    let expected_ids = presentation
        .slides
        .iter()
        .map(|slide| slide.slide_id.as_str())
        .collect::<Vec<_>>();
    let actual_ids = previews
        .iter()
        .map(|preview| preview.slide_id.as_str())
        .collect::<Vec<_>>();
    let unique_files = previews
        .iter()
        .map(|preview| preview.file.as_str())
        .collect::<std::collections::HashSet<_>>();
    let canonical_names = previews.iter().enumerate().all(|(index, preview)| {
        Path::new(&preview.file)
            .file_name()
            .and_then(|value| value.to_str())
            == Some(format!("slide-{:03}.png", index + 1).as_str())
    });
    if actual_ids != expected_ids || unique_files.len() != previews.len() || !canonical_names {
        return Err(
            "Presentation renderer returned duplicate, reordered, or reused slide evidence."
                .to_string(),
        );
    }
    Ok(())
}

struct BoundedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_bounded(program: &Path, args: &[&str], timeout: Duration) -> Result<BoundedOutput, String> {
    const LIMIT: u64 = 1024 * 1024;
    verify_helper_digest(program)?;
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Presentation renderer startup failed: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Presentation renderer stdout is unavailable.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Presentation renderer stderr is unavailable.".to_string())?;
    let out_thread = thread::spawn(move || {
        let mut data = Vec::new();
        stdout.take(LIMIT + 1).read_to_end(&mut data).map(|_| data)
    });
    let err_thread = thread::spawn(move || {
        let mut data = Vec::new();
        stderr.take(LIMIT + 1).read_to_end(&mut data).map(|_| data)
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Presentation renderer exceeded its time limit.".to_string());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = out_thread
        .join()
        .map_err(|_| "Presentation renderer stdout reader failed.".to_string())?
        .map_err(|error| error.to_string())?;
    let stderr = err_thread
        .join()
        .map_err(|_| "Presentation renderer stderr reader failed.".to_string())?
        .map_err(|error| error.to_string())?;
    if stdout.len() > LIMIT as usize || stderr.len() > LIMIT as usize {
        return Err("Presentation renderer exceeded its output limit.".to_string());
    }
    Ok(BoundedOutput {
        status,
        stdout,
        stderr,
    })
}

fn verify_helper_digest(program: &Path) -> Result<(), String> {
    let expected = option_env!("OOMU_ARTIFACT_PDF_HELPER_SHA256").unwrap_or("unprepared");
    crate::artifacts::runtime::verified_packaged_helper_digest(program, expected).map(|_| ())
}

fn map_warning(
    revision: u32,
    index: usize,
    warning: NativeWarning,
) -> Result<PresentationReviewIssue, String> {
    let severity = match warning.code.as_str() {
        "missing_font"
        | "empty_placeholder"
        | "text_overflow"
        | "contrast_failure"
        | "broken_chart"
        | "missing_asset"
        | "low_resolution_image"
        | "element_overlap"
        | "citation_omission"
        | "preview_unavailable" => PresentationIssueSeverity::Blocker,
        value => {
            return Err(format!(
                "Presentation renderer returned unknown warning {value}."
            ))
        }
    };
    Ok(PresentationReviewIssue {
        issue_id: format!("renderer-{revision}-{index}"),
        revision,
        slide_id: warning.slide_id,
        code: warning.code,
        severity,
        message: warning.technical_detail,
        object_id: warning.object_id,
        evidence_ref: None,
    })
}

fn resolve_helper() -> Option<PathBuf> {
    let filename = if cfg!(windows) {
        "oomu-artifact-pdf-helper.exe"
    } else {
        "oomu-artifact-pdf-helper"
    };
    let sibling = env::current_exe().ok()?.parent()?.join(filename);
    if sibling.is_file() {
        return Some(sibling);
    }
    #[cfg(debug_assertions)]
    {
        let triple = Command::new("rustc")
            .args(["--print", "host-tuple"])
            .output()
            .ok()
            .and_then(|value| String::from_utf8(value.stdout).ok())
            .map(|value| value.trim().to_string())?;
        let candidate = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(format!("oomu-artifact-pdf-helper-{triple}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn set_private_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| error.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
    }
    #[cfg(not(unix))]
    fs::write(path, bytes).map_err(|error| error.to_string())?;
    Ok(())
}

fn random_bytes() -> [u8; 12] {
    use rand_core::{OsRng, RngCore};
    let mut bytes = [0_u8; 12];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_manifest_rejects_duplicate_slide_or_file_claims() {
        let mut presentation =
            crate::artifacts::presentations::deterministic_presentation_fixture();
        let mut second = presentation.slides[0].clone();
        second.slide_id = "slide-2".to_string();
        presentation.slides.push(second);
        let duplicate_id = vec![
            NativeSlidePreview {
                slide_id: "slide-summary".to_string(),
                file: "/private/slide-001.png".to_string(),
                width: 1000,
                height: 563,
            },
            NativeSlidePreview {
                slide_id: "slide-summary".to_string(),
                file: "/private/slide-002.png".to_string(),
                width: 1000,
                height: 563,
            },
        ];
        assert!(validate_manifest_claims(&presentation, &duplicate_id).is_err());
        let reused_file = vec![
            NativeSlidePreview {
                slide_id: "slide-summary".to_string(),
                file: "/private/slide-001.png".to_string(),
                width: 1000,
                height: 563,
            },
            NativeSlidePreview {
                slide_id: "slide-2".to_string(),
                file: "/private/slide-001.png".to_string(),
                width: 1000,
                height: 563,
            },
        ];
        assert!(validate_manifest_claims(&presentation, &reused_file).is_err());
    }
}
