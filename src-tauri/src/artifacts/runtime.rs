use super::{ArtifactDocument, ARTIFACT_BUILDER_IDENTITY};
use crate::foundation::digest::sha256_file_hex;
use serde::Deserialize;
use std::{
    collections::HashMap,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const MAX_OUTPUT_BYTES: u64 = 48 * 1024 * 1024;
const BUILD_TIMEOUT: Duration = Duration::from_secs(20);
const HELPER_INTEGRITY_MANIFEST: &str = "oomu-helper-integrity.json";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelperResponse {
    protocol_version: u16,
    builder_identity: String,
    docx_file: String,
    pdf_file: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelperIntegrityManifest {
    schema_version: u16,
    helpers: HashMap<String, String>,
}

pub(super) struct BuiltFiles {
    pub docx_path: PathBuf,
    pub pdf_path: PathBuf,
    pub helper_digest: String,
}

pub(super) fn probe_builder() -> Result<(), String> {
    let helper =
        resolve_helper().ok_or_else(|| "Packaged artifact builder is unavailable.".to_string())?;
    let digest = sha256_file_hex(&helper)
        .map_err(|_| "Artifact builder digest could not be calculated.".to_string())?;
    let expected = option_env!("OOMU_ARTIFACT_HELPER_SHA256").unwrap_or("unprepared");
    verify_helper_digest(&helper, expected, &digest)?;
    let output = Command::new(helper)
        .arg("--probe")
        .env_clear()
        .output()
        .map_err(|error| format!("Artifact builder startup probe failed: {error}"))?;
    if !output.status.success()
        || !String::from_utf8_lossy(&output.stdout).contains(ARTIFACT_BUILDER_IDENTITY)
    {
        return Err("Artifact builder startup probe returned an invalid identity.".to_string());
    }
    Ok(())
}

pub(super) fn rebuild_pdf_with_packaged_renderer(
    document: &ArtifactDocument,
    pdf_path: &Path,
    staging: &Path,
) -> Result<String, String> {
    let helper = resolve_renderer_helper()
        .ok_or_else(|| "Packaged artifact PDF builder is unavailable.".to_string())?;
    let helper_digest = sha256_file_hex(&helper)
        .map_err(|_| "Artifact PDF builder digest could not be calculated.".to_string())?;
    let expected = option_env!("OOMU_ARTIFACT_PDF_HELPER_SHA256").unwrap_or("unprepared");
    verify_helper_digest(&helper, expected, &helper_digest)?;
    let input = staging.join("artifact-document.json");
    fs::write(
        &input,
        serde_json::to_vec(document).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Unable to stage ArtifactDocument for PDF build: {error}"))?;
    let mut command = Command::new(&helper);
    command
        .env_clear()
        .current_dir(staging)
        .arg("--build-artifact-pdf")
        .arg(&input)
        .arg(pdf_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    configure_limits(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Artifact PDF builder failed to start: {error}"))?;
    let started = Instant::now();
    let child_pid = child.id();
    loop {
        if child_resident_bytes(child_pid).is_some_and(|bytes| bytes > 512 * 1024 * 1024) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Artifact PDF builder exceeded its memory limit.".to_string());
        }
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) if status.success() => break,
            Some(_) => return Err("Artifact PDF builder failed.".to_string()),
            None if started.elapsed() < BUILD_TIMEOUT => thread::sleep(Duration::from_millis(10)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Artifact PDF builder exceeded its wall-time limit.".to_string());
            }
        }
    }
    let _ = fs::remove_file(input);
    guard_output(staging, "artifact.pdf")?;
    Ok(helper_digest)
}

pub(super) fn build_contained(
    document: &ArtifactDocument,
    staging: &Path,
) -> Result<BuiltFiles, String> {
    fs::create_dir_all(staging)
        .map_err(|error| format!("Unable to create private artifact staging: {error}"))?;
    let helper =
        resolve_helper().ok_or_else(|| "Packaged artifact builder is unavailable.".to_string())?;
    let helper_digest = sha256_file_hex(&helper)
        .map_err(|_| "Artifact builder digest could not be calculated.".to_string())?;
    let expected = option_env!("OOMU_ARTIFACT_HELPER_SHA256").unwrap_or("unprepared");
    verify_helper_digest(&helper, expected, &helper_digest)?;
    let request = serde_json::to_vec(&serde_json::json!({"protocolVersion":1,"document":document}))
        .map_err(|error| error.to_string())?;
    let mut command = Command::new(&helper);
    command
        .env_clear()
        .current_dir(staging)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_limits(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Artifact builder failed to start: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "Artifact builder stdin is unavailable.".to_string())?
        .write_all(&request)
        .map_err(|error| format!("Artifact builder request failed: {error}"))?;
    let started = Instant::now();
    let child_pid = child.id();
    loop {
        if child_resident_bytes(child_pid).is_some_and(|bytes| bytes > 512 * 1024 * 1024) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Artifact builder exceeded its memory limit.".to_string());
        }
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(_) => break,
            None if started.elapsed() < BUILD_TIMEOUT => thread::sleep(Duration::from_millis(10)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Artifact builder exceeded its wall-time limit.".to_string());
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if output.stdout.len() > 64 * 1024 {
        return Err("Artifact builder response exceeded its protocol limit.".to_string());
    }
    if !output.status.success() {
        let message = serde_json::from_slice::<serde_json::Value>(&output.stdout)
            .ok()
            .and_then(|value| value.get("error")?.as_str().map(str::to_string))
            .unwrap_or_else(|| "Artifact builder failed.".to_string());
        return Err(message);
    }
    let response: HelperResponse = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Artifact builder returned invalid protocol data: {error}"))?;
    if response.protocol_version != 1
        || response.builder_identity != ARTIFACT_BUILDER_IDENTITY
        || response.docx_file != "artifact.docx"
        || response.pdf_file != "artifact.pdf"
    {
        return Err("Artifact builder identity or output contract is invalid.".to_string());
    }
    let docx_path = guard_output(staging, &response.docx_file)?;
    let pdf_path = guard_output(staging, &response.pdf_file)?;
    let total = fs::metadata(&docx_path)
        .map_err(|error| error.to_string())?
        .len()
        .saturating_add(
            fs::metadata(&pdf_path)
                .map_err(|error| error.to_string())?
                .len(),
        );
    if total == 0 || total > MAX_OUTPUT_BYTES {
        return Err("Artifact builder outputs exceeded the private staging limit.".to_string());
    }
    let file_count = fs::read_dir(staging)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().ok().is_some_and(|kind| kind.is_file()))
        .count();
    if file_count > 2 {
        return Err("Artifact builder exceeded its file-count limit.".to_string());
    }
    Ok(BuiltFiles {
        docx_path,
        pdf_path,
        helper_digest,
    })
}

fn verify_helper_digest(
    helper: &Path,
    compiled_expected: &str,
    actual: &str,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if let Some(expected) = signed_bundle_helper_digest(helper)? {
            return (expected == actual).then_some(()).ok_or_else(|| {
                "Artifact helper does not match the signed app bundle.".to_string()
            });
        }
        if !cfg!(debug_assertions) {
            return Err("Signed artifact helper identity is unavailable.".to_string());
        }
    }

    if compiled_expected != "unprepared" && actual == compiled_expected {
        return Ok(());
    }

    if cfg!(debug_assertions)
        && env::current_exe()
            .ok()
            .is_some_and(|executable| is_known_helper_sibling(helper, &executable))
    {
        return Ok(());
    }

    if compiled_expected == "unprepared" {
        return Err("Packaged artifact helper digest is unavailable.".to_string());
    }

    // A debug Tauri bundle compiles every Cargo binary and can place that freshly
    // built helper next to the app, while build.rs records the digest of the
    // prepared external binary. Both are built from this checkout, but debug
    // symbols make their bytes differ. That exception is restricted to a known
    // helper beside the running debug executable; arbitrary regular files never
    // inherit it. Developer builds still execute each helper's identity probe.
    Err("Artifact builder digest does not match the packaged identity.".to_string())
}

fn is_known_helper_sibling(helper: &Path, executable: &Path) -> bool {
    let known_name = helper
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            matches!(
                value,
                "artifact_build_helper"
                    | "artifact_build_helper.exe"
                    | "oomu-artifact-pdf-helper"
                    | "oomu-artifact-pdf-helper.exe"
            )
        });
    if !known_name {
        return false;
    }
    let Ok(helper) = fs::canonicalize(helper) else {
        return false;
    };
    let Ok(executable) = fs::canonicalize(executable) else {
        return false;
    };
    helper.parent() == executable.parent()
}

pub(crate) fn verified_packaged_helper_digest(
    helper: &Path,
    compiled_expected: &str,
) -> Result<String, String> {
    let metadata = fs::symlink_metadata(helper)
        .map_err(|_| "Packaged artifact helper identity is unavailable.".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Packaged artifact helper failed file identity checks.".to_string());
    }
    let actual = sha256_file_hex(helper)
        .map_err(|_| "Packaged artifact helper digest could not be calculated.".to_string())?;
    verify_helper_digest(helper, compiled_expected, &actual)?;
    Ok(actual)
}

#[cfg(target_os = "macos")]
fn signed_bundle_helper_digest(helper: &Path) -> Result<Option<String>, String> {
    let helper_name = helper
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Artifact helper name is invalid.".to_string())?;
    let macos = helper
        .parent()
        .ok_or_else(|| "Artifact helper location is invalid.".to_string())?;
    let contents = macos
        .parent()
        .filter(|path| path.file_name().and_then(|value| value.to_str()) == Some("Contents"));
    let Some(contents) = contents else {
        return Ok(None);
    };
    let manifest_path = contents.join("Resources").join(HELPER_INTEGRITY_MANIFEST);
    if !manifest_path.is_file() {
        return Ok(None);
    }
    let manifest: HelperIntegrityManifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .map_err(|_| "Signed artifact helper identity could not be read.".to_string())?,
    )
    .map_err(|_| "Signed artifact helper identity is invalid.".to_string())?;
    if manifest.schema_version != 1 {
        return Err("Signed artifact helper identity is invalid.".to_string());
    }
    let digest = manifest
        .helpers
        .get(helper_name)
        .filter(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .cloned()
        .ok_or_else(|| "Signed artifact helper identity is incomplete.".to_string())?;
    Ok(Some(digest))
}

fn guard_output(root: &Path, name: &str) -> Result<PathBuf, String> {
    let path = root.join(name);
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| format!("Artifact builder omitted {name}."))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_OUTPUT_BYTES
    {
        return Err(format!("Artifact builder output {name} failed validation."));
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let canonical = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !canonical.starts_with(canonical_root) {
        return Err("Artifact builder output escaped private staging.".to_string());
    }
    Ok(canonical)
}
fn resolve_helper() -> Option<PathBuf> {
    let filename = if cfg!(windows) {
        "artifact_build_helper.exe"
    } else {
        "artifact_build_helper"
    };
    let sibling = env::current_exe().ok()?.parent()?.join(filename);
    if sibling.is_file() {
        return Some(sibling);
    }
    #[cfg(debug_assertions)]
    {
        // Cargo test harnesses run from target/debug/deps, so their executable
        // does not share a directory with the plain debug helper. Prefer the
        // prepared target-triple sidecar whose digest build.rs compiled into
        // this binary; the target/debug fallback below remains only for local
        // developer runs that have not prepared a sidecar yet.
        let triple = Command::new("rustc")
            .args(["--print", "host-tuple"])
            .output()
            .ok()
            .and_then(|value| String::from_utf8(value.stdout).ok())
            .map(|value| value.trim().to_string());
        if let Some(triple) = triple {
            let prepared = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("binaries")
                .join(format!(
                    "artifact_build_helper-{triple}{}",
                    if cfg!(windows) { ".exe" } else { "" }
                ));
            if prepared.is_file() {
                return Some(prepared);
            }
        }
        let debug = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/debug")
            .join(filename);
        if debug.is_file() {
            return Some(debug);
        }
    }
    None
}
fn resolve_renderer_helper() -> Option<PathBuf> {
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

#[cfg(unix)]
fn configure_limits(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            set_limit(libc::RLIMIT_CORE, 0)?;
            set_limit(libc::RLIMIT_CPU, 10)?;
            #[cfg(not(target_os = "macos"))]
            set_limit(libc::RLIMIT_AS, 512 * 1024 * 1024)?;
            set_limit(libc::RLIMIT_FSIZE, MAX_OUTPUT_BYTES)?;
            set_limit(libc::RLIMIT_NOFILE, 32)?;
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
    if unsafe { libc::setrlimit(resource as _, &limit) } != 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(unix))]
fn configure_limits(_command: &mut Command) {}

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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::artifacts::{
        ArtifactBlock, ArtifactDocument, ArtifactMetadata, ArtifactSection, PageControls,
        ParagraphStyle, ThemeTokens, ARTIFACT_DOCUMENT_SCHEMA_VERSION,
    };
    use crate::foundation::digest::sha256_file_hex;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn isolated_test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{label}-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn debug_helper_exception_accepts_only_a_known_canonical_sibling() {
        let root = isolated_test_root("oomu-debug-helper-sibling");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let executable = root.join("oomu");
        let helper = root.join("oomu-artifact-pdf-helper");
        let unknown = root.join("untrusted-helper");
        let displaced = nested.join("oomu-artifact-pdf-helper");
        for path in [&executable, &helper, &unknown, &displaced] {
            fs::write(path, b"test executable").unwrap();
        }

        assert!(is_known_helper_sibling(&helper, &executable));
        assert!(!is_known_helper_sibling(&unknown, &executable));
        assert!(!is_known_helper_sibling(&displaced, &executable));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn regular_file_with_an_unpinned_digest_is_rejected() {
        let root = isolated_test_root("oomu-unpinned-artifact-helper");
        fs::create_dir(&root).unwrap();
        let helper = root.join("random-regular-file");
        fs::write(&helper, b"not-the-packaged-helper").unwrap();

        let error = verified_packaged_helper_digest(&helper, "compiled-helper-digest").unwrap_err();
        assert_eq!(
            error,
            "Artifact builder digest does not match the packaged identity."
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn signed_bundle_manifest_accepts_only_the_exact_post_sign_helper() {
        let root = std::env::temp_dir().join(format!(
            "oomu-helper-integrity-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let macos = root.join("OOMU.app/Contents/MacOS");
        let resources = root.join("OOMU.app/Contents/Resources");
        fs::create_dir_all(&macos).unwrap();
        fs::create_dir_all(&resources).unwrap();
        let helper = macos.join("oomu-artifact-pdf-helper");
        fs::write(&helper, b"signed-helper-bytes").unwrap();
        let digest = sha256_file_hex(&helper).unwrap();
        fs::write(
            resources.join(HELPER_INTEGRITY_MANIFEST),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "helpers": {"oomu-artifact-pdf-helper": digest}
            }))
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            verified_packaged_helper_digest(&helper, "pre-sign-digest").unwrap(),
            digest
        );
        fs::write(&helper, b"tampered-helper-bytes").unwrap();
        let error = verified_packaged_helper_digest(&helper, "pre-sign-digest").unwrap_err();
        assert_eq!(
            error,
            "Artifact helper does not match the signed app bundle."
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn packaged_renderer_paginates_oversized_paragraph_without_losing_its_tail() {
        let root = isolated_test_root("oomu-artifact-pdf-pagination");
        fs::create_dir_all(&root).unwrap();
        let mut lines = (1..=180)
            .map(|index| {
                format!(
                    "Evidence row {index}: verified operational detail remains visible in the final PDF."
                )
            })
            .collect::<Vec<_>>();
        lines.push("Required final evidence marker alpha omega.".to_string());
        let document = ArtifactDocument {
            schema_version: ARTIFACT_DOCUMENT_SCHEMA_VERSION,
            metadata: ArtifactMetadata {
                title: "Paginated Operations Brief".into(),
                subtitle: String::new(),
                author: "OOMU".into(),
                subject: "Native pagination regression".into(),
                keywords: vec!["pagination".into()],
                language: "en-US".into(),
            },
            theme: ThemeTokens::default(),
            page: PageControls::default(),
            header: None,
            footer: None,
            sections: vec![ArtifactSection {
                heading: "Paginated Operations Brief".into(),
                page_break_before: false,
                blocks: vec![ArtifactBlock::Paragraph {
                    text: lines.join("\n"),
                    style: ParagraphStyle::Body,
                    factual: false,
                    sources: Vec::new(),
                }],
            }],
        };
        let pdf = root.join("artifact.pdf");

        rebuild_pdf_with_packaged_renderer(&document, &pdf, &root).unwrap();
        let page_count = super::super::verifier::verify_pdf(&document, &pdf).unwrap();

        assert!(page_count >= 4, "expected a real multi-page PDF");
        fs::remove_dir_all(root).unwrap();
    }
}
