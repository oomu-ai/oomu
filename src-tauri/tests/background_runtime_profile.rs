#![cfg(target_os = "macos")]

use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const HEARTBEAT_PREFIX: &str = "OOMU_BACKGROUND_RUNTIME_HEARTBEAT";
const WORKER_FLAG: &str = "--oomu-background-runtime-worker";
const APP_DATA_ROOT_ENV: &str = "OOMU_APP_DATA_DIR";
const ISOLATED_PROFILE_ENV: &str = "OOMU_SPRINT_294_ISOLATED_PROFILE";
const RUN_ID_ENV: &str = "OOMU_SPRINT_294_FUNCTIONAL_RUN_ID";
const RESTART_SECRET_ENV: &str = "OOMU_SPRINT_300_RESTART_SECRET";
const SCENARIO_ONE_ENV: &str = "OOMU_SCENARIO_ONE_E2E";
const WAIT_FOR_HEARTBEAT: Duration = Duration::from_secs(20);

#[derive(Debug, PartialEq, Eq)]
struct Heartbeat {
    nonce: String,
    registration_generation: String,
    profile_generation: String,
    build_number: u64,
    build_identity: String,
    profile_class: String,
    process_id: u32,
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct QualificationRoot {
    run_root: PathBuf,
    app_data_root: PathBuf,
    run_id: String,
}

impl QualificationRoot {
    fn create() -> Self {
        let run_id = unique_run_id();
        let run_root = PathBuf::from(format!("/private/tmp/oomu-sprint-294-functional-{run_id}"));
        let app_data_root = run_root.join("app-data");
        fs::create_dir(&run_root).expect("qualification run root must be new");
        fs::set_permissions(&run_root, fs::Permissions::from_mode(0o700))
            .expect("qualification run root must be private");
        fs::create_dir(&app_data_root).expect("qualification app-data root must be new");
        fs::set_permissions(&app_data_root, fs::Permissions::from_mode(0o700))
            .expect("qualification app-data root must be private");
        Self {
            run_root,
            app_data_root,
            run_id,
        }
    }
}

impl Drop for QualificationRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.run_root);
    }
}

#[test]
fn qualification_worker_activates_inherited_profile_before_identity() {
    let executable = Path::new(env!("CARGO_BIN_EXE_oomu"));
    let expected = Heartbeat {
        nonce: "qualification-child-nonce".to_string(),
        registration_generation: "registration-generation-302".to_string(),
        profile_generation: "profile-generation-302".to_string(),
        build_number: env!("OOMU_RELEASE_BUILD_NUMBER")
            .parse()
            .expect("release build number must be numeric"),
        build_identity: executable_sha256(executable),
        profile_class: "qualification".to_string(),
        process_id: 0,
    };
    let profile = QualificationRoot::create();
    let restart_secret = "3a".repeat(32);
    let mut child = Command::new(executable)
        .arg(WORKER_FLAG)
        .arg(format!("--nonce={}", expected.nonce))
        .arg(format!(
            "--registration-generation={}",
            expected.registration_generation
        ))
        .arg(format!(
            "--profile-generation={}",
            expected.profile_generation
        ))
        .arg(format!("--build-number={}", expected.build_number))
        .arg(format!("--build-identity={}", expected.build_identity))
        .arg(format!("--profile-class={}", expected.profile_class))
        .env(ISOLATED_PROFILE_ENV, "1")
        .env(RUN_ID_ENV, &profile.run_id)
        .env(APP_DATA_ROOT_ENV, &profile.app_data_root)
        .env(RESTART_SECRET_ENV, restart_secret)
        .env_remove(SCENARIO_ONE_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("real OOMU background worker must start");
    let child_id = child.id();
    let stdout = child.stdout.take().expect("worker stdout must be piped");
    let mut guard = ChildGuard(child);
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    let line = receiver
        .recv_timeout(WAIT_FOR_HEARTBEAT)
        .expect("worker must emit a heartbeat before the qualification timeout")
        .expect("worker heartbeat must be readable");
    let observed = parse_heartbeat(&line).expect("worker heartbeat must follow the native schema");

    assert_eq!(observed.process_id, child_id);
    assert_eq!(
        observed,
        Heartbeat {
            process_id: child_id,
            ..expected
        }
    );

    let _ = guard.0.kill();
    guard
        .0
        .wait()
        .expect("worker must stop cleanly after proof");
}

fn executable_sha256(path: &Path) -> String {
    let bytes = fs::read(path).expect("OOMU qualification executable must be readable");
    format!("{:x}", Sha256::digest(bytes))
}

fn parse_heartbeat(line: &str) -> Option<Heartbeat> {
    let mut fields = line.trim_end().split('\t');
    if fields.next()? != HEARTBEAT_PREFIX {
        return None;
    }
    let heartbeat = Heartbeat {
        nonce: fields.next()?.to_string(),
        registration_generation: fields.next()?.to_string(),
        profile_generation: fields.next()?.to_string(),
        build_number: fields.next()?.parse().ok()?,
        build_identity: fields.next()?.to_string(),
        profile_class: fields.next()?.to_string(),
        process_id: fields.next()?.parse().ok()?,
    };
    fields.next().is_none().then_some(heartbeat)
}

fn unique_run_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_millis();
    format!(
        "{:013}{:04}",
        millis % 10_000_000_000_000,
        std::process::id() % 10_000
    )
}
