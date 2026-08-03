use crate::single_instance_contract::{
    ActivationMessage, HolderRecord, InstanceIdentity, SingleInstanceReceipt,
};

#[derive(Debug)]
pub(crate) struct SingleInstanceFailure {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
}

impl SingleInstanceFailure {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    use std::{
        ffi::{CStr, OsStr},
        fs::{self, File, OpenOptions},
        io::{Read, Seek, SeekFrom, Write},
        mem::MaybeUninit,
        os::{
            fd::AsRawFd,
            unix::{
                ffi::OsStrExt,
                fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
                net::UnixDatagram,
            },
        },
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc, Mutex,
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };

    const INSTANCE_LOCK_FILE: &str = "oomu-single-instance.lock";
    const ACTIVATION_SOCKET_PREFIX: &str = "oomu-a-";
    const MAX_HOLDER_BYTES: u64 = 8 * 1024;
    const MAX_ACTIVATION_BYTES: usize = 512;
    const ACTIVATION_READ_TIMEOUT: Duration = Duration::from_millis(250);
    const ACTIVATION_SHUTDOWN_WAIT: Duration = Duration::from_secs(3);
    const HOLDER_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(120);
    const HOLDER_PUBLICATION_POLL: Duration = Duration::from_millis(25);

    #[derive(Debug)]
    pub(crate) struct InstanceLease {
        file: File,
        identity: InstanceIdentity,
    }

    impl InstanceLease {
        pub(crate) fn identity(&self) -> &InstanceIdentity {
            &self.identity
        }
    }

    impl Drop for InstanceLease {
        fn drop(&mut self) {
            // SAFETY: the descriptor remains owned by this process-lifetime lease.
            let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
    }

    pub(crate) enum InstanceClaim {
        Primary(File),
        Secondary { file: File, holder: HolderRecord },
    }

    impl InstanceClaim {
        pub(crate) fn is_primary(&self) -> bool {
            matches!(self, Self::Primary(_))
        }
    }

    enum LockAttempt {
        Primary(File),
        Secondary(File),
    }

    pub(crate) fn claim() -> Result<InstanceClaim, SingleInstanceFailure> {
        let lock_path = instance_lock_path().map_err(runtime_failure)?;
        match acquire_lock(&lock_path).map_err(runtime_failure)? {
            LockAttempt::Primary(mut file) => {
                write_holder_record(&mut file, HolderRecord::verifying())
                    .map_err(runtime_failure)?;
                emit_boundary("primary_claimed");
                Ok(InstanceClaim::Primary(file))
            }
            LockAttempt::Secondary(file) => wait_for_holder_or_reclaim(
                file,
                HOLDER_PUBLICATION_TIMEOUT,
                HOLDER_PUBLICATION_POLL,
            ),
        }
    }

    pub(crate) fn complete(
        claim: InstanceClaim,
        identity: InstanceIdentity,
    ) -> Result<Option<InstanceLease>, SingleInstanceFailure> {
        match claim {
            InstanceClaim::Primary(mut file) => {
                write_holder_record(&mut file, HolderRecord::current(identity.clone()))
                    .map_err(runtime_failure)?;
                emit_receipt("primary_acquired", &identity, Some(std::process::id()));
                emit_boundary("verified_holder_published");
                Ok(Some(InstanceLease { file, identity }))
            }
            InstanceClaim::Secondary { mut file, holder } => {
                confirm_secondary_holder(&mut file, &holder)?;
                handle_secondary(identity, holder)
            }
        }
    }

    fn handle_secondary(
        identity: InstanceIdentity,
        holder: HolderRecord,
    ) -> Result<Option<InstanceLease>, SingleInstanceFailure> {
        let Some((holder_pid, holder_identity)) = holder.ready() else {
            emit_receipt("holder_unreadable", &identity, None);
            return Err(holder_unreadable_failure());
        };
        if !identity.matches_holder(holder_identity) {
            emit_receipt("identity_mismatch_rejected", &identity, Some(holder_pid));
            return Err(SingleInstanceFailure::new(
                "single_instance_identity_mismatch",
                format!(
                    "the active holder namespace {} does not match launch namespace {}",
                    holder_identity.namespace, identity.namespace
                ),
            ));
        }
        let signalled = signal_existing_instance(&identity).unwrap_or(false);
        let activated = activate_exact_holder(holder_pid);
        if !signalled && !activated {
            emit_receipt("activation_failed", &identity, Some(holder_pid));
            return Err(SingleInstanceFailure::new(
                "single_instance_activation_failed",
                format!(
                    "the verified holder process {} could not be activated",
                    holder_pid
                ),
            ));
        }
        emit_receipt("existing_instance_activated", &identity, Some(holder_pid));
        Ok(None)
    }

    pub(crate) struct SingleInstanceActivationRuntime {
        shutdown: Arc<AtomicBool>,
        socket_path: PathBuf,
        worker: Mutex<Option<(JoinHandle<()>, mpsc::Receiver<()>)>>,
    }

    impl SingleInstanceActivationRuntime {
        pub(crate) fn shutdown(&self) -> Result<(), String> {
            self.shutdown.store(true, Ordering::Release);
            if let Ok(wake) = UnixDatagram::unbound() {
                let _ = wake.send_to(b"shutdown", &self.socket_path);
            }
            let mut worker = self
                .worker
                .lock()
                .map_err(|_| "single_instance_listener_shutdown_lock_failed".to_string())?;
            let Some((handle, completion)) = worker.take() else {
                return Ok(());
            };
            match completion.recv_timeout(ACTIVATION_SHUTDOWN_WAIT) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    handle
                        .join()
                        .map_err(|_| "single_instance_listener_join_panicked".to_string())?;
                    remove_stale_activation_socket(&self.socket_path)
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    *worker = Some((handle, completion));
                    Err("single_instance_listener_join_timeout".to_string())
                }
            }
        }
    }

    pub(crate) fn install_activation_listener(
        app: tauri::AppHandle,
        identity: InstanceIdentity,
    ) -> Result<SingleInstanceActivationRuntime, String> {
        let socket_path = activation_socket_path(&identity)?;
        remove_stale_activation_socket(&socket_path)?;
        let socket = UnixDatagram::bind(&socket_path)
            .map_err(|error| format!("unable to create activation channel: {error}"))?;
        socket
            .set_read_timeout(Some(ACTIVATION_READ_TIMEOUT))
            .map_err(|error| format!("unable to bound activation channel reads: {error}"))?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("unable to secure activation channel: {error}"))?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let listener_shutdown = Arc::clone(&shutdown);
        let (completed, completion) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("oomu-single-instance-activation".to_string())
            .spawn(move || {
                let mut bytes = [0_u8; MAX_ACTIVATION_BYTES];
                while !listener_shutdown.load(Ordering::Acquire) {
                    let received = match socket.recv(&mut bytes) {
                        Ok(received) => received,
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            continue
                        }
                        Err(_) => break,
                    };
                    if listener_shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(message) =
                        serde_json::from_slice::<ActivationMessage>(&bytes[..received])
                    else {
                        continue;
                    };
                    if !message.accepts(&identity) {
                        continue;
                    }
                    let foreground_app = app.clone();
                    if let Err(error) = app.run_on_main_thread(move || {
                        crate::background_runtime_tray::restore_foreground(&foreground_app);
                    }) {
                        eprintln!("OOMU_SINGLE_INSTANCE_ACTIVATION_FAILED error={error}");
                    }
                }
                let _ = completed.send(());
            })
            .map_err(|error| format!("unable to start activation channel: {error}"))?;
        Ok(SingleInstanceActivationRuntime {
            shutdown,
            socket_path,
            worker: Mutex::new(Some((handle, completion))),
        })
    }

    fn runtime_failure(detail: String) -> SingleInstanceFailure {
        SingleInstanceFailure::new("single_instance_runtime_failure", detail)
    }

    fn holder_unreadable_failure() -> SingleInstanceFailure {
        SingleInstanceFailure::new(
            "single_instance_holder_unreadable",
            "the active process lock did not publish a supported verified holder identity",
        )
    }

    fn emit_boundary(state: &str) {
        let at_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        eprintln!(
            "OOMU_STARTUP_MILESTONE milestone=single_instance_{state} at_unix_ms={at_unix_ms}"
        );
    }

    fn emit_receipt(decision: &str, identity: &InstanceIdentity, holder_pid: Option<u32>) {
        let receipt = SingleInstanceReceipt::new(decision, identity, holder_pid);
        if let Ok(encoded) = serde_json::to_string(&receipt) {
            eprintln!("OOMU_NATIVE_RECEIPT {encoded}");
        }
    }

    fn acquire_lock(lock_path: &Path) -> Result<LockAttempt, String> {
        let file = open_verified_lock_file(lock_path)?;
        // SAFETY: the file is a verified regular descriptor and LOCK_NB cannot block.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(LockAttempt::Primary(file));
        }

        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
            return Err(format!("unable to acquire instance boundary: {error}"));
        }
        Ok(LockAttempt::Secondary(file))
    }

    fn wait_for_holder_or_reclaim(
        mut file: File,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<InstanceClaim, SingleInstanceFailure> {
        let deadline = Instant::now() + timeout;
        loop {
            // SAFETY: the verified regular descriptor remains open and LOCK_NB cannot block.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                write_holder_record(&mut file, HolderRecord::verifying())
                    .map_err(runtime_failure)?;
                emit_boundary("primary_reclaimed");
                return Ok(InstanceClaim::Primary(file));
            }
            let lock_error = std::io::Error::last_os_error();
            if lock_error.raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(runtime_failure(format!(
                    "unable to retry instance boundary: {lock_error}"
                )));
            }

            match read_holder_record(&mut file).map_err(runtime_failure)? {
                HolderRead::Record(holder) if !holder.is_supported() => {
                    return Err(holder_unreadable_failure());
                }
                HolderRead::Record(holder @ HolderRecord::Ready { .. }) => {
                    emit_boundary("secondary_verified_holder_observed");
                    return Ok(InstanceClaim::Secondary { file, holder });
                }
                HolderRead::Record(HolderRecord::Verifying { .. }) | HolderRead::Empty => {}
                HolderRead::Malformed => return Err(holder_unreadable_failure()),
            }
            if Instant::now() >= deadline {
                return Err(holder_unreadable_failure());
            }
            thread::sleep(poll_interval);
        }
    }

    fn confirm_secondary_holder(
        file: &mut File,
        expected: &HolderRecord,
    ) -> Result<(), SingleInstanceFailure> {
        // SAFETY: the verified regular descriptor remains open and LOCK_NB cannot block.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            // SAFETY: this process acquired the descriptor lock above.
            let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
            return Err(holder_unreadable_failure());
        }
        let lock_error = std::io::Error::last_os_error();
        if lock_error.raw_os_error() != Some(libc::EWOULDBLOCK) {
            return Err(runtime_failure(format!(
                "unable to confirm instance boundary: {lock_error}"
            )));
        }
        match read_holder_record(file).map_err(runtime_failure)? {
            HolderRead::Record(current) if current == *expected && current.ready().is_some() => {
                Ok(())
            }
            _ => Err(holder_unreadable_failure()),
        }
    }

    fn open_verified_lock_file(lock_path: &Path) -> Result<File, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(lock_path)
            .map_err(|error| format!("unable to open instance boundary: {error}"))?;
        verify_and_secure_lock_descriptor(&file)?;
        Ok(file)
    }

    fn verify_and_secure_lock_descriptor(file: &File) -> Result<(), String> {
        let mut status = MaybeUninit::<libc::stat>::zeroed();
        // SAFETY: status is writable and the descriptor remains valid.
        if unsafe { libc::fstat(file.as_raw_fd(), status.as_mut_ptr()) } != 0 {
            return Err(format!(
                "unable to inspect instance boundary: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: successful fstat initialized the structure.
        let status = unsafe { status.assume_init() };
        if status.st_mode & libc::S_IFMT != libc::S_IFREG {
            return Err("instance boundary is not a regular file".to_string());
        }
        // SAFETY: geteuid has no preconditions.
        if status.st_uid != unsafe { libc::geteuid() } {
            return Err("instance boundary belongs to another user".to_string());
        }
        if status.st_nlink != 1 {
            return Err("instance boundary has an unsafe link count".to_string());
        }
        // SAFETY: the descriptor names the inode verified above.
        if unsafe { libc::fchmod(file.as_raw_fd(), 0o600) } != 0 {
            return Err(format!(
                "unable to secure instance boundary: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    fn write_holder_record(file: &mut File, holder: HolderRecord) -> Result<(), String> {
        let encoded = serde_json::to_vec(&holder)
            .map_err(|error| format!("unable to encode instance holder: {error}"))?;
        file.set_len(0)
            .map_err(|error| format!("unable to reset instance holder: {error}"))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| format!("unable to position instance holder: {error}"))?;
        file.write_all(&encoded)
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.sync_data())
            .map_err(|error| format!("unable to commit instance holder: {error}"))
    }

    #[derive(Debug, Eq, PartialEq)]
    enum HolderRead {
        Empty,
        Record(HolderRecord),
        Malformed,
    }

    fn read_holder_record(file: &mut File) -> Result<HolderRead, String> {
        let metadata = file
            .metadata()
            .map_err(|error| format!("unable to inspect instance holder: {error}"))?;
        if metadata.len() == 0 {
            return Ok(HolderRead::Empty);
        }
        if metadata.len() > MAX_HOLDER_BYTES {
            return Ok(HolderRead::Malformed);
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|error| format!("unable to position instance holder: {error}"))?;
        let mut contents = String::new();
        file.take(MAX_HOLDER_BYTES)
            .read_to_string(&mut contents)
            .map_err(|error| format!("unable to read instance holder: {error}"))?;
        Ok(serde_json::from_str(contents.trim())
            .map(HolderRead::Record)
            .unwrap_or(HolderRead::Malformed))
    }

    fn darwin_user_temp_dir() -> Result<PathBuf, String> {
        // SAFETY: a null buffer asks confstr for the required size.
        let required =
            unsafe { libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, std::ptr::null_mut(), 0) };
        if required == 0 {
            return Err(format!(
                "unable to locate private runtime directory: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut buffer = vec![0_u8; required];
        // SAFETY: the buffer has the size returned by confstr.
        let written = unsafe {
            libc::confstr(
                libc::_CS_DARWIN_USER_TEMP_DIR,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if written == 0 || written > buffer.len() {
            return Err("macOS returned an invalid private runtime directory".to_string());
        }
        let path = CStr::from_bytes_until_nul(&buffer)
            .map_err(|_| "macOS returned an invalid private runtime directory".to_string())?;
        Ok(PathBuf::from(OsStr::from_bytes(path.to_bytes())))
    }

    fn instance_lock_path() -> Result<PathBuf, String> {
        Ok(instance_lock_path_from(&darwin_user_temp_dir()?))
    }

    fn instance_lock_path_from(runtime_dir: &Path) -> PathBuf {
        runtime_dir.join(INSTANCE_LOCK_FILE)
    }

    fn activation_socket_path(identity: &InstanceIdentity) -> Result<PathBuf, String> {
        Ok(activation_socket_path_from(
            &darwin_user_temp_dir()?,
            &identity.namespace,
        ))
    }

    fn activation_socket_path_from(runtime_dir: &Path, namespace: &str) -> PathBuf {
        let short_namespace = &namespace[..namespace.len().min(12)];
        runtime_dir.join(format!("{ACTIVATION_SOCKET_PREFIX}{short_namespace}.sock"))
    }

    fn remove_stale_activation_socket(socket_path: &Path) -> Result<(), String> {
        let metadata = match fs::symlink_metadata(socket_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("unable to inspect activation channel: {error}")),
        };
        // SAFETY: geteuid has no preconditions.
        if !metadata.file_type().is_socket()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
        {
            return Err("activation channel is not a safe stale socket".to_string());
        }
        fs::remove_file(socket_path)
            .map_err(|error| format!("unable to replace stale activation channel: {error}"))
    }

    fn signal_existing_instance(identity: &InstanceIdentity) -> Result<bool, String> {
        let socket = UnixDatagram::unbound()
            .map_err(|error| format!("unable to open activation request: {error}"))?;
        let message = serde_json::to_vec(&ActivationMessage::for_identity(identity))
            .map_err(|error| format!("unable to encode activation request: {error}"))?;
        match socket.send_to(&message, activation_socket_path(identity)?) {
            Ok(written) => Ok(written == message.len()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(format!("unable to signal existing instance: {error}")),
        }
    }

    fn activate_exact_holder(holder_pid: u32) -> bool {
        let Ok(pid) = libc::pid_t::try_from(holder_pid) else {
            return false;
        };
        if pid <= 0 || pid == std::process::id() as libc::pid_t {
            return false;
        }
        let Some(application) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        else {
            return false;
        };
        if application.isTerminated() {
            return false;
        }
        let _ = application.unhide();
        let _ = application.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
        true
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{ffi::CString, os::unix::fs::symlink, sync::mpsc};

        fn test_root(label: &str) -> PathBuf {
            std::env::temp_dir().join(format!(
                "oomu-single-instance-{label}-{}-{}",
                std::process::id(),
                crate::foundation::clock::unix_time_ms_i64()
            ))
        }

        fn identity(build_number: u64) -> InstanceIdentity {
            let process = crate::macos_process_identity::MacosProcessIdentityEvidence {
                requesting_process: "oomu".to_string(),
                release_channel: "development",
                bundle_identifier: Some("ai.eldris.oomu.gpd.development".to_string()),
                team_id: None,
                signing_authority: None,
                build_number,
                code_directory_hash: Some(format!("code-{build_number}")),
                executable_sha256: Some(format!("exe-{build_number}")),
                signature_artifact_sha256: Some("a".repeat(64)),
                signature_verification_exit_status: Some(0),
                signature_verification_failure_code: None,
                designated_requirement_sha256: None,
                hardened_runtime: true,
                strict_signature_valid: true,
            };
            InstanceIdentity::from_process(
                &process,
                crate::runtime_profile::RuntimeProfileClass::Development,
            )
        }

        #[test]
        fn identity_paths_ignore_selected_app_data_roots() {
            let runtime_dir = Path::new("/private/var/folders/user/T");
            let first_profile = Path::new("/private/tmp/oomu-debug-profile");
            let second_profile = Path::new("/Users/tester/Library/Application Support/OOMU");
            let lock_path = instance_lock_path_from(runtime_dir);
            let socket = activation_socket_path_from(runtime_dir, &identity(1).namespace);

            assert_eq!(lock_path, runtime_dir.join(INSTANCE_LOCK_FILE));
            assert!(socket
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(ACTIVATION_SOCKET_PREFIX));
            assert!(!lock_path.starts_with(first_profile));
            assert!(!lock_path.starts_with(second_profile));
        }

        #[test]
        fn symlink_lock_is_rejected_without_touching_its_target() {
            let root = test_root("symlink");
            fs::create_dir_all(&root).unwrap();
            let target = root.join("target");
            let lock_path = root.join("lock");
            fs::write(&target, b"keep-me").unwrap();
            symlink(&target, &lock_path).unwrap();

            let error = open_verified_lock_file(&lock_path).unwrap_err();
            assert!(error.contains("unable to open"));
            assert_eq!(fs::read(&target).unwrap(), b"keep-me");
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn hard_link_lock_is_rejected_without_touching_its_target() {
            let root = test_root("hard-link");
            fs::create_dir_all(&root).unwrap();
            let target = root.join("target");
            let lock_path = root.join("lock");
            fs::write(&target, b"keep-me").unwrap();
            fs::hard_link(&target, &lock_path).unwrap();

            let error = open_verified_lock_file(&lock_path).unwrap_err();
            assert!(error.contains("unsafe link count"));
            assert_eq!(fs::read(&target).unwrap(), b"keep-me");
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn fifo_lock_is_rejected_without_blocking() {
            let root = test_root("fifo");
            fs::create_dir_all(&root).unwrap();
            let lock_path = root.join("lock");
            let lock_path_c = CString::new(lock_path.as_os_str().as_bytes()).unwrap();
            // SAFETY: the path is a valid C string inside the private test root.
            assert_eq!(unsafe { libc::mkfifo(lock_path_c.as_ptr(), 0o600) }, 0);

            let error = open_verified_lock_file(&lock_path).unwrap_err();
            assert!(error.contains("not a regular file"));
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn fast_primary_claim_publishes_verifying_before_the_complete_identity() {
            let root = test_root("exclusive");
            fs::create_dir_all(&root).unwrap();
            let lock_path = root.join("lock");
            let expected = identity(9);
            let LockAttempt::Primary(mut primary) = acquire_lock(&lock_path).unwrap() else {
                panic!("first claimant must own the fast boundary");
            };
            write_holder_record(&mut primary, HolderRecord::verifying()).unwrap();
            let LockAttempt::Secondary(mut secondary) = acquire_lock(&lock_path).unwrap() else {
                panic!("second claimant must observe the active boundary");
            };
            let HolderRead::Record(verifying) = read_holder_record(&mut secondary).unwrap() else {
                panic!("primary must publish a parseable verifying record");
            };
            assert!(verifying.is_supported());
            assert!(verifying.ready().is_none());

            write_holder_record(&mut primary, HolderRecord::current(expected.clone())).unwrap();
            let HolderRead::Record(ready) = read_holder_record(&mut secondary).unwrap() else {
                panic!("primary must publish a complete holder identity");
            };
            assert_eq!(ready.ready(), Some((std::process::id(), &expected)));
            assert_eq!(fs::metadata(&lock_path).unwrap().mode() & 0o777, 0o600);
            drop(primary);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn secondary_waits_without_activating_verifying_then_observes_ready() {
            let root = test_root("verifying-wait");
            fs::create_dir_all(&root).unwrap();
            let lock_path = root.join("lock");
            let expected = identity(302);
            let LockAttempt::Primary(mut primary) = acquire_lock(&lock_path).unwrap() else {
                panic!("first claimant must be primary");
            };
            write_holder_record(&mut primary, HolderRecord::verifying()).unwrap();
            let LockAttempt::Secondary(secondary) = acquire_lock(&lock_path).unwrap() else {
                panic!("second claimant must wait");
            };
            let (sent, received) = mpsc::channel();
            let waiter = thread::spawn(move || {
                sent.send(wait_for_holder_or_reclaim(
                    secondary,
                    Duration::from_secs(1),
                    Duration::from_millis(5),
                ))
                .unwrap();
            });
            thread::sleep(Duration::from_millis(25));
            assert!(matches!(
                received.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            write_holder_record(&mut primary, HolderRecord::current(expected.clone())).unwrap();

            let claim = received
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap();
            let InstanceClaim::Secondary { mut file, holder } = claim else {
                panic!("live primary publication must remain authoritative");
            };
            assert_eq!(holder.ready(), Some((std::process::id(), &expected)));
            confirm_secondary_holder(&mut file, &holder).unwrap();
            waiter.join().unwrap();
            drop(primary);
            assert_eq!(
                confirm_secondary_holder(&mut file, &holder)
                    .unwrap_err()
                    .code,
                "single_instance_holder_unreadable"
            );
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn next_build_reclaims_only_after_the_current_build_releases_the_lock() {
            let root = test_root("build-handoff");
            fs::create_dir_all(&root).unwrap();
            let lock_path = root.join("lock");
            let current = identity(302);
            let LockAttempt::Primary(mut first) = acquire_lock(&lock_path).unwrap() else {
                panic!("first build must own the global lease");
            };
            write_holder_record(&mut first, HolderRecord::current(current)).unwrap();
            let LockAttempt::Secondary(blocked) = acquire_lock(&lock_path).unwrap() else {
                panic!("the next build must observe the one global holder");
            };

            drop(first);
            let successor = wait_for_holder_or_reclaim(
                blocked,
                Duration::from_millis(50),
                Duration::from_millis(1),
            )
            .unwrap();
            assert!(successor.is_primary());
            drop(successor);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn verifying_and_mismatched_ready_records_never_authorize_activation() {
            let expected = identity(10);
            let verifying =
                handle_secondary(expected.clone(), HolderRecord::verifying()).unwrap_err();
            assert_eq!(verifying.code, "single_instance_holder_unreadable");

            let mismatch =
                handle_secondary(expected, HolderRecord::current(identity(11))).unwrap_err();
            assert_eq!(mismatch.code, "single_instance_identity_mismatch");
        }

        #[test]
        fn legacy_pid_only_holder_is_not_treated_as_an_identity_match() {
            let root = test_root("legacy-holder");
            fs::create_dir_all(&root).unwrap();
            let lock_path = root.join("lock");
            let mut file = open_verified_lock_file(&lock_path).unwrap();
            file.write_all(b"1234\n").unwrap();
            assert_eq!(
                read_holder_record(&mut file).unwrap(),
                HolderRead::Malformed
            );
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) use platform::{
    claim, complete, install_activation_listener, InstanceClaim, InstanceLease,
    SingleInstanceActivationRuntime,
};

#[cfg(not(target_os = "macos"))]
pub(crate) struct InstanceLease {
    identity: InstanceIdentity,
}

#[cfg(not(target_os = "macos"))]
impl InstanceLease {
    pub(crate) fn identity(&self) -> &InstanceIdentity {
        &self.identity
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) enum InstanceClaim {
    Primary,
}

#[cfg(not(target_os = "macos"))]
impl InstanceClaim {
    pub(crate) fn is_primary(&self) -> bool {
        true
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn claim() -> Result<InstanceClaim, SingleInstanceFailure> {
    Ok(InstanceClaim::Primary)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn complete(
    _claim: InstanceClaim,
    identity: InstanceIdentity,
) -> Result<Option<InstanceLease>, SingleInstanceFailure> {
    Ok(Some(InstanceLease { identity }))
}
