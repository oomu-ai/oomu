pub(crate) mod commands;
mod destination;
mod downloader;
mod job;
mod manifest;
mod network;
mod partial_io;
mod receipt;
mod state;

use destination::{
    canonical_package_entry_exists, discard_staging, probe_package_shape, DestinationAuthority,
    PackageShape,
};
use downloader::Downloader;
use job::{progress_from_journal, remaining_download_bytes};
use rfd::AsyncFileDialog;
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::Emitter;

pub use commands::{DiscardPartialResponse, LocationGrantResponse, StartInstallResponse};
pub use manifest::{recommended_model_manifest, CANONICAL_MODEL_ID, IMMUTABLE_REVISION};
pub use receipt::{CompletedProviderEvidence, RuntimeInspectionEvidence};
pub(crate) use state::InstallError;
pub(crate) use state::PreviousConfiguration;
pub use state::{
    DestinationKind, InstallCommandError, InstallPhase, InstallProgress,
    RecommendedModelInstallState, INSTALL_PROGRESS_EVENT,
};

use state::{load_journal, new_opaque_id, save_journal, InstallJournal, InstallLocationView};

const JOURNAL_RELATIVE_PATH: &str = "recommended-model-install/install-state.json";
const JOURNAL_PROGRESS_INTERVAL_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) type FinalizationFuture =
    Pin<Box<dyn Future<Output = Result<CompletedProviderEvidence, InstallError>> + Send + 'static>>;

#[derive(Clone, Debug)]
pub(crate) struct FinalizationRequest {
    pub destination_root: PathBuf,
    pub destination_kind: DestinationKind,
    pub canonical_model_directory: PathBuf,
    pub canonical_model_id: String,
    pub manifest_revision: String,
    pub inspection: RuntimeInspectionEvidence,
    pub previous_configuration: PreviousConfiguration,
}

pub(crate) trait RecommendedModelInstallFinalizer: Send + Sync {
    fn snapshot_previous_configuration(&self) -> Result<PreviousConfiguration, InstallError>;

    fn finalize(&self, request: FinalizationRequest) -> FinalizationFuture;
}

trait PackageInspector: Send + Sync {
    fn inspect(&self, package_directory: &Path) -> Result<RuntimeInspectionEvidence, InstallError>;
}

struct NativePackageInspector;

impl PackageInspector for NativePackageInspector {
    fn inspect(&self, package_directory: &Path) -> Result<RuntimeInspectionEvidence, InstallError> {
        let manifest = recommended_model_manifest();
        let primary = package_directory.join(&manifest.primary_asset().filename);
        let projector = package_directory.join(&manifest.projector_asset().filename);
        let projector_count = std::fs::read_dir(package_directory)
            .map_err(|error| {
                InstallError::new("model_install_inspection_failed", true, error.to_string())
            })?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("gguf")
                    && path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|name| name.to_ascii_lowercase().contains("mmproj"))
            })
            .count();
        if projector_count != 1 || !projector.is_file() {
            return Err(InstallError::new(
                "model_install_projector_invalid",
                false,
                "package did not expose exactly one matching multimodal projector",
            ));
        }
        let runtime = crate::native_runtime::NativeRuntime::initialize().map_err(|error| {
            InstallError::new("model_install_runtime_unavailable", true, error.code)
        })?;
        let profile = runtime.inspect_model(&primary).map_err(|error| {
            InstallError::new("model_install_inspection_failed", false, error.code)
        })?;
        if profile.model_bytes != manifest.primary_asset().bytes {
            return Err(InstallError::new(
                "model_install_inspection_failed",
                false,
                "runtime inspection byte count did not match the immutable manifest",
            ));
        }
        Ok(RuntimeInspectionEvidence {
            accepted: true,
            architecture: profile.architecture,
            tensor_count: profile.tensor_count,
            model_bytes: profile.model_bytes,
            multimodal_projector_count: projector_count,
        })
    }
}

trait InstallEventSink: Send + Sync {
    fn publish(&self, progress: &InstallProgress);
}

struct TauriInstallEventSink(tauri::AppHandle);

impl InstallEventSink for TauriInstallEventSink {
    fn publish(&self, progress: &InstallProgress) {
        if let Err(error) = self.0.emit(INSTALL_PROGRESS_EVENT, progress) {
            eprintln!("OOMU_MODEL_INSTALL_EVENT_FAILED {}", error);
        }
    }
}

struct RuntimeState {
    prospective_location: InstallLocationView,
    progress: Option<InstallProgress>,
    journal: Option<InstallJournal>,
    cancellation: Option<Arc<AtomicBool>>,
    active: bool,
}

#[derive(Clone)]
pub struct RecommendedModelInstaller {
    destination: Arc<DestinationAuthority>,
    downloader: Option<Downloader>,
    inspector: Arc<dyn PackageInspector>,
    finalizer: Arc<dyn RecommendedModelInstallFinalizer>,
    journal_path: PathBuf,
    runtime: Arc<Mutex<RuntimeState>>,
}

impl RecommendedModelInstaller {
    pub(crate) fn new(
        managed_models_root: PathBuf,
        app_data_directory: PathBuf,
        finalizer: Arc<dyn RecommendedModelInstallFinalizer>,
    ) -> Self {
        let destination = Arc::new(DestinationAuthority::new(managed_models_root));
        let journal_path = app_data_directory.join(JOURNAL_RELATIVE_PATH);
        let (journal, journal_error_code) = match load_journal(&journal_path) {
            Ok(journal) => (journal, None),
            Err(error) => (None, Some(error.code.to_string())),
        };
        let downloader = match Downloader::production() {
            Ok(downloader) => Some(downloader),
            Err(_) => None,
        };
        let prospective_location = journal
            .as_ref()
            .map(|journal| InstallLocationView {
                kind: journal.destination_kind,
                display_path: journal.destination_root.display().to_string(),
                location_grant_id: None,
            })
            .unwrap_or_else(|| InstallLocationView {
                kind: DestinationKind::Managed,
                display_path: destination.managed_display_path(),
                location_grant_id: None,
            });
        let progress = journal.as_ref().map(progress_from_journal).or_else(|| {
            journal_error_code.map(|code| {
                let mut progress = InstallProgress::new(
                    new_opaque_id("install_"),
                    recommended_model_manifest().total_bytes,
                );
                progress.transition(InstallPhase::RepairRequired, None);
                progress.public_error_code = Some(code);
                progress
            })
        });
        Self {
            destination,
            downloader,
            inspector: Arc::new(NativePackageInspector),
            finalizer,
            journal_path,
            runtime: Arc::new(Mutex::new(RuntimeState {
                prospective_location,
                progress,
                journal,
                cancellation: None,
                active: false,
            })),
        }
    }

    #[cfg(test)]
    fn for_test(
        managed_models_root: PathBuf,
        app_data_directory: PathBuf,
        inspector: Arc<dyn PackageInspector>,
        finalizer: Arc<dyn RecommendedModelInstallFinalizer>,
    ) -> Result<Self, InstallError> {
        let destination = Arc::new(DestinationAuthority::for_test(managed_models_root, None));
        Ok(Self {
            destination: Arc::clone(&destination),
            downloader: Some(Downloader::for_local_fixture()?),
            inspector,
            finalizer,
            journal_path: app_data_directory.join(JOURNAL_RELATIVE_PATH),
            runtime: Arc::new(Mutex::new(RuntimeState {
                prospective_location: InstallLocationView {
                    kind: DestinationKind::Managed,
                    display_path: destination.managed_display_path(),
                    location_grant_id: None,
                },
                progress: None,
                journal: None,
                cancellation: None,
                active: false,
            })),
        })
    }

    pub fn state(&self) -> RecommendedModelInstallState {
        let (active, location, progress, journal) = {
            let runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            (
                runtime.active,
                runtime.prospective_location.clone(),
                runtime.progress.clone(),
                runtime.journal.clone(),
            )
        };
        let manifest = recommended_model_manifest();
        if active {
            return RecommendedModelInstallState {
                manifest,
                location,
                package_state: progress
                    .as_ref()
                    .map(|progress| progress.state)
                    .unwrap_or(InstallPhase::Failed),
                active_install: progress,
                receipt: None,
            };
        }
        let location_path = Path::new(&location.display_path);
        let selected_new_location = location.location_grant_id.is_some()
            && journal
                .as_ref()
                .is_some_and(|journal| journal.destination_root != location_path);
        let relevant_journal = (!selected_new_location)
            .then_some(journal.as_ref())
            .flatten();
        let relevant_progress = (!selected_new_location).then_some(progress).flatten();
        let probe_root = relevant_journal
            .map(|journal| journal.destination_root.as_path())
            .unwrap_or(location_path);
        let shape = probe_package_shape(probe_root, &manifest);
        let package_identity_matches = relevant_journal
            .filter(|journal| journal.phase == InstallPhase::Ready)
            .and_then(|journal| journal.receipt.as_ref())
            .and_then(|receipt| receipt.package_identity_sha256.as_ref())
            .zip(destination::package_identity_sha256(probe_root, &manifest).as_ref())
            .is_some_and(|(expected, actual)| expected == actual);
        let (package_state, active_install, receipt) = project_idle_state(
            relevant_progress,
            relevant_journal,
            shape,
            package_identity_matches,
        );
        RecommendedModelInstallState {
            manifest,
            location,
            package_state,
            active_install,
            receipt,
        }
    }

    async fn choose_location(
        &self,
        dialog_title: String,
    ) -> Result<Option<LocationGrantResponse>, InstallError> {
        let dialog_title = validate_dialog_title(&dialog_title)?;
        let Some(selected) = AsyncFileDialog::new()
            .set_title(dialog_title)
            .pick_folder()
            .await
        else {
            return Ok(None);
        };
        let grant = self.destination.issue_grant(selected.path())?;
        let response = LocationGrantResponse {
            location_grant_id: grant.grant_id,
            display_path: grant.display_path,
        };
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .prospective_location = InstallLocationView {
            kind: DestinationKind::Granted,
            display_path: response.display_path.clone(),
            location_grant_id: Some(response.location_grant_id.clone()),
        };
        Ok(Some(response))
    }

    fn start(
        &self,
        location_grant_id: Option<String>,
        sink: Arc<dyn InstallEventSink>,
    ) -> Result<StartInstallResponse, InstallError> {
        {
            let runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if runtime.active {
                let progress = runtime.progress.clone().ok_or_else(|| {
                    InstallError::new(
                        "model_install_state_invalid",
                        true,
                        "single-flight was active without progress",
                    )
                })?;
                return Ok(StartInstallResponse {
                    install_id: progress.install_id.clone(),
                    attached: true,
                    progress,
                });
            }
        }
        let manifest = recommended_model_manifest();
        let existing_journal = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .journal
            .clone();
        let preliminary = match (&location_grant_id, existing_journal.as_ref()) {
            (None, Some(journal)) => self.destination.resolve_recovered(
                &journal.destination_root,
                journal.destination_kind,
                0,
            )?,
            _ => self.destination.resolve(location_grant_id.as_deref(), 0)?,
        };
        if let Some(journal) = existing_journal.as_ref() {
            if journal.destination_root != preliminary.root
                && !matches!(
                    journal.phase,
                    InstallPhase::Ready | InstallPhase::RepairRequired
                )
            {
                return Err(InstallError::new(
                    "model_install_partial_at_other_location",
                    false,
                    "an unfinished native journal belongs to another destination",
                ));
            }
        }
        let journal_at_destination = existing_journal
            .as_ref()
            .filter(|journal| journal.destination_root == preliminary.root);
        let remaining =
            remaining_download_bytes(&manifest, &preliminary.root, journal_at_destination);
        if remaining > 0 && self.downloader.is_none() {
            return Err(InstallError::new(
                "model_install_transport_unavailable",
                true,
                "native HTTPS client could not be initialized",
            ));
        }
        let destination =
            self.destination
                .resolve_recovered(&preliminary.root, preliminary.kind, remaining)?;
        let now = crate::foundation::clock::unix_time_ms_u128();
        let mut journal = existing_journal
            .filter(|journal| journal.destination_root == destination.root)
            .unwrap_or_else(|| {
                InstallJournal::new(
                    &manifest,
                    new_opaque_id("install_"),
                    destination.root.clone(),
                    destination.kind,
                    now,
                )
            });
        if journal.phase == InstallPhase::Ready {
            journal.previous_configuration = None;
        }
        journal.receipt = None;
        journal.phase = if remaining == 0 {
            InstallPhase::Verifying
        } else {
            InstallPhase::Downloading
        };
        journal.updated_at_ms = now;
        save_journal(&self.journal_path, &journal)?;
        let cancellation = Arc::new(AtomicBool::new(false));
        let progress = progress_from_journal(&journal);
        {
            let mut runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            runtime.prospective_location = InstallLocationView {
                kind: destination.kind,
                display_path: destination.root.display().to_string(),
                location_grant_id,
            };
            runtime.progress = Some(progress.clone());
            runtime.journal = Some(journal.clone());
            runtime.cancellation = Some(Arc::clone(&cancellation));
            runtime.active = true;
        }
        sink.publish(&progress);
        let installer = self.clone();
        let response = StartInstallResponse {
            install_id: journal.install_id.clone(),
            attached: false,
            progress,
        };
        tauri::async_runtime::spawn(async move {
            installer
                .run_install(destination, journal, cancellation, sink)
                .await;
        });
        Ok(response)
    }

    fn cancel(&self, install_id: &str) -> Result<InstallProgress, InstallError> {
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if runtime
            .progress
            .as_ref()
            .map(|progress| progress.install_id.as_str())
            != Some(install_id)
        {
            return Err(InstallError::new(
                "model_install_identifier_mismatch",
                false,
                "cancel id did not match the active native installation",
            ));
        }
        let cancellation = runtime.cancellation.as_ref().ok_or_else(|| {
            InstallError::new(
                "model_install_not_active",
                false,
                "installation is not active",
            )
        })?;
        cancellation.store(true, Ordering::Release);
        let progress = runtime.progress.as_mut().expect("validated above");
        progress.can_cancel = false;
        Ok(progress.clone())
    }

    fn discard(&self, install_id: &str) -> Result<DiscardPartialResponse, InstallError> {
        let mut runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if runtime.active {
            return Err(InstallError::new(
                "model_install_active",
                false,
                "active installation must be cancelled before discarding partials",
            ));
        }
        let journal = runtime.journal.clone().ok_or_else(|| {
            InstallError::new("model_install_not_found", false, "no installation exists")
        })?;
        if journal.install_id != install_id {
            return Err(InstallError::new(
                "model_install_identifier_mismatch",
                false,
                "discard id did not match the native journal",
            ));
        }
        if journal.phase == InstallPhase::Ready {
            return Err(InstallError::new(
                "model_install_ready_package_protected",
                false,
                "discard partial cannot remove a ready package",
            ));
        }
        if canonical_package_entry_exists(&journal.destination_root) {
            return Err(InstallError::new(
                "model_install_verified_package_protected",
                false,
                "discard partial cannot remove or reset a promoted package",
            ));
        }
        let discarded = discard_staging(&journal.destination_root, install_id)?;
        let mut reset = InstallJournal::new(
            &recommended_model_manifest(),
            journal.install_id,
            journal.destination_root,
            journal.destination_kind,
            crate::foundation::clock::unix_time_ms_u128(),
        );
        reset.phase = InstallPhase::Absent;
        save_journal(&self.journal_path, &reset)?;
        runtime.progress = Some(progress_from_journal(&reset));
        runtime.journal = Some(reset);
        Ok(DiscardPartialResponse {
            discarded,
            state: InstallPhase::Absent,
        })
    }
}

fn project_idle_state(
    progress: Option<InstallProgress>,
    journal: Option<&InstallJournal>,
    shape: PackageShape,
    package_identity_matches: bool,
) -> (
    InstallPhase,
    Option<InstallProgress>,
    Option<receipt::RecommendedModelInstallReceipt>,
) {
    let persisted_phase = journal
        .map(|journal| journal.phase)
        .or_else(|| progress.as_ref().map(|progress| progress.state))
        .unwrap_or(InstallPhase::Absent);
    let package_state = match shape {
        PackageShape::Adoptable
            if persisted_phase == InstallPhase::Ready && package_identity_matches =>
        {
            InstallPhase::Ready
        }
        PackageShape::Adoptable => InstallPhase::Adoptable,
        PackageShape::Absent if persisted_phase == InstallPhase::Ready => {
            InstallPhase::RepairRequired
        }
        PackageShape::Absent => persisted_phase,
        PackageShape::Invalid => InstallPhase::RepairRequired,
    };
    let mut active_install = progress.or_else(|| journal.map(progress_from_journal));
    if let Some(active) = active_install.as_mut() {
        active.transition(package_state, None);
        if package_state == InstallPhase::RepairRequired && active.public_error_code.is_none() {
            active.public_error_code = Some(
                match shape {
                    PackageShape::Absent => "model_install_ready_package_missing",
                    PackageShape::Invalid => "model_install_package_shape_invalid",
                    PackageShape::Adoptable => "model_install_repair_required",
                }
                .to_string(),
            );
        }
        if package_state != InstallPhase::Ready {
            active.completed_provider = None;
        }
    }
    let receipt = (package_state == InstallPhase::Ready)
        .then(|| journal.and_then(|journal| journal.receipt.clone()))
        .flatten();
    (package_state, active_install, receipt)
}

fn validate_dialog_title(title: &str) -> Result<&str, InstallError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 120 || title.chars().any(char::is_control) {
        return Err(InstallError::new(
            "model_install_dialog_title_invalid",
            false,
            "localized dialog title failed native validation",
        ));
    }
    Ok(title)
}

#[cfg(test)]
mod tests;
