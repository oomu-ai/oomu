use fs2::available_space;
use std::{
    collections::{BTreeSet, HashMap},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

use super::{
    manifest::CANONICAL_MODEL_ID,
    state::{new_opaque_id, valid_opaque_id, DestinationKind, InstallError},
};

pub const STORAGE_SAFETY_MARGIN_BYTES: u64 = 1024 * 1024 * 1024;
const INSTALLER_DIRECTORY: &str = ".oomu-model-install";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PackageShape {
    Absent,
    Adoptable,
    Invalid,
}

pub(crate) fn probe_package_shape(
    root: &Path,
    manifest: &super::manifest::RecommendedModelManifest,
) -> PackageShape {
    let package = final_directory(root);
    let package_metadata = match fs::symlink_metadata(&package) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return PackageShape::Absent,
        Err(_) => return PackageShape::Invalid,
    };
    if package_metadata.file_type().is_symlink() || !package_metadata.is_dir() {
        return PackageShape::Invalid;
    }
    let entries = match fs::read_dir(&package) {
        Ok(entries) => entries,
        Err(_) => return PackageShape::Invalid,
    };
    let mut actual = BTreeSet::new();
    for entry in entries {
        let Ok(entry) = entry else {
            return PackageShape::Invalid;
        };
        let Ok(name) = entry.file_name().into_string() else {
            return PackageShape::Invalid;
        };
        let Some(asset) = manifest.assets.iter().find(|asset| asset.filename == name) else {
            return PackageShape::Invalid;
        };
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            return PackageShape::Invalid;
        };
        if !super::partial_io::metadata_is_owned_regular_file(&metadata)
            || metadata.len() != asset.bytes
        {
            return PackageShape::Invalid;
        }
        actual.insert(name);
    }
    let expected = manifest
        .assets
        .iter()
        .map(|asset| asset.filename.clone())
        .collect::<BTreeSet<_>>();
    if actual == expected {
        PackageShape::Adoptable
    } else {
        PackageShape::Invalid
    }
}

pub(crate) fn canonical_package_entry_exists(root: &Path) -> bool {
    match fs::symlink_metadata(final_directory(root)) {
        Ok(_) => true,
        Err(error) => error.kind() != io::ErrorKind::NotFound,
    }
}

pub(crate) fn package_identity_sha256(
    root: &Path,
    manifest: &super::manifest::RecommendedModelManifest,
) -> Option<String> {
    if probe_package_shape(root, manifest) != PackageShape::Adoptable {
        return None;
    }
    let package = final_directory(root);
    let mut identity = String::new();
    append_metadata_identity(
        &mut identity,
        "package",
        &fs::symlink_metadata(&package).ok()?,
    );
    for asset in &manifest.assets {
        append_metadata_identity(
            &mut identity,
            &asset.filename,
            &fs::symlink_metadata(package.join(&asset.filename)).ok()?,
        );
    }
    Some(crate::foundation::digest::sha256_hex(identity.as_bytes()))
}

#[cfg(unix)]
fn append_metadata_identity(identity: &mut String, name: &str, metadata: &fs::Metadata) {
    use std::{fmt::Write as _, os::unix::fs::MetadataExt};
    let _ = writeln!(
        identity,
        "{name}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    );
}

#[cfg(not(unix))]
fn append_metadata_identity(identity: &mut String, name: &str, metadata: &fs::Metadata) {
    use std::fmt::Write as _;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let _ = writeln!(identity, "{name}\0{}\0{modified}", metadata.len());
}

#[derive(Clone, Debug)]
pub(crate) struct ValidatedDestination {
    pub root: PathBuf,
    pub kind: DestinationKind,
}

#[derive(Debug)]
pub(crate) struct DestinationGrant {
    pub grant_id: String,
    pub display_path: String,
}

pub struct DestinationAuthority {
    managed_root: PathBuf,
    app_bundle: Option<PathBuf>,
    grants: Mutex<HashMap<String, PathBuf>>,
}

impl DestinationAuthority {
    pub fn new(managed_root: PathBuf) -> Self {
        Self {
            managed_root,
            app_bundle: current_app_bundle(),
            grants: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(managed_root: PathBuf, app_bundle: Option<PathBuf>) -> Self {
        Self {
            managed_root,
            app_bundle,
            grants: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn managed_display_path(&self) -> String {
        self.managed_root.display().to_string()
    }

    pub(crate) fn issue_grant(&self, selected: &Path) -> Result<DestinationGrant, InstallError> {
        let canonical = validate_selected_directory(selected, self.app_bundle.as_deref())?;
        let grant_id = new_opaque_id("model_location_");
        self.grants
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(grant_id.clone(), canonical.clone());
        Ok(DestinationGrant {
            grant_id,
            display_path: canonical.display().to_string(),
        })
    }

    pub(crate) fn resolve(
        &self,
        grant_id: Option<&str>,
        remaining_bytes: u64,
    ) -> Result<ValidatedDestination, InstallError> {
        let (candidate, kind) = match grant_id {
            Some(grant_id) => {
                if !valid_opaque_id(grant_id, "model_location_") {
                    return Err(InstallError::new(
                        "model_install_location_grant_invalid",
                        false,
                        "location grant did not match the native opaque-id contract",
                    ));
                }
                let path = self
                    .grants
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(grant_id)
                    .cloned()
                    .ok_or_else(|| {
                        InstallError::new(
                            "model_install_location_grant_expired",
                            false,
                            "location grant was not found in native authority",
                        )
                    })?;
                (path, DestinationKind::Granted)
            }
            None => (self.managed_root.clone(), DestinationKind::Managed),
        };
        let root =
            prepare_and_validate_root(&candidate, self.app_bundle.as_deref(), remaining_bytes)?;
        Ok(ValidatedDestination { root, kind })
    }

    pub(crate) fn resolve_recovered(
        &self,
        native_approved_root: &Path,
        kind: DestinationKind,
        remaining_bytes: u64,
    ) -> Result<ValidatedDestination, InstallError> {
        let root = prepare_and_validate_root(
            native_approved_root,
            self.app_bundle.as_deref(),
            remaining_bytes,
        )?;
        Ok(ValidatedDestination { root, kind })
    }
}

pub(crate) fn staging_directory(root: &Path, install_id: &str) -> Result<PathBuf, InstallError> {
    if !valid_opaque_id(install_id, "install_") {
        return Err(InstallError::new(
            "model_install_identifier_invalid",
            false,
            "install id failed native validation",
        ));
    }
    Ok(root.join(INSTALLER_DIRECTORY).join(install_id))
}

pub(crate) fn final_directory(root: &Path) -> PathBuf {
    root.join(CANONICAL_MODEL_ID)
}

pub(crate) fn create_staging(root: &Path, install_id: &str) -> Result<PathBuf, InstallError> {
    let installer_root = root.join(INSTALLER_DIRECTORY);
    ensure_owned_directory(&installer_root)?;
    let staging = staging_directory(root, install_id)?;
    if staging.exists() {
        reject_symlink(&staging)?;
        if !staging.is_dir() {
            return Err(InstallError::new(
                "model_install_staging_collision",
                false,
                "installer staging path is not a directory",
            ));
        }
    } else {
        ensure_owned_directory(&staging)?;
    }
    Ok(staging)
}

pub(crate) fn discard_staging(root: &Path, install_id: &str) -> Result<bool, InstallError> {
    let staging = staging_directory(root, install_id)?;
    let installer_root = root.join(INSTALLER_DIRECTORY);
    if !staging.exists() {
        return Ok(false);
    }
    reject_symlink(&installer_root)?;
    reject_symlink(&staging)?;
    let canonical_installer = installer_root.canonicalize().map_err(|error| {
        InstallError::new(
            "model_install_partial_remove_failed",
            true,
            error.to_string(),
        )
    })?;
    let canonical_staging = staging.canonicalize().map_err(|error| {
        InstallError::new(
            "model_install_partial_remove_failed",
            true,
            error.to_string(),
        )
    })?;
    if canonical_staging.parent() != Some(canonical_installer.as_path()) {
        return Err(InstallError::new(
            "model_install_partial_remove_refused",
            false,
            "staging directory escaped installer ownership",
        ));
    }
    fs::remove_dir_all(&canonical_staging).map_err(|error| {
        InstallError::new(
            "model_install_partial_remove_failed",
            true,
            error.to_string(),
        )
    })?;
    Ok(true)
}

pub(crate) fn seal_staging_assets(
    staging: &Path,
    filenames: &[String],
) -> Result<(), InstallError> {
    let canonical_staging = staging.canonicalize().map_err(|error| {
        InstallError::new("model_install_staging_unavailable", true, error.to_string())
    })?;
    for filename in filenames {
        validate_filename(filename)?;
        let partial = canonical_staging.join(format!("{filename}.part"));
        let completed = canonical_staging.join(filename);
        if completed.exists() {
            reject_symlink(&completed)?;
            continue;
        }
        reject_symlink(&partial)?;
        let file = OpenOptions::new()
            .read(true)
            .open(&partial)
            .map_err(|error| {
                InstallError::new("model_install_promotion_failed", true, error.to_string())
            })?;
        file.sync_all().map_err(|error| {
            InstallError::new("model_install_promotion_failed", true, error.to_string())
        })?;
        fs::rename(&partial, &completed).map_err(|error| {
            InstallError::new("model_install_promotion_failed", true, error.to_string())
        })?;
    }
    validate_exact_staging_entries(&canonical_staging, filenames)?;
    sync_directory(&canonical_staging)
}

pub(crate) fn promote_staging(
    root: &Path,
    staging: &Path,
    filenames: &[String],
) -> Result<PathBuf, InstallError> {
    let final_path = final_directory(root);
    if canonical_package_entry_exists(root) {
        return Err(InstallError::new(
            "model_install_destination_collision",
            false,
            "canonical model directory already exists",
        ));
    }
    let canonical_root = root.canonicalize().map_err(|error| {
        InstallError::new(
            "model_install_destination_unavailable",
            true,
            error.to_string(),
        )
    })?;
    let canonical_staging = staging.canonicalize().map_err(|error| {
        InstallError::new("model_install_staging_unavailable", true, error.to_string())
    })?;
    if !canonical_staging.starts_with(canonical_root.join(INSTALLER_DIRECTORY)) {
        return Err(InstallError::new(
            "model_install_promotion_refused",
            false,
            "promotion source escaped installer ownership",
        ));
    }
    validate_exact_staging_entries(&canonical_staging, filenames)?;
    for filename in filenames {
        validate_filename(filename)?;
        let completed = canonical_staging.join(filename);
        reject_symlink(&completed)?;
        let file = OpenOptions::new()
            .read(true)
            .open(&completed)
            .map_err(|error| {
                InstallError::new("model_install_promotion_failed", true, error.to_string())
            })?;
        file.sync_all().map_err(|error| {
            InstallError::new("model_install_promotion_failed", true, error.to_string())
        })?;
    }
    sync_directory(&canonical_staging)?;
    fs::rename(&canonical_staging, &final_path).map_err(|error| {
        InstallError::new("model_install_promotion_failed", true, error.to_string())
    })?;
    sync_directory(&canonical_root)?;
    Ok(final_path)
}

pub(crate) fn validate_exact_staging_entries(
    staging: &Path,
    filenames: &[String],
) -> Result<(), InstallError> {
    let expected = filenames.iter().cloned().collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(staging).map_err(|error| {
        InstallError::new("model_install_staging_unavailable", true, error.to_string())
    })? {
        let entry = entry.map_err(|error| {
            InstallError::new("model_install_staging_unavailable", true, error.to_string())
        })?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            InstallError::new("model_install_staging_unavailable", true, error.to_string())
        })?;
        if !super::partial_io::metadata_is_owned_regular_file(&metadata) {
            return Err(InstallError::new(
                "model_install_staging_not_exact",
                false,
                "staging contained a non-regular or multiply linked entry",
            ));
        }
        let name = entry.file_name().into_string().map_err(|_| {
            InstallError::new(
                "model_install_staging_not_exact",
                false,
                "staging contained a non-Unicode entry",
            )
        })?;
        actual.insert(name);
    }
    if actual != expected {
        return Err(InstallError::new(
            "model_install_staging_not_exact",
            false,
            "staging did not contain exactly the release-controlled assets",
        ));
    }
    Ok(())
}

pub(crate) fn validate_filename(filename: &str) -> Result<(), InstallError> {
    let mut components = Path::new(filename).components();
    if filename.is_empty()
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || filename
            .chars()
            .any(|character| matches!(character, '/' | '\0' | '\\'))
    {
        return Err(InstallError::new(
            "model_install_manifest_invalid",
            false,
            "manifest filename was not a single native path component",
        ));
    }
    Ok(())
}

fn validate_selected_directory(
    selected: &Path,
    app_bundle: Option<&Path>,
) -> Result<PathBuf, InstallError> {
    if !selected.is_absolute() || !selected.is_dir() {
        return Err(InstallError::new(
            "model_install_location_invalid",
            false,
            "selected destination is not an absolute directory",
        ));
    }
    reject_symlink_ancestry(selected)?;
    let canonical = selected.canonicalize().map_err(|error| {
        InstallError::new(
            "model_install_location_unavailable",
            false,
            error.to_string(),
        )
    })?;
    reject_app_bundle_alias(&canonical, app_bundle)?;
    Ok(canonical)
}

fn prepare_and_validate_root(
    candidate: &Path,
    app_bundle: Option<&Path>,
    remaining_bytes: u64,
) -> Result<PathBuf, InstallError> {
    if !candidate.is_absolute() {
        return Err(InstallError::new(
            "model_install_location_invalid",
            false,
            "destination was not absolute",
        ));
    }
    if candidate.exists() && !candidate.is_dir() {
        return Err(InstallError::new(
            "model_install_location_invalid",
            false,
            "destination aliases a non-directory",
        ));
    }
    reject_symlink_ancestry(candidate)?;
    fs::create_dir_all(candidate).map_err(|error| {
        InstallError::new(
            "model_install_location_unavailable",
            false,
            error.to_string(),
        )
    })?;
    let canonical = candidate.canonicalize().map_err(|error| {
        InstallError::new(
            "model_install_location_unavailable",
            false,
            error.to_string(),
        )
    })?;
    reject_app_bundle_alias(&canonical, app_bundle)?;
    verify_writable(&canonical)?;
    let required = remaining_bytes
        .checked_add(STORAGE_SAFETY_MARGIN_BYTES)
        .ok_or_else(|| {
            InstallError::new(
                "model_install_storage_calculation_failed",
                false,
                "required storage overflowed u64",
            )
        })?;
    if remaining_bytes > 0 {
        let available = available_space(&canonical).map_err(|error| {
            InstallError::new("model_install_storage_unavailable", true, error.to_string())
        })?;
        if available < required {
            return Err(InstallError::new(
                "model_install_insufficient_storage",
                false,
                format!("available={available}, required={required}"),
            ));
        }
    }
    Ok(canonical)
}

fn verify_writable(root: &Path) -> Result<(), InstallError> {
    let probe = root.join(format!(".oomu-write-probe-{}", new_opaque_id("probe_")));
    let result = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe)
        .and_then(|mut file| file.write_all(b"oomu").and_then(|_| file.sync_all()));
    let _ = fs::remove_file(&probe);
    result.map_err(|error| {
        InstallError::new(
            "model_install_location_not_writable",
            false,
            error.to_string(),
        )
    })
}

fn ensure_owned_directory(path: &Path) -> Result<(), InstallError> {
    fs::create_dir_all(path).map_err(|error| {
        InstallError::new("model_install_staging_unavailable", true, error.to_string())
    })?;
    reject_symlink(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            InstallError::new("model_install_staging_unavailable", true, error.to_string())
        })?;
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), InstallError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        InstallError::new("model_install_path_unavailable", false, error.to_string())
    })?;
    if metadata.file_type().is_symlink() {
        return Err(InstallError::new(
            "model_install_symlink_refused",
            false,
            "installer-owned path is a symlink",
        ));
    }
    Ok(())
}

fn reject_symlink_ancestry(path: &Path) -> Result<(), InstallError> {
    let mut cursor = Some(path);
    while let Some(candidate) = cursor {
        if candidate.exists() {
            reject_symlink(candidate)?;
        }
        cursor = candidate.parent();
    }
    Ok(())
}

fn reject_app_bundle_alias(
    canonical_root: &Path,
    app_bundle: Option<&Path>,
) -> Result<(), InstallError> {
    if let Some(bundle) = app_bundle {
        let canonical_bundle = bundle
            .canonicalize()
            .unwrap_or_else(|_| bundle.to_path_buf());
        if canonical_root.starts_with(&canonical_bundle) {
            return Err(InstallError::new(
                "model_install_application_bundle_refused",
                false,
                "destination aliases the running application bundle",
            ));
        }
    }
    Ok(())
}

fn current_app_bundle() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|executable| {
        executable
            .ancestors()
            .find(|path| path.extension().is_some_and(|extension| extension == "app"))
            .map(Path::to_path_buf)
    })
}

fn sync_directory(path: &Path) -> Result<(), InstallError> {
    OpenOptions::new()
        .read(true)
        .open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            InstallError::new("model_install_promotion_failed", true, error.to_string())
        })
}
