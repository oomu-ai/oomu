use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

use super::{
    destination::{
        self, create_staging, final_directory, promote_staging, seal_staging_assets,
        ValidatedDestination,
    },
    downloader::{verify_asset_file, AssetDownloadProgress},
    manifest::{self, recommended_model_manifest, RecommendedModelManifest, CANONICAL_MODEL_ID},
    receipt::{RecommendedModelInstallReceipt, RuntimeInspectionEvidence},
    state::{
        new_opaque_id, save_journal, InstallError, InstallJournal, InstallPhase, InstallProgress,
    },
    FinalizationRequest, InstallEventSink, RecommendedModelInstaller,
    JOURNAL_PROGRESS_INTERVAL_BYTES,
};

const PROGRESS_EVENT_INTERVAL_BYTES: u64 = 4 * 1024 * 1024;

impl RecommendedModelInstaller {
    pub(super) async fn run_install(
        &self,
        destination: ValidatedDestination,
        journal: InstallJournal,
        cancellation: Arc<AtomicBool>,
        sink: Arc<dyn InstallEventSink>,
    ) {
        let install_id = journal.install_id.clone();
        let result = self
            .execute_install(
                destination.clone(),
                journal,
                Arc::clone(&cancellation),
                Arc::clone(&sink),
            )
            .await;
        match result {
            Ok((journal, progress)) => {
                let mut runtime = self
                    .runtime
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                runtime.journal = Some(journal);
                runtime.progress = Some(progress.clone());
                runtime.cancellation = None;
                runtime.active = false;
                drop(runtime);
                sink.publish(&progress);
            }
            Err(error) => {
                eprintln!(
                    "OOMU_MODEL_INSTALL_FAILED installId={} code={} detail={}",
                    install_id,
                    error.code,
                    redact_private_detail(error.private_detail())
                );
                let mut runtime = self
                    .runtime
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut journal = runtime.journal.clone().unwrap_or_else(|| {
                    InstallJournal::new(
                        &recommended_model_manifest(),
                        install_id.clone(),
                        destination.root,
                        destination.kind,
                        crate::foundation::clock::unix_time_ms_u128(),
                    )
                });
                journal.phase = if error.code == "model_install_cancelled" {
                    InstallPhase::Cancelled
                } else if destination::canonical_package_entry_exists(&journal.destination_root) {
                    InstallPhase::RepairRequired
                } else {
                    InstallPhase::Failed
                };
                journal.updated_at_ms = crate::foundation::clock::unix_time_ms_u128();
                let _ = save_journal(&self.journal_path, &journal);
                let mut progress = progress_from_journal(&journal);
                progress.public_error_code =
                    (error.code != "model_install_cancelled").then(|| error.code.to_string());
                runtime.journal = Some(journal);
                runtime.progress = Some(progress.clone());
                runtime.cancellation = None;
                runtime.active = false;
                drop(runtime);
                sink.publish(&progress);
            }
        }
    }

    async fn execute_install(
        &self,
        destination: ValidatedDestination,
        mut journal: InstallJournal,
        cancellation: Arc<AtomicBool>,
        sink: Arc<dyn InstallEventSink>,
    ) -> Result<(InstallJournal, InstallProgress), InstallError> {
        let manifest = recommended_model_manifest();
        let final_path = final_directory(&destination.root);
        let (package_path, inspection) =
            if destination::canonical_package_entry_exists(&destination.root) {
                self.prepare_existing_package(
                    &manifest,
                    &mut journal,
                    final_path,
                    Arc::clone(&sink),
                )
                .await?
            } else {
                self.prepare_new_package(
                    &manifest,
                    &destination,
                    &mut journal,
                    cancellation,
                    Arc::clone(&sink),
                )
                .await?
            };
        self.finalize_install(destination, journal, package_path, inspection, sink)
            .await
    }

    async fn prepare_existing_package(
        &self,
        manifest: &RecommendedModelManifest,
        journal: &mut InstallJournal,
        final_path: PathBuf,
        sink: Arc<dyn InstallEventSink>,
    ) -> Result<(PathBuf, RuntimeInspectionEvidence), InstallError> {
        transition_shared(self, journal, InstallPhase::Verifying, None, &sink)?;
        verify_exact_package(manifest, &final_path).await?;
        transition_shared(self, journal, InstallPhase::Inspecting, None, &sink)?;
        let inspection = self.inspect_package(final_path.clone()).await?;
        Ok((final_path, inspection))
    }

    async fn prepare_new_package(
        &self,
        manifest: &RecommendedModelManifest,
        destination: &ValidatedDestination,
        journal: &mut InstallJournal,
        cancellation: Arc<AtomicBool>,
        sink: Arc<dyn InstallEventSink>,
    ) -> Result<(PathBuf, RuntimeInspectionEvidence), InstallError> {
        let staging = create_staging(&destination.root, &journal.install_id)?;
        self.download_assets(manifest, journal, &staging, cancellation, Arc::clone(&sink))
            .await?;
        transition_shared(self, journal, InstallPhase::Verifying, None, &sink)?;
        verify_staging_assets(manifest, &staging).await?;
        let filenames = manifest
            .assets
            .iter()
            .map(|asset| asset.filename.clone())
            .collect::<Vec<_>>();
        seal_staging_assets(&staging, &filenames)?;
        transition_shared(self, journal, InstallPhase::Inspecting, None, &sink)?;
        let inspection = self.inspect_package(staging.clone()).await?;
        transition_shared(self, journal, InstallPhase::Promoting, None, &sink)?;
        let promoted = promote_staging(&destination.root, &staging, &filenames)?;
        Ok((promoted, inspection))
    }

    async fn download_assets(
        &self,
        manifest: &RecommendedModelManifest,
        journal: &mut InstallJournal,
        staging: &Path,
        cancellation: Arc<AtomicBool>,
        sink: Arc<dyn InstallEventSink>,
    ) -> Result<(), InstallError> {
        for index in 0..manifest.assets.len() {
            self.download_one_asset(
                manifest,
                journal,
                staging,
                index,
                Arc::clone(&cancellation),
                Arc::clone(&sink),
            )
            .await?;
        }
        Ok(())
    }

    async fn download_one_asset(
        &self,
        manifest: &RecommendedModelManifest,
        journal: &mut InstallJournal,
        staging: &Path,
        index: usize,
        cancellation: Arc<AtomicBool>,
        sink: Arc<dyn InstallEventSink>,
    ) -> Result<(), InstallError> {
        if cancellation.load(Ordering::Acquire) {
            return Err(cancelled_install_error());
        }
        let asset = &manifest.assets[index];
        if reuse_verified_asset(asset, journal, index, staging).await? {
            return Ok(());
        }
        transition_shared(
            self,
            journal,
            InstallPhase::Downloading,
            Some(asset.role),
            &sink,
        )?;
        let aggregate_before = manifest.assets[..index]
            .iter()
            .map(|asset| asset.bytes)
            .sum::<u64>();
        let callback = self.progress_callback(journal, index, aggregate_before, sink);
        let outcome = self
            .downloader
            .as_ref()
            .ok_or_else(|| {
                InstallError::new(
                    "model_install_transport_unavailable",
                    true,
                    "native HTTPS client is unavailable",
                )
            })?
            .download_asset(
                asset,
                &staging.join(format!("{}.part", asset.filename)),
                journal.assets[index].etag.clone(),
                aggregate_before,
                cancellation,
                callback,
            )
            .await?;
        journal.assets[index].downloaded_bytes = outcome.bytes;
        journal.assets[index].etag = outcome.etag;
        journal.assets[index].verified = true;
        journal.updated_at_ms = crate::foundation::clock::unix_time_ms_u128();
        store_shared_journal(self, journal)
    }

    fn progress_callback(
        &self,
        journal: &InstallJournal,
        index: usize,
        aggregate_before: u64,
        sink: Arc<dyn InstallEventSink>,
    ) -> Arc<dyn Fn(AssetDownloadProgress) + Send + Sync> {
        let runtime = Arc::clone(&self.runtime);
        let journal_path = self.journal_path.clone();
        let install_id = journal.install_id.clone();
        let filename = journal.assets[index].filename.clone();
        let role = journal.assets[index].role;
        let last_persisted = Arc::new(AtomicU64::new(journal.assets[index].downloaded_bytes));
        let persisted_counter = Arc::clone(&last_persisted);
        let last_emitted = Arc::new(AtomicU64::new(
            aggregate_before + journal.assets[index].downloaded_bytes,
        ));
        let emitted_counter = Arc::clone(&last_emitted);
        let asset_total = recommended_model_manifest().assets[index].bytes;
        Arc::new(move |download| {
            let (snapshot, progress) = record_native_download_progress(
                &runtime,
                &install_id,
                role,
                &filename,
                &persisted_counter,
                &download,
            );
            let should_emit = should_emit_progress(&emitted_counter, &download, asset_total);
            if let Some(snapshot) = snapshot {
                let _ = save_journal(&journal_path, &snapshot);
            }
            if should_emit {
                if let Some(progress) = progress {
                    sink.publish(&progress);
                }
            }
        })
    }

    async fn inspect_package(
        &self,
        package_path: PathBuf,
    ) -> Result<RuntimeInspectionEvidence, InstallError> {
        let inspector = Arc::clone(&self.inspector);
        tokio::task::spawn_blocking(move || inspector.inspect(&package_path))
            .await
            .map_err(|error| {
                InstallError::new("model_install_inspection_failed", true, error.to_string())
            })?
    }

    async fn finalize_install(
        &self,
        destination: ValidatedDestination,
        mut journal: InstallJournal,
        model_directory: PathBuf,
        inspection: RuntimeInspectionEvidence,
        sink: Arc<dyn InstallEventSink>,
    ) -> Result<(InstallJournal, InstallProgress), InstallError> {
        let package_identity_before =
            destination::package_identity_sha256(&destination.root, &recommended_model_manifest())
                .ok_or_else(|| {
                    InstallError::new(
                        "model_install_package_identity_unavailable",
                        false,
                        "verified package metadata could not be bound to the receipt",
                    )
                })?;
        let previous_configuration = match journal.previous_configuration.clone() {
            Some(previous) => previous,
            None => {
                let previous = self.finalizer.snapshot_previous_configuration()?;
                journal.previous_configuration = Some(previous.clone());
                journal.updated_at_ms = crate::foundation::clock::unix_time_ms_u128();
                store_shared_journal(self, &journal)?;
                previous
            }
        };
        transition_shared(self, &mut journal, InstallPhase::Configuring, None, &sink)?;
        let provider = self
            .finalizer
            .finalize(FinalizationRequest {
                destination_root: destination.root.clone(),
                destination_kind: destination.kind,
                canonical_model_directory: model_directory,
                canonical_model_id: CANONICAL_MODEL_ID.to_string(),
                manifest_revision: manifest::IMMUTABLE_REVISION.to_string(),
                inspection: inspection.clone(),
                previous_configuration,
            })
            .await?;
        if !provider.validate() {
            return Err(InstallError::new(
                "model_install_provider_evidence_invalid",
                false,
                "finalizer did not return verified exact-model provider evidence",
            ));
        }
        let package_identity_after =
            destination::package_identity_sha256(&destination.root, &recommended_model_manifest())
                .filter(|identity| identity == &package_identity_before)
                .ok_or_else(|| {
                    InstallError::new(
                        "model_install_package_changed_during_activation",
                        false,
                        "package metadata changed while native activation was committing",
                    )
                })?;
        let completed_at_ms = crate::foundation::clock::unix_time_ms_u128();
        let receipt = RecommendedModelInstallReceipt::completed(
            new_opaque_id("model_receipt_"),
            &recommended_model_manifest(),
            inspection,
            provider.clone(),
            package_identity_after,
            journal.started_at_ms,
            completed_at_ms,
        );
        journal.phase = InstallPhase::Ready;
        journal.updated_at_ms = completed_at_ms;
        journal.receipt = Some(receipt);
        save_journal(&self.journal_path, &journal)?;
        let mut progress = progress_from_journal(&journal);
        progress.completed_provider = Some(provider);
        progress.downloaded_bytes = progress.total_bytes;
        store_shared_progress(self, &journal, &progress);
        Ok((journal, progress))
    }
}

async fn reuse_verified_asset(
    asset: &manifest::RecommendedModelAsset,
    journal: &mut InstallJournal,
    index: usize,
    staging: &Path,
) -> Result<bool, InstallError> {
    let completed = staging.join(&asset.filename);
    if completed.exists() {
        verify_asset_file(asset, &completed).await?;
        mark_asset_verified(journal, index, asset.bytes);
        return Ok(true);
    }
    let partial = staging.join(format!("{}.part", asset.filename));
    if super::partial_io::partial_length(&partial).await? != asset.bytes {
        return Ok(false);
    }
    match verify_asset_file(asset, &partial).await {
        Ok(()) => {
            mark_asset_verified(journal, index, asset.bytes);
            Ok(true)
        }
        Err(error) if error.code == "model_install_integrity_mismatch" => {
            std::fs::remove_file(&partial).map_err(|remove_error| {
                InstallError::new(
                    "model_install_invalid_partial_remove_failed",
                    true,
                    remove_error.to_string(),
                )
            })?;
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn mark_asset_verified(journal: &mut InstallJournal, index: usize, bytes: u64) {
    journal.assets[index].downloaded_bytes = bytes;
    journal.assets[index].verified = true;
}

async fn verify_staging_assets(
    manifest: &RecommendedModelManifest,
    staging: &Path,
) -> Result<(), InstallError> {
    for asset in &manifest.assets {
        let partial = staging.join(format!("{}.part", asset.filename));
        let completed = staging.join(&asset.filename);
        verify_asset_file(
            asset,
            if completed.exists() {
                &completed
            } else {
                &partial
            },
        )
        .await?;
    }
    Ok(())
}

fn record_native_download_progress(
    runtime: &Arc<std::sync::Mutex<super::RuntimeState>>,
    install_id: &str,
    role: manifest::AssetRole,
    filename: &str,
    persisted_counter: &AtomicU64,
    download: &AssetDownloadProgress,
) -> (Option<InstallJournal>, Option<InstallProgress>) {
    let mut runtime = runtime
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut snapshot = None;
    if let Some(journal) = runtime
        .journal
        .as_mut()
        .filter(|journal| journal.install_id == install_id)
    {
        if let Some(asset) = journal
            .assets
            .iter_mut()
            .find(|asset| asset.role == role && asset.filename == filename)
        {
            asset.downloaded_bytes = download.asset_downloaded_bytes;
            asset.etag = download.etag.clone();
        }
        journal.updated_at_ms = crate::foundation::clock::unix_time_ms_u128();
        let previous = persisted_counter.load(Ordering::Acquire);
        if download.asset_downloaded_bytes.saturating_sub(previous)
            >= JOURNAL_PROGRESS_INTERVAL_BYTES
        {
            persisted_counter.store(download.asset_downloaded_bytes, Ordering::Release);
            snapshot = Some(journal.clone());
        }
    }
    let progress = runtime.progress.as_mut().map(|progress| {
        progress.downloaded_bytes = download.aggregate_downloaded_bytes;
        progress.clone()
    });
    (snapshot, progress)
}

fn should_emit_progress(
    emitted_counter: &AtomicU64,
    download: &AssetDownloadProgress,
    asset_total: u64,
) -> bool {
    let previous = emitted_counter.load(Ordering::Acquire);
    let should_emit = download.aggregate_downloaded_bytes.saturating_sub(previous)
        >= PROGRESS_EVENT_INTERVAL_BYTES
        || download.asset_downloaded_bytes == asset_total;
    if should_emit {
        emitted_counter.store(download.aggregate_downloaded_bytes, Ordering::Release);
    }
    should_emit
}

fn cancelled_install_error() -> InstallError {
    InstallError::new(
        "model_install_cancelled",
        true,
        "native cancellation flag was set",
    )
}

fn transition_shared(
    installer: &RecommendedModelInstaller,
    journal: &mut InstallJournal,
    phase: InstallPhase,
    current_asset: Option<manifest::AssetRole>,
    sink: &Arc<dyn InstallEventSink>,
) -> Result<(), InstallError> {
    journal.phase = phase;
    journal.updated_at_ms = crate::foundation::clock::unix_time_ms_u128();
    save_journal(&installer.journal_path, journal)?;
    let mut progress = progress_from_journal(journal);
    progress.current_asset = current_asset.map(|role| match role {
        manifest::AssetRole::PrimaryModel => "primaryModel".to_string(),
        manifest::AssetRole::MultimodalProjector => "multimodalProjector".to_string(),
    });
    store_shared_progress(installer, journal, &progress);
    sink.publish(&progress);
    Ok(())
}

fn store_shared_journal(
    installer: &RecommendedModelInstaller,
    journal: &InstallJournal,
) -> Result<(), InstallError> {
    save_journal(&installer.journal_path, journal)?;
    let progress = progress_from_journal(journal);
    store_shared_progress(installer, journal, &progress);
    Ok(())
}

fn store_shared_progress(
    installer: &RecommendedModelInstaller,
    journal: &InstallJournal,
    progress: &InstallProgress,
) {
    let mut runtime = installer
        .runtime
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    runtime.journal = Some(journal.clone());
    runtime.progress = Some(progress.clone());
}

pub(super) fn progress_from_journal(journal: &InstallJournal) -> InstallProgress {
    let manifest = recommended_model_manifest();
    let provider = (journal.phase == InstallPhase::Ready)
        .then(|| {
            journal
                .receipt
                .as_ref()
                .map(|receipt| receipt.provider.clone())
        })
        .flatten();
    let mut progress = InstallProgress::new(journal.install_id.clone(), manifest.total_bytes);
    progress.downloaded_bytes = journal.downloaded_bytes().min(manifest.total_bytes);
    progress.transition(journal.phase, None);
    progress.completed_provider = provider;
    progress
}

pub(super) fn remaining_download_bytes(
    manifest: &RecommendedModelManifest,
    root: &Path,
    journal: Option<&InstallJournal>,
) -> u64 {
    if destination::canonical_package_entry_exists(root) {
        return 0;
    }
    let staging =
        journal.and_then(|journal| destination::staging_directory(root, &journal.install_id).ok());
    manifest
        .assets
        .iter()
        .map(|asset| {
            let present = staging
                .as_ref()
                .and_then(|directory| {
                    [
                        directory.join(&asset.filename),
                        directory.join(format!("{}.part", asset.filename)),
                    ]
                    .into_iter()
                    .find_map(|path| std::fs::symlink_metadata(path).ok())
                })
                .filter(super::partial_io::metadata_is_owned_regular_file)
                .map(|metadata| metadata.len().min(asset.bytes))
                .unwrap_or_default();
            asset.bytes.saturating_sub(present)
        })
        .sum()
}

pub(super) async fn verify_exact_package(
    manifest: &RecommendedModelManifest,
    directory: &Path,
) -> Result<(), InstallError> {
    let metadata = tokio::fs::symlink_metadata(directory)
        .await
        .map_err(|error| {
            InstallError::new("model_install_package_unavailable", true, error.to_string())
        })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(InstallError::new(
            "model_install_destination_collision",
            false,
            "canonical package path is not an adoptable directory",
        ));
    }
    let expected = manifest
        .assets
        .iter()
        .map(|asset| asset.filename.clone())
        .collect::<BTreeSet<_>>();
    let actual = std::fs::read_dir(directory)
        .map_err(|error| {
            InstallError::new("model_install_package_unavailable", true, error.to_string())
        })?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(InstallError::new(
            "model_install_destination_collision",
            false,
            "canonical directory was not the exact release-controlled package",
        ));
    }
    for asset in &manifest.assets {
        verify_asset_file(asset, &directory.join(&asset.filename)).await?;
    }
    Ok(())
}

fn redact_private_detail(detail: &str) -> String {
    if detail.contains('/') || detail.contains("http") || detail.len() > 160 {
        "redacted".to_string()
    } else {
        detail.to_string()
    }
}
