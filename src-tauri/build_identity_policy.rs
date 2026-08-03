use sha2::{Digest, Sha256};
use std::{io::ErrorKind, path::Component};

const GENERATED_PACKAGE_DIRECTORIES: &[&str] =
    &["out", "src-tauri/binaries", "src-tauri/resources/python"];
const MISSING_DIRECTORY_DOMAIN: &str = "oomu.package-identity.missing-generated-directory.v1";
const MISSING_FRONTEND_DOMAIN: &str = "oomu.frontend-export.missing-development.v1\0out";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MissingFrontendIdentity {
    pub(crate) digest: String,
    pub(crate) file_count: usize,
}

pub(crate) fn validate_profile(profile: &str) -> Result<(), String> {
    match profile {
        "debug" | "release" => Ok(()),
        _ => Err(format!("unknown Cargo build profile: {profile}")),
    }
}

pub(crate) fn missing_generated_directory_marker(
    profile: &str,
    relative_path: &str,
    error_kind: ErrorKind,
) -> Result<String, String> {
    validate_profile(profile)?;
    if error_kind != ErrorKind::NotFound {
        return Err("package identity input is unreadable".to_string());
    }
    if profile != "debug" {
        return Err("missing generated package input is permitted only in debug".to_string());
    }
    let relative_path = normalized_relative_path(relative_path)?;
    if !GENERATED_PACKAGE_DIRECTORIES
        .iter()
        .any(|root| relative_path == *root || relative_path.starts_with(&format!("{root}/")))
    {
        return Err("missing tracked package input is forbidden".to_string());
    }
    Ok(format!("{MISSING_DIRECTORY_DOMAIN}\0{relative_path}"))
}

pub(crate) fn missing_frontend_export_identity(
    profile: &str,
    error_kind: ErrorKind,
) -> Result<MissingFrontendIdentity, String> {
    validate_profile(profile)?;
    if error_kind != ErrorKind::NotFound {
        return Err("frontend export is unreadable".to_string());
    }
    if profile != "debug" {
        return Err("missing frontend export is permitted only in debug".to_string());
    }
    Ok(MissingFrontendIdentity {
        digest: hex::encode(Sha256::digest(MISSING_FRONTEND_DOMAIN.as_bytes())),
        file_count: 0,
    })
}

fn normalized_relative_path(path: &str) -> Result<String, String> {
    let normalized = path.replace('\\', "/");
    if normalized.is_empty() {
        return Err("package identity path is empty".to_string());
    }
    let mut components = Vec::new();
    for component in std::path::Path::new(&normalized).components() {
        match component {
            Component::Normal(value) => components.push(value.to_string_lossy().into_owned()),
            _ => return Err("package identity path must be normalized and root-relative".into()),
        }
    }
    Ok(components.join("/"))
}
