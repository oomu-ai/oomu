use super::{
    native_preview::PresentationPreviewImage, ooxml::hex_digest, PresentationVerificationCheck,
};
use crate::artifacts::{
    exact_package_runtime::{acquire_exact_package_process, ExactPackageSurface},
    ARTIFACT_RENDERER_IDENTITY,
};
use lopdf::{Document, Object};
use serde::Deserialize;
use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

const QUALIFIED_ENGINE_RELEASE: &str = "26.2.5 (build 26.2.5.2)";
const QUALIFIED_ENGINE_BRAND: &str = "LibreOffice";
const QUALIFIED_ENGINE_EXECUTABLE: &str = "soffice";
const QUALIFIED_ENGINE_RELEASES: [QualifiedEngineRelease; 2] = [
    QualifiedEngineRelease {
        version: "26.2.4.2",
        build_id: "0229ac93fcf0d7cbc6376066c6f35021cef002dc",
        executable_digests: &[
            "407fa38798973075956ed8ce10b2953a1d4661956289bc47d32bf9afbb6ddadd",
            "110fa35389455cf076aae0f041262e7b53f45ac4bdc3ba94220db94e3d1631b2",
        ],
    },
    QualifiedEngineRelease {
        version: "26.2.5.2",
        build_id: "cd7284b4cbbfeb507e630c1aac019f4157393acb",
        executable_digests: &["5a5cfc4621df7a9284c1152b9f1a04dbc71e961b5883b7609ba88079abb97940"],
    },
];
const QUALIFIED_ENGINE_DOWNLOAD_URL: &str =
    "https://www.libreoffice.org/download/download-libreoffice/?lang=en-US&version=26.2.5";
const MAX_PACKAGE_BYTES: usize = 128 * 1024 * 1024;
const MAX_PROTOCOL_BYTES: u64 = 1024 * 1024;
const MAX_RESIDENT_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) struct ExactPackageRender {
    pub previews: Vec<PresentationPreviewImage>,
    pub renderer_identity: String,
    pub check: PresentationVerificationCheck,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct EngineIdentity {
    brand: String,
    version: String,
    build_id: String,
    executable_sha256: String,
}

struct QualifiedEngineRelease {
    version: &'static str,
    build_id: &'static str,
    executable_digests: &'static [&'static str],
}

struct QualifiedEngine {
    executable: PathBuf,
    identity: EngineIdentity,
}

#[derive(Deserialize)]
struct PdfRendererProbe {
    backend: String,
    available: bool,
}

#[derive(Deserialize)]
struct PdfRenderOutput {
    backend: String,
    page_count: usize,
    page_files: Vec<String>,
    warnings: Vec<String>,
}

pub(crate) fn render_exact_package(
    package: &[u8],
    slide_ids: &[String],
) -> Result<ExactPackageRender, String> {
    let _converter_guard = acquire_exact_package_process(ExactPackageSurface::Presentation)?;
    if package.is_empty() || package.len() > MAX_PACKAGE_BYTES {
        return Err("The staged presentation package is outside supported bounds.".to_string());
    }
    if slide_ids.is_empty() || slide_ids.len() > 128 {
        return Err(
            "Exact-package preview supports between one and 128 slides per revision.".to_string(),
        );
    }
    let engine = resolve_qualified_engine()?;
    let root = std::env::temp_dir().join(format!(
        "oomu-presentation-package-{}",
        hex::encode(random_bytes())
    ));
    create_private_directory(&root)?;
    let _cleanup = Cleanup(root.clone());
    let input = root.join("presentation.pptx");
    let converted = root.join("converted");
    let profile = root.join("profile");
    let home = root.join("home");
    let temporary = root.join("temporary");
    let rendered = root.join("rendered");
    for directory in [&converted, &profile, &home, &temporary, &rendered] {
        create_private_directory(directory)?;
    }
    write_private_file(&input, package)?;
    if hex_digest(&fs::read(&input).map_err(|error| error.to_string())?) != hex_digest(package) {
        return Err("Private presentation staging changed the package bytes.".to_string());
    }
    let profile_url = url::Url::from_directory_path(&profile)
        .map_err(|_| "Private converter profile path is invalid.".to_string())?;
    let args = [
        OsString::from("--headless"),
        OsString::from("--nologo"),
        OsString::from("--nodefault"),
        OsString::from("--nofirststartwizard"),
        OsString::from("--nolockcheck"),
        OsString::from("--norestore"),
        OsString::from(format!("-env:UserInstallation={profile_url}")),
        OsString::from("--convert-to"),
        OsString::from("pdf:impress_pdf_Export"),
        OsString::from("--outdir"),
        converted.as_os_str().to_owned(),
        input.as_os_str().to_owned(),
    ];
    let envs = [
        (OsString::from("HOME"), home.as_os_str().to_owned()),
        (OsString::from("TMPDIR"), temporary.as_os_str().to_owned()),
        (OsString::from("LANG"), OsString::from("C")),
        (OsString::from("LC_ALL"), OsString::from("C")),
        (OsString::from("SAL_USE_VCLPLUGIN"), OsString::from("svp")),
    ];
    let conversion = run_bounded(
        &engine.executable,
        &args,
        &envs,
        Some(&root),
        Duration::from_secs(45),
        MAX_PROTOCOL_BYTES,
    )?;
    if !conversion.status.success() {
        return Err(format!(
            "Qualified presentation conversion failed with status {:?}: stdout: {}; stderr: {}",
            conversion.status.code(),
            bounded_message(&conversion.stdout),
            bounded_message(&conversion.stderr),
        ));
    }
    let pdf = guard_single_output(&converted, "presentation.pdf", MAX_PACKAGE_BYTES as u64)?;
    let (page_count, pdf_sha256) = verify_pdf(&pdf, slide_ids.len())?;
    let helper = resolve_pdf_helper()?;
    let helper_sha256 = verify_pdf_helper(&helper)?;
    let probe = run_bounded(
        &helper,
        &[OsString::from("--probe-pdf-renderer")],
        &[],
        Some(&root),
        Duration::from_secs(3),
        64 * 1024,
    )?;
    let probe: PdfRendererProbe = serde_json::from_slice(&probe.stdout)
        .map_err(|error| format!("Packaged PDF renderer probe returned invalid JSON: {error}"))?;
    if !probe.available || probe.backend != ARTIFACT_RENDERER_IDENTITY {
        return Err("Packaged PDF renderer probe returned an invalid identity.".to_string());
    }
    let output = run_bounded(
        &helper,
        &[
            OsString::from("--render-pdf"),
            pdf.as_os_str().to_owned(),
            rendered.as_os_str().to_owned(),
        ],
        &[],
        Some(&root),
        Duration::from_secs(30),
        MAX_PROTOCOL_BYTES,
    )?;
    if !output.status.success() {
        return Err(format!(
            "Packaged PDF rendering failed: {}",
            bounded_message(&output.stderr)
        ));
    }
    let manifest: PdfRenderOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Packaged PDF renderer returned invalid JSON: {error}"))?;
    if manifest.backend != ARTIFACT_RENDERER_IDENTITY
        || manifest.page_count != page_count
        || manifest.page_files.len() != page_count
        || !manifest.warnings.is_empty()
    {
        return Err("Packaged PDF rendering did not verify every presentation page.".to_string());
    }
    let previews = collect_previews(&rendered, slide_ids, &manifest.page_files)?;
    let identity = &engine.identity;
    let renderer_identity = format!(
        "{} {} build {} sha256:{} -> {} sha256:{}",
        identity.brand,
        identity.version,
        identity.build_id,
        identity.executable_sha256,
        ARTIFACT_RENDERER_IDENTITY,
        helper_sha256
    );
    let check = PresentationVerificationCheck {
        code: "exact_package_pages_rendered".to_string(),
        passed: true,
        detail: format!(
            "Exact package sha256:{} converted to PDF sha256:{}; {} of {} pages rendered through {}.",
            hex_digest(package),
            pdf_sha256,
            previews.len(),
            slide_ids.len(),
            renderer_identity
        ),
        slide_id: None,
        object_id: None,
    };
    Ok(ExactPackageRender {
        previews,
        renderer_identity,
        check,
    })
}

pub(crate) fn presentation_checker_probe() -> (bool, bool, bool, bool) {
    let supported = cfg!(target_os = "macos");
    let candidate = supported
        && qualified_engine_candidates()
            .iter()
            .any(|path| fs::symlink_metadata(path).is_ok());
    let engine = supported && resolve_qualified_engine().is_ok();
    let component = engine
        && resolve_pdf_helper()
            .and_then(|path| verify_pdf_helper(&path))
            .is_ok();
    (supported, candidate, engine, component)
}

pub(crate) fn presentation_checker_release() -> &'static str {
    QUALIFIED_ENGINE_RELEASE
}

pub(crate) fn presentation_checker_download_url() -> &'static str {
    QUALIFIED_ENGINE_DOWNLOAD_URL
}

fn resolve_qualified_engine() -> Result<QualifiedEngine, String> {
    for executable in qualified_engine_candidates() {
        let Ok(metadata) = fs::symlink_metadata(&executable) else {
            continue;
        };
        let canonical = fs::canonicalize(&executable).map_err(|error| error.to_string())?;
        let canonical_metadata = fs::metadata(&canonical).map_err(|error| error.to_string())?;
        if !canonical_metadata.is_file()
            || !qualified_engine_path(&executable, &canonical, &metadata)
        {
            continue;
        }
        let executable_sha256 = crate::foundation::digest::sha256_file_hex(&canonical)
            .map_err(|_| "Presentation converter digest could not be calculated.".to_string())?;
        let identity = qualified_engine_identity(executable_sha256)?;
        return Ok(QualifiedEngine {
            executable,
            identity,
        });
    }
    Err("No qualified local presentation converter is installed.".to_string())
}

fn qualified_engine_candidates() -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    vec![
        Path::new("/Applications")
            .join(format!("{QUALIFIED_ENGINE_BRAND}.app"))
            .join("Contents")
            .join("MacOS")
            .join(QUALIFIED_ENGINE_EXECUTABLE),
        Path::new("/opt/homebrew/bin").join(QUALIFIED_ENGINE_EXECUTABLE),
    ]
}

fn qualified_engine_path(requested: &Path, canonical: &Path, metadata: &fs::Metadata) -> bool {
    let primary_root = Path::new("/Applications")
        .join(format!("{QUALIFIED_ENGINE_BRAND}.app"))
        .join("Contents")
        .join("MacOS");
    if requested.starts_with(&primary_root) {
        return !metadata.file_type().is_symlink() && canonical.starts_with(primary_root);
    }
    let fallback =
        Path::new("/opt/homebrew/Caskroom").join(QUALIFIED_ENGINE_BRAND.to_ascii_lowercase());
    requested.starts_with("/opt/homebrew/bin") && canonical.starts_with(fallback)
}

fn qualified_engine_identity(executable_sha256: String) -> Result<EngineIdentity, String> {
    let Some(release) = QUALIFIED_ENGINE_RELEASES.iter().find(|release| {
        release
            .executable_digests
            .contains(&executable_sha256.as_str())
    }) else {
        return Err("Presentation converter version is not qualified by this build.".to_string());
    };
    Ok(EngineIdentity {
        brand: QUALIFIED_ENGINE_BRAND.to_string(),
        version: release.version.to_string(),
        build_id: release.build_id.to_string(),
        executable_sha256,
    })
}

fn verify_pdf(path: &Path, expected_pages: usize) -> Result<(usize, String), String> {
    let document = Document::load(path)
        .map_err(|error| format!("Converted presentation PDF is invalid: {error}"))?;
    if document.is_encrypted() {
        return Err("Converted presentation PDF is encrypted.".to_string());
    }
    let pages = document.get_pages();
    if pages.len() != expected_pages {
        return Err(format!(
            "Converted presentation PDF contains {} pages for {} slides.",
            pages.len(),
            expected_pages
        ));
    }
    for (number, id) in &pages {
        let page = document
            .get_object(*id)
            .map_err(|error| error.to_string())?
            .as_dict()
            .map_err(|error| error.to_string())?;
        let contents = page
            .get(b"Contents")
            .map_err(|_| format!("Converted PDF page {number} has no content stream."))?;
        let references = match contents {
            Object::Reference(id) => vec![*id],
            Object::Array(values) => values
                .iter()
                .filter_map(|value| value.as_reference().ok())
                .collect(),
            _ => Vec::new(),
        };
        if references.is_empty()
            || references.iter().any(|id| {
                document
                    .get_object(*id)
                    .ok()
                    .and_then(|value| value.as_stream().ok())
                    .is_none_or(|stream| stream.content.is_empty())
            })
        {
            return Err(format!(
                "Converted PDF page {number} has no usable content stream."
            ));
        }
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    Ok((pages.len(), hex_digest(&bytes)))
}

pub(crate) fn collect_previews(
    root: &Path,
    slide_ids: &[String],
    claims: &[String],
) -> Result<Vec<PresentationPreviewImage>, String> {
    if claims.len() != slide_ids.len() {
        return Err("PDF page evidence count does not match the slide count.".to_string());
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let mut used = HashSet::new();
    let mut previews = Vec::with_capacity(slide_ids.len());
    for (index, slide_id) in slide_ids.iter().enumerate() {
        let claimed = PathBuf::from(&claims[index]);
        let metadata = fs::symlink_metadata(&claimed)
            .map_err(|_| "A rendered presentation page is missing.".to_string())?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > 16 * 1024 * 1024
        {
            return Err("A rendered presentation page failed file validation.".to_string());
        }
        let canonical = fs::canonicalize(&claimed).map_err(|error| error.to_string())?;
        let expected = fs::canonicalize(root.join(format!("page-{:03}.png", index + 1)))
            .map_err(|_| "The PDF renderer omitted a canonical page image.".to_string())?;
        if canonical != expected
            || !canonical.starts_with(&canonical_root)
            || !used.insert(canonical.clone())
        {
            return Err(
                "PDF page evidence was reordered, reused, or escaped private staging.".to_string(),
            );
        }
        let bytes = fs::read(canonical).map_err(|error| error.to_string())?;
        let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|error| format!("Rendered presentation page is invalid: {error}"))?
            .to_luma8();
        if !(300..=4_000).contains(&decoded.width()) || !(200..=4_000).contains(&decoded.height()) {
            return Err("Rendered presentation page dimensions are invalid.".to_string());
        }
        let mut min = 255u8;
        let mut max = 0u8;
        let mut dark = 0usize;
        for pixel in decoded.pixels() {
            min = min.min(pixel[0]);
            max = max.max(pixel[0]);
            dark += usize::from(pixel[0] < 245);
        }
        if max.saturating_sub(min) < 24 || dark < 50 {
            return Err("Rendered presentation page appears blank or unreadable.".to_string());
        }
        previews.push(PresentationPreviewImage {
            slide_id: slide_id.clone(),
            sha256: hex_digest(&bytes),
            bytes,
            width: decoded.width(),
            height: decoded.height(),
        });
    }
    if fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .count()
        != slide_ids.len()
    {
        return Err("PDF renderer emitted files outside the page manifest.".to_string());
    }
    Ok(previews)
}

fn resolve_pdf_helper() -> Result<PathBuf, String> {
    let filename = if cfg!(windows) {
        "oomu-artifact-pdf-helper.exe"
    } else {
        "oomu-artifact-pdf-helper"
    };
    if let Some(sibling) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(filename)))
        .filter(|path| path.is_file())
    {
        return Ok(sibling);
    }
    #[cfg(debug_assertions)]
    {
        let target = match (std::env::consts::ARCH, std::env::consts::OS) {
            ("aarch64", "macos") => "aarch64-apple-darwin",
            ("x86_64", "macos") => "x86_64-apple-darwin",
            ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
            _ => return Err("No packaged PDF renderer exists for this target.".to_string()),
        };
        let candidate = Path::new(crate::runtime_profile::OOMU_MANIFEST_DIR)
            .join("binaries")
            .join(format!("oomu-artifact-pdf-helper-{target}"));
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("Packaged PDF renderer is unavailable.".to_string())
}

fn verify_pdf_helper(path: &Path) -> Result<String, String> {
    let expected = option_env!("OOMU_ARTIFACT_PDF_HELPER_SHA256").unwrap_or("unprepared");
    crate::artifacts::runtime::verified_packaged_helper_digest(path, expected)
}

fn guard_single_output(root: &Path, name: &str, limit: u64) -> Result<PathBuf, String> {
    let entries = fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    if entries.len() != 1 || entries[0].file_name() != name {
        return Err("Presentation conversion emitted an unexpected output manifest.".to_string());
    }
    let path = root.join(name);
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > limit
    {
        return Err("Converted presentation PDF failed file validation.".to_string());
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let canonical = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !canonical.starts_with(canonical_root) {
        return Err("Converted presentation PDF escaped private staging.".to_string());
    }
    Ok(canonical)
}

struct BoundedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_bounded(
    program: &Path,
    args: &[OsString],
    envs: &[(OsString, OsString)],
    current_dir: Option<&Path>,
    timeout: Duration,
    output_limit: u64,
) -> Result<BoundedOutput, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(envs.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(directory) = current_dir {
        command.current_dir(directory);
    }
    configure_limits(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Qualified rendering process failed to start: {error}"))?;
    let child_pid = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Rendering process stdout is unavailable.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Rendering process stderr is unavailable.".to_string())?;
    let out_thread = thread::spawn(move || {
        let mut data = Vec::new();
        stdout
            .take(output_limit + 1)
            .read_to_end(&mut data)
            .map(|_| data)
    });
    let err_thread = thread::spawn(move || {
        let mut data = Vec::new();
        stderr
            .take(output_limit + 1)
            .read_to_end(&mut data)
            .map(|_| data)
    });
    let started = Instant::now();
    let status = loop {
        if child_resident_bytes(child_pid).is_some_and(|bytes| bytes > MAX_RESIDENT_BYTES) {
            terminate_process(&mut child);
            return Err("Qualified rendering process exceeded its memory limit.".to_string());
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if started.elapsed() >= timeout {
            terminate_process(&mut child);
            return Err("Qualified rendering process exceeded its time limit.".to_string());
        }
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = out_thread
        .join()
        .map_err(|_| "Rendering process stdout reader failed.".to_string())?
        .map_err(|error| error.to_string())?;
    let stderr = err_thread
        .join()
        .map_err(|_| "Rendering process stderr reader failed.".to_string())?
        .map_err(|error| error.to_string())?;
    if stdout.len() > output_limit as usize || stderr.len() > output_limit as usize {
        return Err("Qualified rendering process exceeded its output limit.".to_string());
    }
    Ok(BoundedOutput {
        status,
        stdout,
        stderr,
    })
}

fn bounded_message(bytes: &[u8]) -> String {
    let value = String::from_utf8_lossy(bytes)
        .chars()
        .take(500)
        .collect::<String>();
    if value.trim().is_empty() {
        "no diagnostic output".to_string()
    } else {
        value
    }
}

#[cfg(unix)]
fn configure_limits(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            set_limit(libc::RLIMIT_CORE, 0)?;
            set_limit(libc::RLIMIT_CPU, 50)?;
            set_limit(libc::RLIMIT_FSIZE, MAX_PACKAGE_BYTES as u64)?;
            set_limit(libc::RLIMIT_NOFILE, 256)?;
            Ok(())
        });
    }
}

#[cfg(unix)]
fn set_limit(resource: libc::c_int, value: u64) -> std::io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value as libc::rlim_t,
        rlim_max: value as libc::rlim_t,
    };
    if unsafe { libc::setrlimit(resource as _, &limit) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn configure_limits(_command: &mut Command) {}

fn terminate_process(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(target_os = "macos")]
fn child_resident_bytes(pid: u32) -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    let status = unsafe {
        libc::proc_pid_rusage(
            pid as libc::c_int,
            libc::RUSAGE_INFO_V2,
            usage.as_mut_ptr() as *mut libc::rusage_info_t,
        )
    };
    (status == 0).then(|| unsafe { usage.assume_init() }.ri_phys_footprint)
}

#[cfg(not(target_os = "macos"))]
fn child_resident_bytes(_pid: u32) -> Option<u64> {
    None
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir(path).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())
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
#[path = "exact_package_preview_tests.rs"]
mod tests;
