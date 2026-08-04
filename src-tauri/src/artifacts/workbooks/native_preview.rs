use super::{
    WorkbookIr, WorkbookLocation, WorkbookPreviewEvidence, WorkbookPreviewImage, WorkbookWarning,
    WorkbookWarningCode,
};
use crate::foundation::digest::sha256_hex;
use serde::Deserialize;
use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

const NATIVE_RENDERER: &str = "oomu-artifact-pdf-helper/apple-pdfkit-v1+appkit-sheet-v1";

#[derive(Deserialize)]
struct NativeRenderOutput {
    backend: String,
    sheet_previews: Vec<NativeSheetPreview>,
    warnings: Vec<NativeWarning>,
}

#[derive(Deserialize)]
struct NativeSheetPreview {
    sheet_id: String,
    file: String,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct NativeWarning {
    code: String,
    sheet_id: Option<String>,
    range: Option<String>,
    chart_id: Option<String>,
    technical_detail: String,
}

pub(crate) fn render_native_previews(
    workbook: &WorkbookIr,
) -> Result<(Vec<WorkbookPreviewImage>, Vec<WorkbookWarning>), String> {
    let helper = resolve_helper()
        .ok_or_else(|| "Packaged AppKit sheet renderer is unavailable.".to_string())?;
    let root = env::temp_dir().join(format!(
        "oomu-workbook-native-preview-{}",
        hex::encode(random_bytes())
    ));
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    set_private_directory(&root)?;
    let cleanup = Cleanup(root.clone());
    let input = root.join("workbook.json");
    let output_root = root.join("rendered");
    write_private_file(
        &input,
        &serde_json::to_vec(workbook).map_err(|error| error.to_string())?,
    )?;
    let output = run_bounded(
        &helper,
        &[
            "--render-workbook-preview",
            input
                .to_str()
                .ok_or_else(|| "Private workbook preview path is invalid.".to_string())?,
            output_root
                .to_str()
                .ok_or_else(|| "Private workbook preview path is invalid.".to_string())?,
        ],
        Duration::from_secs(15),
    )?;
    if !output.status.success() {
        return Err(format!(
            "AppKit sheet renderer failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(500)
                .collect::<String>()
        ));
    }
    let parsed: NativeRenderOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("AppKit sheet renderer returned invalid JSON: {error}"))?;
    if parsed.backend != NATIVE_RENDERER || parsed.sheet_previews.len() != workbook.worksheets.len()
    {
        return Err("AppKit sheet renderer identity or sheet count is invalid.".to_string());
    }
    let canonical_root = fs::canonicalize(&output_root).map_err(|error| error.to_string())?;
    let mut images = Vec::new();
    for sheet in &workbook.worksheets {
        let rendered = parsed
            .sheet_previews
            .iter()
            .find(|preview| preview.sheet_id == sheet.sheet_id)
            .ok_or_else(|| format!("AppKit preview for sheet {} is missing.", sheet.sheet_id))?;
        let path = Path::new(&rendered.file);
        let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > 16 * 1024 * 1024
        {
            return Err("AppKit sheet preview file failed validation.".to_string());
        }
        let canonical = fs::canonicalize(path).map_err(|error| error.to_string())?;
        if !canonical.starts_with(&canonical_root) {
            return Err("AppKit sheet preview escaped its private render directory.".to_string());
        }
        let bytes = fs::read(canonical).map_err(|error| error.to_string())?;
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|error| error.to_string())?
            .to_luma8();
        if image.width() != rendered.width
            || image.height() != rendered.height
            || !(300..=4_000).contains(&image.width())
            || !(300..=4_000).contains(&image.height())
        {
            return Err("AppKit sheet preview dimensions are invalid.".to_string());
        }
        let dark = image
            .pixels()
            .filter(|pixel| pixel[0] < 245)
            .take(51)
            .count();
        if dark < 50 {
            return Err("AppKit sheet preview appears blank.".to_string());
        }
        let evidence = WorkbookPreviewEvidence {
            sheet_id: sheet.sheet_id.clone(),
            mime_type: "image/png".into(),
            width: rendered.width,
            height: rendered.height,
            sha256: sha256_hex(&bytes),
        };
        images.push(WorkbookPreviewImage { evidence, bytes });
    }
    let warnings = parsed
        .warnings
        .into_iter()
        .map(map_warning)
        .collect::<Result<Vec<_>, _>>()?;
    drop(cleanup);
    Ok((images, warnings))
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("AppKit sheet renderer startup failed: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "AppKit renderer stdout pipe is unavailable.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "AppKit renderer stderr pipe is unavailable.".to_string())?;
    let out_thread = thread::spawn(move || {
        let mut data = Vec::new();
        stdout.take(LIMIT + 1).read_to_end(&mut data).map(|_| data)
    });
    let err_thread = thread::spawn(move || {
        let mut data = Vec::new();
        stderr.take(LIMIT + 1).read_to_end(&mut data).map(|_| data)
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err("AppKit sheet renderer exceeded its time limit.".to_string());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = out_thread
        .join()
        .map_err(|_| "AppKit renderer stdout reader failed.".to_string())?
        .map_err(|error| error.to_string())?;
    let stderr = err_thread
        .join()
        .map_err(|_| "AppKit renderer stderr reader failed.".to_string())?
        .map_err(|error| error.to_string())?;
    if stdout.len() > LIMIT as usize || stderr.len() > LIMIT as usize {
        return Err("AppKit sheet renderer exceeded its output limit.".to_string());
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
    {
        fs::write(path, bytes).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn native_renderer_available() -> bool {
    let Some(helper) = resolve_helper() else {
        return false;
    };
    run_bounded(
        &helper,
        &["--probe-workbook-renderer"],
        Duration::from_secs(2),
    )
    .is_ok_and(|output| {
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains(NATIVE_RENDERER)
    })
}

fn map_warning(warning: NativeWarning) -> Result<WorkbookWarning, String> {
    let code = match warning.code.as_str() {
        "column_content_clipped" => WorkbookWarningCode::ColumnContentClipped,
        "preview_truncated" => WorkbookWarningCode::PreviewTruncated,
        "chart_data_missing" => WorkbookWarningCode::ChartDataMissing,
        "preview_unavailable" => WorkbookWarningCode::PreviewUnavailable,
        value => {
            return Err(format!(
                "AppKit renderer returned unknown warning code {value}."
            ))
        }
    };
    Ok(WorkbookWarning {
        code,
        location: WorkbookLocation {
            sheet_id: warning.sheet_id,
            range: warning.range,
            chart_id: warning.chart_id,
        },
        technical_detail: warning.technical_detail,
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
        let candidate = Path::new(crate::runtime_profile::OOMU_MANIFEST_DIR)
            .join("binaries")
            .join(format!("oomu-artifact-pdf-helper-{triple}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
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
    use crate::artifacts::workbooks::{deterministic_fixture, CellValue, FormulaResult};

    #[test]
    fn native_renderer_accepts_unicode_cached_text_formula_results() {
        if !native_renderer_available() {
            return;
        }
        let mut workbook = deterministic_fixture().unwrap();
        workbook.worksheets[0]
            .cells
            .iter_mut()
            .find(|cell| cell.address == "A2")
            .unwrap()
            .value = CellValue::Formula {
            expression: "UNICODE_TEXT()".into(),
            cached_value: Some(FormulaResult::Text {
                value: "承認済み".into(),
            }),
        };
        let (images, warnings) = render_native_previews(&workbook).unwrap();
        assert_eq!(images.len(), workbook.worksheets.len());
        assert!(!warnings
            .iter()
            .any(|warning| warning.code == WorkbookWarningCode::PreviewUnavailable));
    }

    #[test]
    fn helper_digest_rejects_an_unpinned_executable() {
        let root = std::env::temp_dir().join(format!(
            "oomu-unpinned-workbook-renderer-{}",
            hex::encode(random_bytes())
        ));
        fs::write(&root, b"not-the-packaged-helper").unwrap();
        assert!(verify_helper_digest(&root).is_err());
        fs::remove_file(root).unwrap();
    }
}
