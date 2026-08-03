use super::{
    destination::{
        create_staging, final_directory, package_identity_sha256, promote_staging,
        seal_staging_assets, validate_exact_staging_entries, DestinationAuthority,
        STORAGE_SAFETY_MARGIN_BYTES,
    },
    downloader::{verify_asset_file, Downloader},
    job::verify_exact_package,
    manifest::{
        fixture_manifest, recommended_model_manifest, CANONICAL_MODEL_ID, IMMUTABLE_REVISION,
        PACKAGE_TOTAL_BYTES,
    },
    network::{validate_content_range, validate_network_destination},
    receipt::{
        CompletedProviderEvidence, RecommendedModelInstallReceipt, RuntimeInspectionEvidence,
    },
    state::{
        load_journal, new_opaque_id, save_journal, DestinationKind, InstallJournal, InstallPhase,
        InstallProgress, PreviousConfiguration,
    },
    FinalizationFuture, FinalizationRequest, InstallEventSink, PackageInspector,
    RecommendedModelInstallFinalizer, RecommendedModelInstaller,
};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_RANGE};
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::sleep,
};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap_or_else(|_| std::env::temp_dir());
        let path = base.join(format!(
            "oomu-model-installer-{label}-{}",
            new_opaque_id("fixture_")
        ));
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct FixtureInspector;

impl PackageInspector for FixtureInspector {
    fn inspect(
        &self,
        _package_directory: &Path,
    ) -> Result<RuntimeInspectionEvidence, super::state::InstallError> {
        Ok(RuntimeInspectionEvidence {
            accepted: true,
            architecture: "fixture".to_string(),
            tensor_count: 1,
            model_bytes: 1,
            multimodal_projector_count: 1,
        })
    }
}

struct FixtureFinalizer;

impl RecommendedModelInstallFinalizer for FixtureFinalizer {
    fn snapshot_previous_configuration(
        &self,
    ) -> Result<PreviousConfiguration, super::state::InstallError> {
        Ok(PreviousConfiguration {
            active_models_root: None,
            prewarmed_model_id: None,
        })
    }

    fn finalize(&self, _request: FinalizationRequest) -> FinalizationFuture {
        Box::pin(async {
            Ok(CompletedProviderEvidence::verified_local(
                "local-model",
                None,
            ))
        })
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<InstallProgress>>);

impl InstallEventSink for RecordingSink {
    fn publish(&self, progress: &InstallProgress) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(progress.clone());
    }
}

#[derive(Clone)]
struct FixtureResponse {
    status: &'static str,
    headers: Vec<(String, String)>,
    chunks: Vec<Vec<u8>>,
    delay_ms: u64,
}

async fn fixture_server(responses: Vec<FixtureResponse>) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let handle = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().await.expect("accept fixture request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream
                    .read(&mut buffer)
                    .await
                    .expect("read fixture request");
                if read == 0 || request.len() > 64 * 1024 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            requests.push(String::from_utf8_lossy(&request).into_owned());
            let body_bytes = response.chunks.iter().map(Vec::len).sum::<usize>();
            let mut head = format!(
                "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                response.status, body_bytes
            );
            for (name, value) in response.headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str("\r\n");
            stream
                .write_all(head.as_bytes())
                .await
                .expect("write headers");
            for chunk in response.chunks {
                if stream.write_all(&chunk).await.is_err() {
                    break;
                }
                if response.delay_ms > 0 {
                    sleep(Duration::from_millis(response.delay_ms)).await;
                }
            }
        }
        requests
    });
    (format!("http://{address}"), handle)
}

fn fixture_staging(root: &Path) -> (String, PathBuf) {
    let install_id = new_opaque_id("install_");
    let staging = create_staging(root, &install_id).expect("create fixture staging");
    (install_id, staging)
}

#[test]
fn immutable_manifest_matches_the_approved_google_package() {
    let manifest = recommended_model_manifest();
    assert_eq!(manifest.model_id, CANONICAL_MODEL_ID);
    assert_eq!(manifest.revision, IMMUTABLE_REVISION);
    assert_eq!(manifest.total_bytes, PACKAGE_TOTAL_BYTES);
    assert_eq!(manifest.assets.len(), 2);
    assert_eq!(manifest.assets[0].bytes, 3_349_516_256);
    assert_eq!(manifest.assets[1].bytes, 986_833_664);
    assert_eq!(manifest.displayed_license, "Apache License 2.0");
    assert_eq!(manifest.attribution, "Google");
    assert!(manifest
        .assets
        .iter()
        .all(|asset| asset.url.contains(IMMUTABLE_REVISION)));
}

#[test]
fn cancellation_is_truthful_only_during_streaming() {
    assert!(InstallPhase::Downloading.can_cancel());
    assert!(!InstallPhase::Verifying.can_cancel());
    assert!(!InstallPhase::Inspecting.can_cancel());
    assert!(!InstallPhase::Adoptable.can_cancel());
    assert!(InstallPhase::Cancelled.can_resume());
}

#[test]
fn default_and_opaque_granted_destinations_resolve_natively() {
    let root = TestDirectory::new("destination");
    let managed = root.path().join("managed");
    let custom = root.path().join("custom");
    fs::create_dir_all(&custom).unwrap();
    let authority = DestinationAuthority::for_test(managed.clone(), None);
    assert_eq!(
        authority.resolve(None, 0).unwrap().kind,
        DestinationKind::Managed
    );
    let grant = authority.issue_grant(&custom).unwrap();
    let granted = authority.resolve(Some(&grant.grant_id), 0).unwrap();
    assert_eq!(granted.kind, DestinationKind::Granted);
    assert_eq!(granted.root, custom.canonicalize().unwrap());
    assert_eq!(
        authority.resolve(Some("../../escape"), 0).unwrap_err().code,
        "model_install_location_grant_invalid"
    );
}

#[test]
fn app_bundle_alias_and_impossible_storage_request_are_refused() {
    let root = TestDirectory::new("destination-policy");
    let bundle = root.path().join("OOMU.app");
    let nested = bundle.join("Contents/Models");
    fs::create_dir_all(&nested).unwrap();
    let authority = DestinationAuthority::for_test(root.path().join("managed"), Some(bundle));
    assert_eq!(
        authority.issue_grant(&nested).unwrap_err().code,
        "model_install_application_bundle_refused"
    );
    assert_eq!(
        authority
            .resolve(None, u64::MAX - STORAGE_SAFETY_MARGIN_BYTES)
            .unwrap_err()
            .code,
        "model_install_insufficient_storage"
    );
}

#[cfg(unix)]
#[test]
fn destination_and_partial_symlinks_are_never_followed() {
    use std::os::unix::fs::symlink;

    let root = TestDirectory::new("symlinks");
    let real = root.path().join("real");
    let alias = root.path().join("alias");
    fs::create_dir_all(&real).unwrap();
    symlink(&real, &alias).unwrap();
    let authority = DestinationAuthority::for_test(root.path().join("managed"), None);
    assert_eq!(
        authority.issue_grant(&alias).unwrap_err().code,
        "model_install_symlink_refused"
    );
}

#[tokio::test]
async fn downloader_streams_a_real_local_200_response_and_hashes_disk_bytes() {
    let body = b"fixture-primary-model".repeat(1024);
    let projector = b"projector".to_vec();
    let response = FixtureResponse {
        status: "200 OK",
        headers: vec![("ETag".to_string(), "\"fixture-v1\"".to_string())],
        chunks: vec![body[..4096].to_vec(), body[4096..].to_vec()],
        delay_ms: 0,
    };
    let (base_url, server) = fixture_server(vec![response]).await;
    let manifest = fixture_manifest(&base_url, &body, &projector);
    let root = TestDirectory::new("download-200");
    let (_, staging) = fixture_staging(root.path());
    let partial = staging.join(format!("{}.part", manifest.assets[0].filename));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let observed_callback = Arc::clone(&observed);
    let outcome = Downloader::for_local_fixture()
        .unwrap()
        .download_asset(
            &manifest.assets[0],
            &partial,
            None,
            0,
            Arc::new(AtomicBool::new(false)),
            Arc::new(move |progress| {
                observed_callback
                    .lock()
                    .unwrap()
                    .push(progress.aggregate_downloaded_bytes)
            }),
        )
        .await
        .unwrap();
    assert_eq!(outcome.bytes, body.len() as u64);
    assert_eq!(fs::read(&partial).unwrap(), body);
    assert_eq!(
        observed.lock().unwrap().last().copied(),
        Some(body.len() as u64)
    );
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn downloader_resumes_only_a_valid_206_range_with_if_range() {
    let body = b"0123456789abcdefghijklmnopqrstuvwxyz".to_vec();
    let split = 10;
    let response = FixtureResponse {
        status: "206 Partial Content",
        headers: vec![
            ("ETag".to_string(), "\"resume-v1\"".to_string()),
            (
                "Content-Range".to_string(),
                format!("bytes {split}-{}/{}", body.len() - 1, body.len()),
            ),
        ],
        chunks: vec![body[split..].to_vec()],
        delay_ms: 0,
    };
    let (base_url, server) = fixture_server(vec![response]).await;
    let manifest = fixture_manifest(&base_url, &body, b"projector");
    let root = TestDirectory::new("download-resume");
    let (_, staging) = fixture_staging(root.path());
    let partial = staging.join(format!("{}.part", manifest.assets[0].filename));
    fs::write(&partial, &body[..split]).unwrap();
    Downloader::for_local_fixture()
        .unwrap()
        .download_asset(
            &manifest.assets[0],
            &partial,
            Some("\"resume-v1\"".to_string()),
            0,
            Arc::new(AtomicBool::new(false)),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
    let requests = server.await.unwrap();
    assert!(requests[0].contains("range: bytes=10-"));
    assert!(requests[0].contains("if-range: \"resume-v1\""));
    assert_eq!(fs::read(partial).unwrap(), body);
}

#[tokio::test]
async fn changed_validator_restarts_the_asset_without_reusing_partial_bytes() {
    let body = b"release-controlled-body".to_vec();
    let split = 7;
    let responses = vec![
        FixtureResponse {
            status: "206 Partial Content",
            headers: vec![
                ("ETag".to_string(), "\"new\"".to_string()),
                (
                    "Content-Range".to_string(),
                    format!("bytes {split}-{}/{}", body.len() - 1, body.len()),
                ),
            ],
            chunks: vec![body[split..].to_vec()],
            delay_ms: 0,
        },
        FixtureResponse {
            status: "200 OK",
            headers: vec![("ETag".to_string(), "\"new\"".to_string())],
            chunks: vec![body.clone()],
            delay_ms: 0,
        },
    ];
    let (base_url, server) = fixture_server(responses).await;
    let manifest = fixture_manifest(&base_url, &body, b"projector");
    let root = TestDirectory::new("validator-change");
    let (_, staging) = fixture_staging(root.path());
    let partial = staging.join(format!("{}.part", manifest.assets[0].filename));
    fs::write(&partial, &body[..split]).unwrap();
    Downloader::for_local_fixture()
        .unwrap()
        .download_asset(
            &manifest.assets[0],
            &partial,
            Some("\"old\"".to_string()),
            0,
            Arc::new(AtomicBool::new(false)),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(!requests[1].contains("range:"));
    assert_eq!(fs::read(partial).unwrap(), body);
}

#[tokio::test]
async fn cancellation_preserves_a_resumable_partial_and_stops_writes() {
    let body = vec![b'x'; 32 * 1024];
    let response = FixtureResponse {
        status: "200 OK",
        headers: vec![("ETag".to_string(), "\"cancel-v1\"".to_string())],
        chunks: vec![body[..4096].to_vec(), body[4096..].to_vec()],
        delay_ms: 75,
    };
    let (base_url, server) = fixture_server(vec![response]).await;
    let manifest = fixture_manifest(&base_url, &body, b"projector");
    let root = TestDirectory::new("cancel");
    let (_, staging) = fixture_staging(root.path());
    let partial = staging.join(format!("{}.part", manifest.assets[0].filename));
    let cancellation = Arc::new(AtomicBool::new(false));
    let callback_cancel = Arc::clone(&cancellation);
    let error = Downloader::for_local_fixture()
        .unwrap()
        .download_asset(
            &manifest.assets[0],
            &partial,
            None,
            0,
            Arc::clone(&cancellation),
            Arc::new(move |_| callback_cancel.store(true, Ordering::Release)),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "model_install_cancelled");
    let partial_length = fs::metadata(&partial).unwrap().len();
    assert!(partial_length > 0 && partial_length < body.len() as u64);
    let _ = server.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn downloader_refuses_a_swapped_partial_symlink_without_touching_target() {
    use std::os::unix::fs::symlink;

    let body = b"expected".to_vec();
    let manifest = fixture_manifest("http://127.0.0.1:9", &body, b"projector");
    let root = TestDirectory::new("partial-symlink");
    let (_, staging) = fixture_staging(root.path());
    let target = root.path().join("unrelated.txt");
    fs::write(&target, b"preserve me").unwrap();
    let partial = staging.join(format!("{}.part", manifest.assets[0].filename));
    symlink(&target, &partial).unwrap();
    let error = Downloader::for_local_fixture()
        .unwrap()
        .download_asset(
            &manifest.assets[0],
            &partial,
            None,
            0,
            Arc::new(AtomicBool::new(false)),
            Arc::new(|_| {}),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "model_install_partial_invalid");
    assert_eq!(fs::read(target).unwrap(), b"preserve me");
}

#[cfg(unix)]
#[tokio::test]
async fn downloader_refuses_a_partial_hard_link_without_touching_target() {
    let body = b"expected".to_vec();
    let manifest = fixture_manifest("http://127.0.0.1:9", &body, b"projector");
    let root = TestDirectory::new("partial-hard-link");
    let (_, staging) = fixture_staging(root.path());
    let target = root.path().join("unrelated.txt");
    fs::write(&target, b"preserve me").unwrap();
    let partial = staging.join(format!("{}.part", manifest.assets[0].filename));
    fs::hard_link(&target, &partial).unwrap();
    let error = Downloader::for_local_fixture()
        .unwrap()
        .download_asset(
            &manifest.assets[0],
            &partial,
            None,
            0,
            Arc::new(AtomicBool::new(false)),
            Arc::new(|_| {}),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "model_install_partial_invalid");
    assert_eq!(fs::read(target).unwrap(), b"preserve me");
}

#[tokio::test]
async fn hash_mismatch_removes_only_the_installer_owned_invalid_partial() {
    let expected = b"expected".to_vec();
    let wrong = b"xxxxxxxx".to_vec();
    let response = FixtureResponse {
        status: "200 OK",
        headers: vec![],
        chunks: vec![wrong],
        delay_ms: 0,
    };
    let (base_url, server) = fixture_server(vec![response]).await;
    let manifest = fixture_manifest(&base_url, &expected, b"projector");
    let root = TestDirectory::new("hash-mismatch");
    let unrelated = root.path().join("unrelated.txt");
    fs::write(&unrelated, b"preserve").unwrap();
    let (_, staging) = fixture_staging(root.path());
    let partial = staging.join(format!("{}.part", manifest.assets[0].filename));
    let error = Downloader::for_local_fixture()
        .unwrap()
        .download_asset(
            &manifest.assets[0],
            &partial,
            None,
            0,
            Arc::new(AtomicBool::new(false)),
            Arc::new(|_| {}),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "model_install_integrity_mismatch");
    assert!(!partial.exists());
    assert_eq!(fs::read(unrelated).unwrap(), b"preserve");
    let _ = server.await.unwrap();
}

#[tokio::test]
async fn verified_staging_promotes_atomically_and_exact_existing_package_adopts() {
    let primary = b"primary".to_vec();
    let projector = b"projector".to_vec();
    let manifest = fixture_manifest("http://127.0.0.1:9", &primary, &projector);
    let root = TestDirectory::new("promotion");
    let (_, staging) = fixture_staging(root.path());
    for (asset, bytes) in manifest.assets.iter().zip([&primary, &projector]) {
        let partial = staging.join(format!("{}.part", asset.filename));
        fs::write(&partial, bytes).unwrap();
        verify_asset_file(asset, &partial).await.unwrap();
    }
    let filenames = manifest
        .assets
        .iter()
        .map(|asset| asset.filename.clone())
        .collect::<Vec<_>>();
    seal_staging_assets(&staging, &filenames).unwrap();
    let promoted = promote_staging(root.path(), &staging, &filenames).unwrap();
    assert_eq!(promoted, final_directory(root.path()));
    assert!(!staging.exists());
    verify_exact_package(&manifest, &promoted).await.unwrap();
    assert_eq!(fs::read_dir(promoted).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn promotion_refuses_a_broken_symlink_at_the_canonical_path() {
    use std::os::unix::fs::symlink;

    let manifest = fixture_manifest("http://127.0.0.1:9", b"primary", b"projector");
    let root = TestDirectory::new("promotion-broken-symlink");
    let (_, staging) = fixture_staging(root.path());
    for (asset, bytes) in manifest
        .assets
        .iter()
        .zip([b"primary".as_slice(), b"projector".as_slice()])
    {
        fs::write(staging.join(&asset.filename), bytes).unwrap();
    }
    let canonical = final_directory(root.path());
    symlink(root.path().join("missing-target"), &canonical).unwrap();
    let filenames = manifest
        .assets
        .iter()
        .map(|asset| asset.filename.clone())
        .collect::<Vec<_>>();

    assert_eq!(
        promote_staging(root.path(), &staging, &filenames)
            .unwrap_err()
            .code,
        "model_install_destination_collision"
    );
    assert!(fs::symlink_metadata(canonical)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn staging_with_any_extra_entry_is_never_promoted() {
    let manifest = fixture_manifest("http://127.0.0.1:9", b"primary", b"projector");
    let root = TestDirectory::new("promotion-extra");
    let (_, staging) = fixture_staging(root.path());
    let filenames = manifest
        .assets
        .iter()
        .map(|asset| asset.filename.clone())
        .collect::<Vec<_>>();
    for (asset, bytes) in manifest
        .assets
        .iter()
        .zip([b"primary".as_slice(), b"projector".as_slice()])
    {
        fs::write(staging.join(&asset.filename), bytes).unwrap();
    }
    fs::write(staging.join("unexpected.txt"), b"not part of package").unwrap();
    assert_eq!(
        validate_exact_staging_entries(&staging, &filenames)
            .unwrap_err()
            .code,
        "model_install_staging_not_exact"
    );
    assert_eq!(
        promote_staging(root.path(), &staging, &filenames)
            .unwrap_err()
            .code,
        "model_install_staging_not_exact"
    );
    assert!(!final_directory(root.path()).exists());
}

#[cfg(unix)]
#[test]
fn staging_with_multiply_linked_asset_is_never_promoted() {
    let manifest = fixture_manifest("http://127.0.0.1:9", b"primary", b"projector");
    let root = TestDirectory::new("promotion-hard-link");
    let (_, staging) = fixture_staging(root.path());
    let filenames = manifest
        .assets
        .iter()
        .map(|asset| asset.filename.clone())
        .collect::<Vec<_>>();
    let unrelated = root.path().join("unrelated.gguf");
    fs::write(&unrelated, b"primary").unwrap();
    fs::hard_link(&unrelated, staging.join(&manifest.assets[0].filename)).unwrap();
    fs::write(staging.join(&manifest.assets[1].filename), b"projector").unwrap();

    assert_eq!(
        validate_exact_staging_entries(&staging, &filenames)
            .unwrap_err()
            .code,
        "model_install_staging_not_exact"
    );
    assert_eq!(fs::read(&unrelated).unwrap(), b"primary");
    assert!(!final_directory(root.path()).exists());
}

#[test]
fn journal_round_trip_recovers_phase_without_urls_or_credentials() {
    let root = TestDirectory::new("journal");
    let path = root.path().join("journal/install-state.json");
    let mut journal = InstallJournal::new(
        &recommended_model_manifest(),
        new_opaque_id("install_"),
        root.path().join("models"),
        DestinationKind::Managed,
        10,
    );
    journal.phase = InstallPhase::Cancelled;
    journal.assets[0].downloaded_bytes = 4096;
    journal.assets[0].etag = Some("\"safe-validator\"".to_string());
    save_journal(&path, &journal).unwrap();
    let recovered = load_journal(&path).unwrap().unwrap();
    assert_eq!(recovered.phase, InstallPhase::Cancelled);
    assert_eq!(recovered.assets[0].downloaded_bytes, 4096);
    let encoded = fs::read_to_string(path).unwrap();
    assert!(!encoded.contains("huggingface.co"));
    assert!(!encoded.contains("Authorization"));
}

#[cfg(unix)]
#[test]
fn journal_symlinks_and_hard_links_are_refused() {
    use std::os::unix::fs::symlink;

    let root = TestDirectory::new("journal-links");
    let target = root.path().join("unrelated.json");
    fs::write(&target, b"{}").unwrap();
    let symlink_path = root.path().join("journal-symlink.json");
    symlink(&target, &symlink_path).unwrap();
    assert_eq!(
        load_journal(&symlink_path).unwrap_err().code,
        "model_install_journal_invalid"
    );

    let hard_link_path = root.path().join("journal-hard-link.json");
    fs::hard_link(&target, &hard_link_path).unwrap();
    assert_eq!(
        load_journal(&hard_link_path).unwrap_err().code,
        "model_install_journal_invalid"
    );
    assert_eq!(fs::read(&target).unwrap(), b"{}");
}

#[test]
fn single_flight_reattaches_repeated_start_instead_of_spawning() {
    let root = TestDirectory::new("single-flight");
    let installer = RecommendedModelInstaller::for_test(
        root.path().join("models"),
        root.path().join("app-data"),
        Arc::new(FixtureInspector),
        Arc::new(FixtureFinalizer),
    )
    .unwrap();
    let install_id = new_opaque_id("install_");
    {
        let mut runtime = installer.runtime.lock().unwrap();
        runtime.active = true;
        runtime.progress = Some(InstallProgress::new(
            install_id.clone(),
            PACKAGE_TOTAL_BYTES,
        ));
        runtime.cancellation = Some(Arc::new(AtomicBool::new(false)));
    }
    let response = installer
        .start(None, Arc::new(RecordingSink::default()))
        .unwrap();
    assert!(response.attached);
    assert_eq!(response.install_id, install_id);
}

#[test]
fn corrupt_journal_degrades_to_repair_state_without_startup_failure() {
    let root = TestDirectory::new("corrupt-journal");
    let journal = root
        .path()
        .join("app-data")
        .join(super::JOURNAL_RELATIVE_PATH);
    fs::create_dir_all(journal.parent().unwrap()).unwrap();
    fs::File::create(&journal)
        .unwrap()
        .write_all(b"not-json")
        .unwrap();
    let installer = RecommendedModelInstaller::new(
        root.path().join("models"),
        root.path().join("app-data"),
        Arc::new(FixtureFinalizer),
    );
    let state = installer.state();
    assert_eq!(state.package_state, InstallPhase::RepairRequired);
    assert_eq!(
        state.active_install.unwrap().public_error_code.as_deref(),
        Some("model_install_journal_invalid")
    );
}

#[test]
fn startup_probe_surfaces_a_size_exact_package_as_adoptable() {
    let root = TestDirectory::new("adoptable-startup");
    let managed = root.path().join("models");
    write_sparse_release_shape(&managed);
    let installer = RecommendedModelInstaller::for_test(
        managed,
        root.path().join("app-data"),
        Arc::new(FixtureInspector),
        Arc::new(FixtureFinalizer),
    )
    .unwrap();

    let state = installer.state();
    assert_eq!(state.package_state, InstallPhase::Adoptable);
    assert!(state.active_install.is_none());
    assert!(state.receipt.is_none());
}

#[test]
fn ready_receipt_is_withheld_after_same_length_package_tampering() {
    let root = TestDirectory::new("ready-package-tampered");
    let managed = root.path().join("models");
    write_sparse_release_shape(&managed);
    let installer = RecommendedModelInstaller::for_test(
        managed.clone(),
        root.path().join("app-data"),
        Arc::new(FixtureInspector),
        Arc::new(FixtureFinalizer),
    )
    .unwrap();
    let manifest = recommended_model_manifest();
    let mut journal = InstallJournal::new(
        &manifest,
        new_opaque_id("install_"),
        managed.clone(),
        DestinationKind::Managed,
        1,
    );
    journal.phase = InstallPhase::Ready;
    journal.receipt = Some(RecommendedModelInstallReceipt::completed(
        new_opaque_id("model_receipt_"),
        &manifest,
        RuntimeInspectionEvidence {
            accepted: true,
            architecture: "fixture".to_string(),
            tensor_count: 1,
            model_bytes: manifest.primary_asset().bytes,
            multimodal_projector_count: 1,
        },
        CompletedProviderEvidence::verified_local("local-model", None),
        package_identity_sha256(&managed, &manifest).unwrap(),
        1,
        2,
    ));
    let progress = super::job::progress_from_journal(&journal);
    {
        let mut runtime = installer.runtime.lock().unwrap();
        runtime.journal = Some(journal);
        runtime.progress = Some(progress);
    }
    assert_eq!(installer.state().package_state, InstallPhase::Ready);

    let primary = final_directory(&managed).join(&manifest.primary_asset().filename);
    let mut file = fs::OpenOptions::new().write(true).open(primary).unwrap();
    file.write_all(b"x").unwrap();
    file.sync_all().unwrap();

    let state = installer.state();
    assert_eq!(state.package_state, InstallPhase::Adoptable);
    assert!(state.receipt.is_none());
    assert!(state
        .active_install
        .is_some_and(|progress| progress.completed_provider.is_none()));
}

#[test]
fn ready_journal_with_missing_package_is_reported_as_repair_required() {
    let root = TestDirectory::new("ready-package-missing");
    let managed = root.path().join("models");
    let installer = RecommendedModelInstaller::for_test(
        managed.clone(),
        root.path().join("app-data"),
        Arc::new(FixtureInspector),
        Arc::new(FixtureFinalizer),
    )
    .unwrap();
    let mut journal = InstallJournal::new(
        &recommended_model_manifest(),
        new_opaque_id("install_"),
        managed,
        DestinationKind::Managed,
        1,
    );
    journal.phase = InstallPhase::Ready;
    let progress = super::job::progress_from_journal(&journal);
    {
        let mut runtime = installer.runtime.lock().unwrap();
        runtime.journal = Some(journal);
        runtime.progress = Some(progress);
    }

    let state = installer.state();
    assert_eq!(state.package_state, InstallPhase::RepairRequired);
    let active = state.active_install.unwrap();
    assert_eq!(active.state, InstallPhase::RepairRequired);
    assert_eq!(
        active.public_error_code.as_deref(),
        Some("model_install_ready_package_missing")
    );
    assert!(active.completed_provider.is_none());
    assert!(state.receipt.is_none());
}

#[test]
fn ready_journal_with_damaged_package_shape_is_reported_as_repair_required() {
    let root = TestDirectory::new("ready-package-damaged");
    let managed = root.path().join("models");
    let package = final_directory(&managed);
    fs::create_dir_all(&package).unwrap();
    for asset in recommended_model_manifest().assets {
        fs::write(package.join(asset.filename), b"damaged").unwrap();
    }
    let installer = RecommendedModelInstaller::for_test(
        managed.clone(),
        root.path().join("app-data"),
        Arc::new(FixtureInspector),
        Arc::new(FixtureFinalizer),
    )
    .unwrap();
    let mut journal = InstallJournal::new(
        &recommended_model_manifest(),
        new_opaque_id("install_"),
        managed,
        DestinationKind::Managed,
        1,
    );
    journal.phase = InstallPhase::Ready;
    let progress = super::job::progress_from_journal(&journal);
    {
        let mut runtime = installer.runtime.lock().unwrap();
        runtime.journal = Some(journal);
        runtime.progress = Some(progress);
    }

    let state = installer.state();
    assert_eq!(state.package_state, InstallPhase::RepairRequired);
    assert_eq!(
        state.active_install.unwrap().public_error_code.as_deref(),
        Some("model_install_package_shape_invalid")
    );
    assert!(state.receipt.is_none());
}

#[test]
fn repair_journal_can_switch_destination_without_deleting_ambiguous_data() {
    let root = TestDirectory::new("repair-destination-switch");
    let managed = root.path().join("models");
    let custom = root.path().join("custom");
    fs::create_dir_all(&custom).unwrap();
    let mut installer = RecommendedModelInstaller::for_test(
        managed.clone(),
        root.path().join("app-data"),
        Arc::new(FixtureInspector),
        Arc::new(FixtureFinalizer),
    )
    .unwrap();
    let ambiguous = final_directory(&managed);
    fs::create_dir_all(&ambiguous).unwrap();
    fs::write(ambiguous.join("unrecognized-user-data.txt"), b"preserve").unwrap();
    let mut journal = InstallJournal::new(
        &recommended_model_manifest(),
        new_opaque_id("install_"),
        managed,
        DestinationKind::Managed,
        1,
    );
    journal.phase = InstallPhase::RepairRequired;
    let progress = super::job::progress_from_journal(&journal);
    {
        let mut runtime = installer.runtime.lock().unwrap();
        runtime.journal = Some(journal);
        runtime.progress = Some(progress);
    }
    let grant = installer.destination.issue_grant(&custom).unwrap();
    installer.downloader = None;

    let error = installer
        .start(Some(grant.grant_id), Arc::new(RecordingSink::default()))
        .unwrap_err();
    assert_eq!(error.code, "model_install_transport_unavailable");
    assert_eq!(
        fs::read(ambiguous.join("unrecognized-user-data.txt")).unwrap(),
        b"preserve"
    );
}

fn write_sparse_release_shape(root: &Path) {
    let package = final_directory(root);
    fs::create_dir_all(&package).unwrap();
    for asset in recommended_model_manifest().assets {
        let file = fs::File::create(package.join(asset.filename)).unwrap();
        file.set_len(asset.bytes).unwrap();
    }
}

#[tokio::test]
async fn invalid_ranges_and_private_networks_are_typed_failures() {
    let headers = HeaderMap::new();
    assert_eq!(
        validate_content_range(&headers, 4, 10).unwrap_err().code,
        "model_install_content_range_invalid"
    );
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_RANGE, HeaderValue::from_static("bytes 3-9/10"));
    assert_eq!(
        validate_content_range(&headers, 4, 10).unwrap_err().code,
        "model_install_content_range_invalid"
    );
    let url = url::Url::parse("https://127.0.0.1/model.gguf").unwrap();
    assert_eq!(
        validate_network_destination(&url, false)
            .await
            .unwrap_err()
            .code,
        "model_install_private_network_refused"
    );
}
