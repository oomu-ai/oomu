use std::{
    fs::{self, File},
    io,
    path::Path,
};
use subtle::ConstantTimeEq;

use super::{manifest::RecommendedModelAsset, state::InstallError};

pub(crate) async fn verify_asset_file(
    asset: &RecommendedModelAsset,
    path: &Path,
) -> Result<(), InstallError> {
    let file = secure_open_regular(path, false).map_err(|error| {
        InstallError::new("model_install_asset_unavailable", true, error.to_string())
    })?;
    let metadata = file.metadata().map_err(|error| {
        InstallError::new("model_install_asset_unavailable", true, error.to_string())
    })?;
    if metadata.len() != asset.bytes {
        return Err(InstallError::new(
            "model_install_size_mismatch",
            false,
            "completed asset did not match the immutable byte length",
        ));
    }
    let actual = tokio::task::spawn_blocking(move || {
        crate::foundation::digest::sha256_reader(file).map(|digest| digest.to_hex())
    })
    .await
    .map_err(|error| InstallError::new("model_install_hash_failed", true, error.to_string()))?
    .map_err(|error| InstallError::new("model_install_hash_failed", true, error.to_string()))?;
    let expected_bytes = hex::decode(&asset.sha256).map_err(|error| {
        InstallError::new("model_install_manifest_invalid", false, error.to_string())
    })?;
    let actual_bytes = hex::decode(actual).map_err(|error| {
        InstallError::new("model_install_hash_failed", false, error.to_string())
    })?;
    if expected_bytes.len() != 32
        || actual_bytes.len() != 32
        || !bool::from(expected_bytes.ct_eq(&actual_bytes))
    {
        return Err(InstallError::new(
            "model_install_integrity_mismatch",
            false,
            "SHA-256 did not match the release-controlled manifest",
        ));
    }
    Ok(())
}

pub(crate) async fn partial_length(path: &Path) -> Result<u64, InstallError> {
    validate_partial_parent(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata_is_owned_regular_file(&metadata) => Err(InstallError::new(
            "model_install_partial_invalid",
            false,
            "partial asset is not a singly linked installer-owned regular file",
        )),
        Ok(_) => secure_open_regular(path, false)
            .and_then(|file| file.metadata())
            .map(|metadata| metadata.len())
            .map_err(|error| {
                InstallError::new("model_install_partial_unavailable", true, error.to_string())
            }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(InstallError::new(
            "model_install_partial_unavailable",
            true,
            error.to_string(),
        )),
    }
}

pub(crate) async fn reset_partial(path: &Path) -> Result<(), InstallError> {
    let file = secure_open_partial(path, PartialOpenMode::CreateOrTruncate).map_err(|error| {
        InstallError::new("model_install_write_failed", true, error.to_string())
    })?;
    let file = tokio::fs::File::from_std(file);
    file.sync_all()
        .await
        .map_err(|error| InstallError::new("model_install_write_failed", true, error.to_string()))
}

#[derive(Clone, Copy)]
enum PartialOpenMode {
    Append,
    CreateOrTruncate,
}

pub(crate) fn open_partial_for_download(path: &Path, append: bool) -> io::Result<File> {
    secure_open_partial(
        path,
        if append {
            PartialOpenMode::Append
        } else {
            PartialOpenMode::CreateOrTruncate
        },
    )
}

fn secure_open_partial(path: &Path, mode: PartialOpenMode) -> io::Result<File> {
    validate_partial_parent(path).map_err(|error| io::Error::other(error.code))?;
    let file = match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata_is_owned_regular_file(&metadata) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "partial path is not a singly linked regular file",
            ));
        }
        Ok(_) => secure_open_existing_write(path, matches!(mode, PartialOpenMode::Append))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut options = fs::OpenOptions::new();
            options.create_new(true).write(true);
            apply_no_follow(&mut options);
            let file = options.open(path)?;
            if !metadata_is_owned_regular_file(&file.metadata()?) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "new partial path is not a singly linked regular file",
                ));
            }
            file
        }
        Err(error) => return Err(error),
    };
    if matches!(mode, PartialOpenMode::CreateOrTruncate) {
        file.set_len(0)?;
    }
    set_restrictive_file_permissions(&file)?;
    Ok(file)
}

fn secure_open_regular(path: &Path, _append: bool) -> io::Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata_is_owned_regular_file(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is not a singly linked regular file",
        ));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    apply_no_follow(&mut options);
    let file = options.open(path)?;
    if !metadata_is_owned_regular_file(&file.metadata()?) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "opened path is not a singly linked regular file",
        ));
    }
    Ok(file)
}

pub(crate) fn metadata_is_owned_regular_file(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.nlink() == 1
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn secure_open_existing_write(path: &Path, append: bool) -> io::Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata_is_owned_regular_file(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is not a singly linked regular file",
        ));
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).append(append);
    apply_no_follow(&mut options);
    let file = options.open(path)?;
    if !metadata_is_owned_regular_file(&file.metadata()?) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "opened path is not a singly linked regular file",
        ));
    }
    Ok(file)
}

fn validate_partial_parent(path: &Path) -> Result<(), InstallError> {
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| value.ends_with(".part"))
        .ok_or_else(|| {
            InstallError::new(
                "model_install_partial_invalid",
                false,
                "partial filename did not match installer ownership",
            )
        })?;
    super::destination::validate_filename(filename.trim_end_matches(".part"))?;
    let staging = path.parent().ok_or_else(|| {
        InstallError::new(
            "model_install_partial_invalid",
            false,
            "partial path had no staging parent",
        )
    })?;
    let installer_root = staging.parent().ok_or_else(|| {
        InstallError::new(
            "model_install_partial_invalid",
            false,
            "partial path escaped installer staging",
        )
    })?;
    if !staging
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| super::state::valid_opaque_id(value, "install_"))
        || installer_root.file_name().and_then(|value| value.to_str())
            != Some(".oomu-model-install")
    {
        return Err(InstallError::new(
            "model_install_partial_invalid",
            false,
            "partial path escaped installer staging",
        ));
    }
    for directory in [installer_root, staging] {
        let metadata = fs::symlink_metadata(directory).map_err(|error| {
            InstallError::new("model_install_partial_unavailable", true, error.to_string())
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(InstallError::new(
                "model_install_symlink_refused",
                false,
                "installer staging ancestry is not a regular directory",
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn apply_no_follow(options: &mut fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .mode(0o600);
}

#[cfg(not(unix))]
fn apply_no_follow(_options: &mut fs::OpenOptions) {}

#[cfg(unix)]
fn set_restrictive_file_permissions(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_restrictive_file_permissions(_file: &File) -> io::Result<()> {
    Ok(())
}
