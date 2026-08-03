use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use super::{
    manifest::{RecommendedModelManifest, IMMUTABLE_REVISION, MANIFEST_SCHEMA_VERSION},
    receipt::{CompletedProviderEvidence, RecommendedModelInstallReceipt},
};

pub const INSTALL_PROGRESS_EVENT: &str = "recommended-model-install-progress";
const MAX_JOURNAL_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallPhase {
    Absent,
    Partial,
    Downloading,
    Verifying,
    Inspecting,
    Promoting,
    Configuring,
    Adoptable,
    Ready,
    RepairRequired,
    Failed,
    Cancelled,
}

impl InstallPhase {
    pub fn can_cancel(self) -> bool {
        matches!(self, Self::Downloading)
    }

    pub fn can_resume(self) -> bool {
        matches!(
            self,
            Self::Partial | Self::Failed | Self::Cancelled | Self::RepairRequired
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DestinationKind {
    Managed,
    Granted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    pub install_id: String,
    pub state: InstallPhase,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub current_asset: Option<String>,
    pub can_cancel: bool,
    pub can_resume: bool,
    pub public_error_code: Option<String>,
    pub completed_provider: Option<CompletedProviderEvidence>,
}

impl InstallProgress {
    pub(crate) fn new(install_id: String, total_bytes: u64) -> Self {
        Self {
            install_id,
            state: InstallPhase::Absent,
            downloaded_bytes: 0,
            total_bytes,
            current_asset: None,
            can_cancel: false,
            can_resume: false,
            public_error_code: None,
            completed_provider: None,
        }
    }

    pub(crate) fn transition(&mut self, state: InstallPhase, current_asset: Option<String>) {
        self.state = state;
        self.current_asset = current_asset;
        self.can_cancel = state.can_cancel();
        self.can_resume = state.can_resume();
        if !matches!(state, InstallPhase::Failed | InstallPhase::RepairRequired) {
            self.public_error_code = None;
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallLocationView {
    pub kind: DestinationKind,
    pub display_path: String,
    pub location_grant_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedModelInstallState {
    pub manifest: RecommendedModelManifest,
    pub location: InstallLocationView,
    pub package_state: InstallPhase,
    pub active_install: Option<InstallProgress>,
    pub receipt: Option<RecommendedModelInstallReceipt>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssetJournal {
    pub role: super::manifest::AssetRole,
    pub filename: String,
    pub downloaded_bytes: u64,
    pub etag: Option<String>,
    pub verified: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreviousConfiguration {
    pub active_models_root: Option<PathBuf>,
    pub prewarmed_model_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstallJournal {
    pub schema_version: u32,
    pub manifest_revision: String,
    pub install_id: String,
    pub destination_root: PathBuf,
    pub destination_kind: DestinationKind,
    pub assets: Vec<AssetJournal>,
    pub phase: InstallPhase,
    pub started_at_ms: u128,
    pub updated_at_ms: u128,
    pub previous_configuration: Option<PreviousConfiguration>,
    pub receipt: Option<RecommendedModelInstallReceipt>,
}

impl InstallJournal {
    pub(crate) fn new(
        manifest: &RecommendedModelManifest,
        install_id: String,
        destination_root: PathBuf,
        destination_kind: DestinationKind,
        now_ms: u128,
    ) -> Self {
        Self {
            schema_version: manifest.schema_version,
            manifest_revision: manifest.revision.clone(),
            install_id,
            destination_root,
            destination_kind,
            assets: manifest
                .assets
                .iter()
                .map(|asset| AssetJournal {
                    role: asset.role,
                    filename: asset.filename.clone(),
                    downloaded_bytes: 0,
                    etag: None,
                    verified: false,
                })
                .collect(),
            phase: InstallPhase::Absent,
            started_at_ms: now_ms,
            updated_at_ms: now_ms,
            previous_configuration: None,
            receipt: None,
        }
    }

    pub(crate) fn downloaded_bytes(&self) -> u64 {
        self.assets.iter().map(|asset| asset.downloaded_bytes).sum()
    }

    pub(crate) fn is_current(&self) -> bool {
        self.schema_version == MANIFEST_SCHEMA_VERSION
            && self.manifest_revision == IMMUTABLE_REVISION
            && valid_opaque_id(&self.install_id, "install_")
            && self.destination_root.is_absolute()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{code}")]
pub struct InstallError {
    pub code: &'static str,
    pub retryable: bool,
    detail: String,
}

impl InstallError {
    pub(crate) fn new(code: &'static str, retryable: bool, detail: impl Into<String>) -> Self {
        Self {
            code,
            retryable,
            detail: detail.into(),
        }
    }

    pub(crate) fn private_detail(&self) -> &str {
        &self.detail
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallCommandError {
    pub code: String,
    pub retryable: bool,
}

impl From<InstallError> for InstallCommandError {
    fn from(error: InstallError) -> Self {
        Self {
            code: error.code.to_string(),
            retryable: error.retryable,
        }
    }
}

pub(crate) fn new_opaque_id(prefix: &str) -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}{}", hex::encode(bytes))
}

pub(crate) fn valid_opaque_id(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

pub(crate) fn load_journal(path: &Path) -> Result<Option<InstallJournal>, InstallError> {
    let path_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(InstallError::new(
                "model_install_journal_unavailable",
                true,
                error.to_string(),
            ));
        }
    };
    if !super::partial_io::metadata_is_owned_regular_file(&path_metadata) {
        return Err(InstallError::new(
            "model_install_journal_invalid",
            false,
            "journal is not a singly linked native regular file",
        ));
    }
    let mut file = open_journal_read(path).map_err(|error| {
        InstallError::new("model_install_journal_unavailable", true, error.to_string())
    })?;
    let metadata = file.metadata().map_err(|error| {
        InstallError::new("model_install_journal_unavailable", true, error.to_string())
    })?;
    if !super::partial_io::metadata_is_owned_regular_file(&metadata)
        || metadata.len() > MAX_JOURNAL_BYTES
    {
        return Err(InstallError::new(
            "model_install_journal_invalid",
            false,
            "journal identity or byte length failed native validation",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes).map_err(|error| {
        InstallError::new("model_install_journal_unavailable", true, error.to_string())
    })?;
    let journal: InstallJournal = serde_json::from_slice(&bytes).map_err(|error| {
        InstallError::new("model_install_journal_invalid", false, error.to_string())
    })?;
    Ok(journal.is_current().then_some(journal))
}

pub(crate) fn save_journal(path: &Path, journal: &InstallJournal) -> Result<(), InstallError> {
    let parent = path.parent().ok_or_else(|| {
        InstallError::new(
            "model_install_journal_unavailable",
            false,
            "journal has no parent directory",
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        InstallError::new("model_install_journal_unavailable", true, error.to_string())
    })?;
    let temporary = parent.join(format!(".journal-{}.tmp", new_opaque_id("write_")));
    let bytes = serde_json::to_vec(journal).map_err(|error| {
        InstallError::new("model_install_journal_invalid", false, error.to_string())
    })?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| {
            InstallError::new("model_install_journal_unavailable", true, error.to_string())
        })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                InstallError::new("model_install_journal_unavailable", true, error.to_string())
            })?;
    }
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(InstallError::new(
            "model_install_journal_unavailable",
            true,
            error.to_string(),
        ));
    }
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        InstallError::new("model_install_journal_unavailable", true, error.to_string())
    })?;
    sync_directory(parent).map_err(|error| {
        InstallError::new("model_install_journal_unavailable", true, error.to_string())
    })
}

fn open_journal_read(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    options.open(path)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}
