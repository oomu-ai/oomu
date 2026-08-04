use super::VerificationCheck;
use crate::{
    artifacts::{
        exact_package_runtime::{acquire_exact_package_process, ExactPackageSurface},
        ARTIFACT_RENDERER_IDENTITY,
    },
    foundation::digest::sha256_hex,
};
#[path = "exact_package_process.rs"]
mod process;
use lopdf::{Document, Object};
use process::{bounded_message, exit_status_diagnostic, run_bounded, run_qualified_conversion};
use serde::Deserialize;
use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

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
const MAX_PACKAGE_BYTES: usize = 128 * 1024 * 1024;
const MAX_PROTOCOL_BYTES: u64 = 1024 * 1024;
const MAX_RENDERED_PAGES: usize = 4_096;

pub(crate) struct ExactPackageQualification {
    pub renderer_identity: String,
    pub page_count: usize,
    pub check: VerificationCheck,
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

pub(crate) fn qualify_exact_package(package: &[u8]) -> Result<ExactPackageQualification, String> {
    let _converter_guard = acquire_exact_package_process(ExactPackageSurface::Workbook)?;
    if package.is_empty() || package.len() > MAX_PACKAGE_BYTES {
        return Err("The staged workbook package is outside supported bounds.".to_string());
    }
    let engine = resolve_qualified_engine()?;
    let root = std::env::temp_dir().join(format!(
        "oomu-workbook-package-{}",
        hex::encode(random_bytes())
    ));
    create_private_directory(&root)?;
    let _cleanup = Cleanup(root.clone());
    let input = root.join("workbook.xlsx");
    let converted = root.join("converted");
    let profile = root.join("profile");
    let home = root.join("home");
    let temporary = root.join("temporary");
    let rendered = root.join("rendered");
    for directory in [&converted, &profile, &home, &temporary, &rendered] {
        create_private_directory(directory)?;
    }
    write_private_file(&input, package)?;
    let staged = fs::read(&input).map_err(|error| error.to_string())?;
    if sha256_hex(&staged) != sha256_hex(package) {
        return Err("Private workbook staging changed the package bytes.".to_string());
    }
    let profile_url = url::Url::from_directory_path(&profile)
        .map_err(|_| "Private workbook converter profile path is invalid.".to_string())?;
    let profile_argument = OsString::from(format!("-env:UserInstallation={profile_url}"));
    let args = [
        OsString::from("--headless"),
        OsString::from("--nologo"),
        OsString::from("--nodefault"),
        OsString::from("--nofirststartwizard"),
        OsString::from("--nolockcheck"),
        OsString::from("--norestore"),
        profile_argument.clone(),
        OsString::from("--convert-to"),
        OsString::from("pdf:calc_pdf_Export"),
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
    let conversion = run_qualified_conversion(
        &engine.executable,
        QUALIFIED_ENGINE_BRAND,
        QUALIFIED_ENGINE_EXECUTABLE,
        &profile_argument,
        &args,
        &envs,
        Some(&root),
        Duration::from_secs(60),
        MAX_PROTOCOL_BYTES,
    )?;
    if !conversion.status.success() {
        return Err(format!(
            "Qualified workbook conversion failed with {}: stdout: {}; stderr: {}",
            exit_status_diagnostic(conversion.status),
            bounded_message(&conversion.stdout),
            bounded_message(&conversion.stderr),
        ));
    }
    let pdf = guard_single_output(&converted, "workbook.pdf", MAX_PACKAGE_BYTES as u64)?;
    let (page_count, pdf_sha256) = verify_pdf(&pdf)?;
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
    if !probe.status.success() {
        return Err(format!(
            "Packaged PDF renderer probe failed: {}",
            bounded_message(&probe.stderr)
        ));
    }
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
        Duration::from_secs(45),
        MAX_PROTOCOL_BYTES,
    )?;
    if !output.status.success() {
        return Err(format!(
            "Packaged workbook PDF rendering failed: {}",
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
        return Err("Packaged PDF rendering did not verify every workbook page.".to_string());
    }
    let rendered_page_digest = verify_rendered_pages(&rendered, page_count, &manifest.page_files)?;
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
    let check = VerificationCheck {
        code: "exact_package_pages_rendered".to_string(),
        passed: true,
        evidence: format!(
            "Exact XLSX sha256:{} converted to PDF sha256:{}; all {} PDF pages rendered nonblank with page-evidence sha256:{} through {}.",
            sha256_hex(package),
            pdf_sha256,
            page_count,
            rendered_page_digest,
            renderer_identity
        ),
    };
    Ok(ExactPackageQualification {
        renderer_identity,
        page_count,
        check,
    })
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
            .map_err(|_| "Workbook converter digest could not be calculated.".to_string())?;
        let probe = run_bounded(
            &canonical,
            &[OsString::from("--version")],
            &[],
            None,
            Duration::from_secs(5),
            64 * 1024,
        )?;
        if !probe.status.success() || !probe.stderr.is_empty() {
            continue;
        }
        let identity = parse_engine_identity(&probe.stdout, executable_sha256)?;
        return Ok(QualifiedEngine {
            executable: canonical,
            identity,
        });
    }
    Err("No qualified local workbook converter is installed.".to_string())
}

fn qualified_engine_candidates() -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    vec![Path::new("/Applications")
        .join(format!("{QUALIFIED_ENGINE_BRAND}.app"))
        .join("Contents")
        .join("MacOS")
        .join(QUALIFIED_ENGINE_EXECUTABLE)]
}

fn qualified_engine_path(requested: &Path, canonical: &Path, metadata: &fs::Metadata) -> bool {
    let primary_root = Path::new("/Applications")
        .join(format!("{QUALIFIED_ENGINE_BRAND}.app"))
        .join("Contents")
        .join("MacOS");
    if requested.starts_with(&primary_root) {
        return !metadata.file_type().is_symlink() && canonical.starts_with(primary_root);
    }
    false
}

fn parse_engine_identity(
    stdout: &[u8],
    executable_sha256: String,
) -> Result<EngineIdentity, String> {
    let text = std::str::from_utf8(stdout)
        .map_err(|_| "Workbook converter returned a non-UTF-8 identity.".to_string())?
        .trim();
    let fields = text.split_ascii_whitespace().collect::<Vec<_>>();
    let release = QUALIFIED_ENGINE_RELEASES.iter().find(|release| {
        fields.len() == 3
            && fields[0] == QUALIFIED_ENGINE_BRAND
            && fields[1] == release.version
            && fields[2] == release.build_id
            && release
                .executable_digests
                .contains(&executable_sha256.as_str())
    });
    if release.is_none() {
        return Err("Workbook converter version is not qualified by this build.".to_string());
    }
    Ok(EngineIdentity {
        brand: QUALIFIED_ENGINE_BRAND.to_string(),
        version: fields[1].to_string(),
        build_id: fields[2].to_ascii_lowercase(),
        executable_sha256,
    })
}

fn verify_pdf(path: &Path) -> Result<(usize, String), String> {
    let document = Document::load(path)
        .map_err(|error| format!("Converted workbook PDF is invalid: {error}"))?;
    if document.is_encrypted() {
        return Err("Converted workbook PDF is encrypted.".to_string());
    }
    let pages = document.get_pages();
    if pages.is_empty() || pages.len() > MAX_RENDERED_PAGES {
        return Err(format!(
            "Converted workbook PDF contains an unsupported page count: {}.",
            pages.len()
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
            .map_err(|_| format!("Converted workbook PDF page {number} has no content stream."))?;
        if !pdf_contents_are_usable(&document, contents) {
            return Err(format!(
                "Converted workbook PDF page {number} has no usable content stream."
            ));
        }
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    Ok((pages.len(), sha256_hex(&bytes)))
}

fn pdf_contents_are_usable(document: &Document, contents: &Object) -> bool {
    match contents {
        Object::Reference(id) => document
            .get_object(*id)
            .ok()
            .is_some_and(|object| pdf_contents_are_usable(document, object)),
        Object::Array(values) => {
            !values.is_empty()
                && values
                    .iter()
                    .all(|value| pdf_contents_are_usable(document, value))
        }
        Object::Stream(stream) => !stream.content.is_empty(),
        _ => false,
    }
}

fn verify_rendered_pages(
    root: &Path,
    page_count: usize,
    claims: &[String],
) -> Result<String, String> {
    if page_count == 0 || page_count > MAX_RENDERED_PAGES || claims.len() != page_count {
        return Err("PDF page evidence count does not match the workbook PDF.".to_string());
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let mut used = HashSet::new();
    let mut page_hashes = Vec::with_capacity(page_count);
    for (index, claim) in claims.iter().enumerate() {
        let claimed = PathBuf::from(claim);
        let metadata = fs::symlink_metadata(&claimed)
            .map_err(|_| "A rendered workbook page is missing.".to_string())?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > 16 * 1024 * 1024
        {
            return Err("A rendered workbook page failed file validation.".to_string());
        }
        let canonical = fs::canonicalize(&claimed).map_err(|error| error.to_string())?;
        let expected = fs::canonicalize(root.join(format!("page-{:03}.png", index + 1)))
            .map_err(|_| "The PDF renderer omitted a canonical workbook page image.".to_string())?;
        if canonical != expected
            || !canonical.starts_with(&canonical_root)
            || !used.insert(canonical.clone())
        {
            return Err(
                "Workbook PDF page evidence was reordered, reused, or escaped private staging."
                    .to_string(),
            );
        }
        let bytes = fs::read(canonical).map_err(|error| error.to_string())?;
        let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .map_err(|error| format!("Rendered workbook page is invalid: {error}"))?
            .to_luma8();
        if !(300..=4_000).contains(&decoded.width()) || !(200..=4_000).contains(&decoded.height()) {
            return Err("Rendered workbook page dimensions are invalid.".to_string());
        }
        let mut minimum = 255_u8;
        let mut maximum = 0_u8;
        let mut dark = 0_usize;
        for pixel in decoded.pixels() {
            minimum = minimum.min(pixel[0]);
            maximum = maximum.max(pixel[0]);
            dark += usize::from(pixel[0] < 245);
        }
        if maximum.saturating_sub(minimum) < 24 || dark < 50 {
            return Err("Rendered workbook page appears blank or unreadable.".to_string());
        }
        page_hashes.push(sha256_hex(&bytes));
    }
    if fs::read_dir(root)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .count()
        != page_count
    {
        return Err("PDF renderer emitted files outside the workbook page manifest.".to_string());
    }
    Ok(sha256_hex(page_hashes.join(":").as_bytes()))
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
        let candidate = Path::new(crate::OOMU_MANIFEST_DIR)
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
        return Err("Workbook conversion emitted an unexpected output manifest.".to_string());
    }
    let path = root.join(name);
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > limit
    {
        return Err("Converted workbook PDF failed file validation.".to_string());
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let canonical = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !canonical.starts_with(canonical_root) {
        return Err("Converted workbook PDF escaped private staging.".to_string());
    }
    Ok(canonical)
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
mod tests {
    use super::*;
    use image::{GrayImage, ImageFormat, Luma};

    #[test]
    fn rendered_page_evidence_accepts_nonblank_and_rejects_blank_pages() {
        let root = std::env::temp_dir().join(format!(
            "oomu-workbook-page-evidence-{}",
            hex::encode(random_bytes())
        ));
        create_private_directory(&root).unwrap();
        let _cleanup = Cleanup(root.clone());
        let page = root.join("page-001.png");
        let mut nonblank = GrayImage::from_pixel(600, 400, Luma([255]));
        for y in 40..80 {
            for x in 40..160 {
                nonblank.put_pixel(x, y, Luma([0]));
            }
        }
        nonblank.save_with_format(&page, ImageFormat::Png).unwrap();
        let claims = vec![page.to_string_lossy().to_string()];
        assert!(verify_rendered_pages(&root, 1, &claims).is_ok());

        GrayImage::from_pixel(600, 400, Luma([255]))
            .save_with_format(&page, ImageFormat::Png)
            .unwrap();
        assert!(verify_rendered_pages(&root, 1, &claims)
            .unwrap_err()
            .contains("blank or unreadable"));
    }

    #[test]
    fn engine_identity_requires_the_exact_qualified_build_and_digest() {
        let accepted = format!(
            "{QUALIFIED_ENGINE_BRAND} 26.2.4.2 {}\n",
            QUALIFIED_ENGINE_RELEASES[0].build_id
        );
        let identity = parse_engine_identity(
            accepted.as_bytes(),
            QUALIFIED_ENGINE_RELEASES[0].executable_digests[0].to_string(),
        )
        .unwrap();
        assert_eq!(identity.version, "26.2.4.2");
        let current = format!(
            "{QUALIFIED_ENGINE_BRAND} 26.2.5.2 {}\n",
            QUALIFIED_ENGINE_RELEASES[1].build_id
        );
        assert_eq!(
            parse_engine_identity(
                current.as_bytes(),
                QUALIFIED_ENGINE_RELEASES[1].executable_digests[0].to_string(),
            )
            .expect("current qualified engine"),
            EngineIdentity {
                brand: QUALIFIED_ENGINE_BRAND.to_string(),
                version: "26.2.5.2".to_string(),
                build_id: QUALIFIED_ENGINE_RELEASES[1].build_id.to_string(),
                executable_sha256: QUALIFIED_ENGINE_RELEASES[1].executable_digests[0].to_string(),
            }
        );
        let mixed = format!(
            "{QUALIFIED_ENGINE_BRAND} 26.2.4.2 {}\n",
            QUALIFIED_ENGINE_RELEASES[1].build_id
        );
        assert!(parse_engine_identity(
            mixed.as_bytes(),
            QUALIFIED_ENGINE_RELEASES[1].executable_digests[0].to_string(),
        )
        .is_err());
        assert!(parse_engine_identity(accepted.as_bytes(), "0".repeat(64)).is_err());
    }
}
