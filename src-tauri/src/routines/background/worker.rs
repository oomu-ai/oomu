use crate::db::PersistenceEngine;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, MutexGuard,
    },
    thread,
    time::Duration,
};
use tauri::Emitter;

use super::state;

const WORKER_FLAG: &str = "--oomu-background-runtime-worker";
const HEARTBEAT_PREFIX: &str = "OOMU_BACKGROUND_RUNTIME_HEARTBEAT";
const SHUTDOWN_PREFIX: &str = "OOMU_BACKGROUND_RUNTIME_SHUTDOWN";
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(not(test))]
const GRACEFUL_SHUTDOWN_WAIT: Duration = Duration::from_millis(750);
#[cfg(test)]
const GRACEFUL_SHUTDOWN_WAIT: Duration = Duration::from_millis(100);
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug)]
struct OwnedWorker {
    child: Child,
    stdin: Option<ChildStdin>,
    nonce: String,
    generation: String,
    process_id: u32,
}

#[derive(Clone, Default)]
pub struct BackgroundRuntimeSupervisor {
    worker: Arc<Mutex<Option<OwnedWorker>>>,
    transition: Arc<Mutex<()>>,
    stopping: Arc<AtomicBool>,
}

#[derive(Debug, PartialEq, Eq)]
struct WorkerHeartbeat {
    nonce: String,
    generation: String,
    profile_generation: String,
    build_number: i64,
    build_identity: String,
    profile_class: String,
    process_id: i64,
}

struct WorkerExpectation {
    nonce: String,
    generation: String,
    profile_generation: String,
    build_number: i64,
    build_identity: String,
    profile_class: String,
    process_id: u32,
}

impl BackgroundRuntimeSupervisor {
    pub(super) fn lock_transition(&self) -> Result<MutexGuard<'_, ()>, String> {
        self.transition
            .lock()
            .map_err(|_| "background_runtime_transition_lock_failed".to_string())
    }

    pub(super) fn ensure_started(
        &self,
        app: tauri::AppHandle,
        engine: PersistenceEngine,
        generation: &str,
        profile_generation: &str,
        build_number: i64,
        build_identity: &str,
        profile_class: &str,
    ) -> Result<u32, String> {
        self.stopping.store(false, Ordering::Release);
        let mut guard = self
            .worker
            .lock()
            .map_err(|_| "background_runtime_worker_lock_failed".to_string())?;
        if let Some(worker) = guard.as_mut() {
            if worker.generation == generation
                && worker
                    .child
                    .try_wait()
                    .map_err(|_| "background_runtime_worker_check_failed".to_string())?
                    .is_none()
            {
                return Ok(worker.process_id);
            }
            stop_child(worker)?;
            *guard = None;
        }

        let (mut child, nonce) = start_worker_process(
            generation,
            profile_generation,
            build_number,
            build_identity,
            profile_class,
        )?;
        let process_id = child.id();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "background_runtime_worker_pipe_failed".to_string())?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "background_runtime_worker_control_pipe_failed".to_string())?;
        let expectation = WorkerExpectation {
            nonce: nonce.clone(),
            generation: generation.to_string(),
            profile_generation: profile_generation.to_string(),
            build_number,
            build_identity: build_identity.to_string(),
            profile_class: profile_class.to_string(),
            process_id,
        };
        if spawn_monitor(
            stdout,
            expectation,
            engine.clone(),
            app.clone(),
            self.stopping.clone(),
        )
        .is_err()
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err("background_runtime_monitor_start_failed".to_string());
        }
        if spawn_watchdog(
            engine,
            app,
            generation.to_string(),
            self.clone(),
            self.stopping.clone(),
        )
        .is_err()
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err("background_runtime_watchdog_start_failed".to_string());
        }
        *guard = Some(OwnedWorker {
            child,
            stdin: Some(stdin),
            nonce,
            generation: generation.to_string(),
            process_id,
        });
        Ok(process_id)
    }

    pub(super) fn stop(&self) -> Result<Option<u32>, String> {
        self.stopping.store(true, Ordering::Release);
        let mut guard = self
            .worker
            .lock()
            .map_err(|_| "background_runtime_worker_lock_failed".to_string())?;
        let Some(worker) = guard.as_mut() else {
            return Ok(None);
        };
        let process_id = worker.process_id;
        stop_child(worker)?;
        *guard = None;
        Ok(Some(process_id))
    }

    fn stop_generation(&self, generation: &str, intentional: bool) -> Result<Option<u32>, String> {
        let mut guard = self
            .worker
            .lock()
            .map_err(|_| "background_runtime_worker_lock_failed".to_string())?;
        let Some(worker) = guard.as_mut() else {
            return Ok(None);
        };
        if worker.generation != generation {
            return Ok(None);
        }
        if intentional {
            self.stopping.store(true, Ordering::Release);
        }
        let process_id = worker.process_id;
        stop_child(worker)?;
        *guard = None;
        Ok(Some(process_id))
    }

    #[cfg(test)]
    fn running_generation(&self) -> Option<String> {
        self.worker
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|worker| worker.generation.clone()))
    }
}

fn start_worker_process(
    generation: &str,
    profile_generation: &str,
    build_number: i64,
    build_identity: &str,
    profile_class: &str,
) -> Result<(Child, String), String> {
    let nonce = crate::p0_contracts::TaskId::new().to_string();
    let executable = std::env::current_exe()
        .map_err(|_| "background_runtime_executable_unavailable".to_string())?;
    let child = Command::new(executable)
        .arg(WORKER_FLAG)
        .arg(format!("--nonce={nonce}"))
        .arg(format!("--registration-generation={generation}"))
        .arg(format!("--profile-generation={profile_generation}"))
        .arg(format!("--build-number={build_number}"))
        .arg(format!("--build-identity={build_identity}"))
        .arg(format!("--profile-class={profile_class}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "background_runtime_worker_start_failed".to_string())?;
    eprintln!(
        "OOMU_BACKGROUND_RECEIPT {}",
        serde_json::json!({
            "schema":"oomu.background-runtime.v1",
            "kind":"owned_process_started",
            "owner":"background_runtime",
            "pid":child.id(),
            "registrationGeneration":generation.parse::<i64>().ok(),
            "buildNumber":build_number,
            "buildIdentity":build_identity,
            "profileClass":profile_class,
        })
    );
    Ok((child, nonce))
}

fn spawn_monitor(
    stdout: impl std::io::Read + Send + 'static,
    expected: WorkerExpectation,
    engine: PersistenceEngine,
    app: tauri::AppHandle,
    stopping: Arc<AtomicBool>,
) -> std::io::Result<()> {
    thread::Builder::new()
        .name("oomu-background-runtime-monitor".to_string())
        .spawn(move || monitor_worker(stdout, expected, engine, app, stopping))
        .map(|_| ())
}

fn monitor_worker(
    stdout: impl std::io::Read,
    expected: WorkerExpectation,
    engine: PersistenceEngine,
    app: tauri::AppHandle,
    stopping: Arc<AtomicBool>,
) {
    let reader = BufReader::new(stdout);
    let mut verified_any = false;
    for encoded in reader.lines().map_while(Result::ok) {
        let Some(heartbeat) = parse_heartbeat(&encoded) else {
            continue;
        };
        if !heartbeat_matches(&heartbeat, &expected) {
            continue;
        }
        verified_any = true;
        if state::record_heartbeat(
            &engine,
            &heartbeat.generation,
            &heartbeat.profile_generation,
            heartbeat.build_number,
            &heartbeat.build_identity,
            &heartbeat.profile_class,
            heartbeat.process_id,
        )
        .is_ok()
        {
            publish_status(&app, &engine);
        }
    }
    let intentional =
        stopping.load(Ordering::Acquire) || !state::requested(&engine).unwrap_or(false);
    if verified_any || !intentional {
        let _ = state::record_worker_stopped(&engine, &expected.generation, intentional);
        publish_status(&app, &engine);
    }
}

fn heartbeat_matches(heartbeat: &WorkerHeartbeat, expected: &WorkerExpectation) -> bool {
    heartbeat.nonce == expected.nonce
        && heartbeat.generation == expected.generation
        && heartbeat.profile_generation == expected.profile_generation
        && heartbeat.build_number == expected.build_number
        && heartbeat.build_identity == expected.build_identity
        && heartbeat.profile_class == expected.profile_class
        && heartbeat.process_id == i64::from(expected.process_id)
}

fn spawn_watchdog(
    engine: PersistenceEngine,
    app: tauri::AppHandle,
    generation: String,
    supervisor: BackgroundRuntimeSupervisor,
    stopping: Arc<AtomicBool>,
) -> std::io::Result<()> {
    thread::Builder::new()
        .name("oomu-background-runtime-watchdog".to_string())
        .spawn(move || watchdog_worker(engine, app, generation, supervisor, stopping))
        .map(|_| ())
}

fn watchdog_worker(
    engine: PersistenceEngine,
    app: tauri::AppHandle,
    generation: String,
    supervisor: BackgroundRuntimeSupervisor,
    stopping: Arc<AtomicBool>,
) {
    loop {
        thread::sleep(WATCHDOG_INTERVAL);
        if stopping.load(Ordering::Acquire) {
            break;
        }
        match state::expire_stale_heartbeat(&engine, &generation) {
            Ok(state::WatchdogObservation::Continue) => {}
            Ok(state::WatchdogObservation::Stop) => break,
            Ok(state::WatchdogObservation::Expired) => {
                let _ = supervisor.stop_generation(&generation, false);
                publish_status(&app, &engine);
                break;
            }
            Err(error) => {
                eprintln!(
                    "OOMU_BACKGROUND_WATCHDOG_FAILED {}",
                    crate::redaction::redacted_log_text(&error)
                );
                let _ = state::record_watchdog_failure(&engine, &generation);
                let _ = supervisor.stop_generation(&generation, false);
                publish_status(&app, &engine);
                break;
            }
        }
    }
}

fn stop_child(worker: &mut OwnedWorker) -> Result<(), String> {
    if child_has_exited(&mut worker.child)? {
        worker.stdin.take();
        return Ok(());
    }
    let graceful_request_sent = worker.stdin.take().is_some_and(|mut input| {
        writeln!(
            input,
            "{SHUTDOWN_PREFIX}\t{}\t{}",
            worker.nonce, worker.generation
        )
        .and_then(|_| input.flush())
        .is_ok()
    });
    if graceful_request_sent && wait_for_child_exit(&mut worker.child, GRACEFUL_SHUTDOWN_WAIT)? {
        return Ok(());
    }
    worker
        .child
        .kill()
        .map_err(|_| "background_runtime_worker_force_stop_failed".to_string())?;
    if !wait_for_child_exit(&mut worker.child, GRACEFUL_SHUTDOWN_WAIT)? {
        return Err("background_runtime_worker_exit_unverified".to_string());
    }
    eprintln!(
        "OOMU_BACKGROUND_RECEIPT {}",
        serde_json::json!({
            "schema":"oomu.background-runtime.v1",
            "kind":"forced_process_stop",
            "owner":"background_runtime",
            "pid":worker.process_id,
            "registrationGeneration":worker.generation.parse::<i64>().ok(),
        })
    );
    Ok(())
}

fn child_has_exited(child: &mut Child) -> Result<bool, String> {
    child
        .try_wait()
        .map(|status| status.is_some())
        .map_err(|_| "background_runtime_worker_exit_check_failed".to_string())
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> Result<bool, String> {
    let started = std::time::Instant::now();
    loop {
        if child_has_exited(child)? {
            return Ok(true);
        }
        if started.elapsed() >= timeout {
            return Ok(false);
        }
        thread::sleep(EXIT_POLL_INTERVAL);
    }
}

fn publish_status(app: &tauri::AppHandle, engine: &PersistenceEngine) {
    let Ok(menu_visible) = super::menu_should_be_visible(engine) else {
        return;
    };
    let status_before_menu_sync = super::status(engine).ok();
    if !menu_visible
        && status_before_menu_sync
            .as_ref()
            .is_some_and(|value| value.user_enabled && value.state == "needs_attention")
    {
        if let Err(error) = crate::background_runtime_lifecycle::reveal_recovery_without_focus(app)
        {
            eprintln!(
                "OOMU_BACKGROUND_RECOVERY_WINDOW_FAILED {}",
                crate::redaction::redacted_log_text(&error)
            );
        }
    }
    if let Err(error) = crate::sync_background_tray(app, engine, menu_visible) {
        eprintln!(
            "OOMU_BACKGROUND_TRAY_SYNC_FAILED {}",
            crate::redaction::redacted_log_text(&error)
        );
        super::record_runtime_attention(engine, "background_menu_evidence_failed");
    }
    let Some(status) = super::status(engine).ok() else {
        return;
    };
    let _ = app.emit(super::BACKGROUND_RUNTIME_STATUS_EVENT, status);
}

pub(crate) fn run_worker_if_requested() -> bool {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if !arguments.iter().any(|value| value == WORKER_FLAG) {
        return false;
    }
    let process_identity = match crate::launch_startup::validate_scenario_profile() {
        Ok(identity) => identity,
        Err(error) => {
            eprintln!(
                "OOMU_BACKGROUND_WORKER_STARTUP_FAILED code={} message={}",
                crate::redaction::redacted_log_text(error.code()),
                crate::redaction::redacted_log_text(&error.message())
            );
            return true;
        }
    };
    let Some(nonce) = argument_value(&arguments, "--nonce=") else {
        return true;
    };
    let Some(generation) = argument_value(&arguments, "--registration-generation=") else {
        return true;
    };
    let Some(profile_generation) = argument_value(&arguments, "--profile-generation=") else {
        return true;
    };
    let Some(build_number) =
        argument_value(&arguments, "--build-number=").and_then(|value| value.parse::<i64>().ok())
    else {
        return true;
    };
    let Some(expected_build_identity) = argument_value(&arguments, "--build-identity=") else {
        return true;
    };
    let Some(expected_profile_class) = argument_value(&arguments, "--profile-class=") else {
        return true;
    };
    let identity = state::current_identity_from_process(&process_identity);
    let arguments_valid = valid_token(nonce)
        && valid_token(generation)
        && valid_token(profile_generation)
        && valid_token(expected_build_identity)
        && valid_token(expected_profile_class);
    if !arguments_valid {
        eprintln!("OOMU_BACKGROUND_WORKER_IDENTITY_REJECTED argument_contract=false");
        return true;
    }
    let build_number_matches = identity.build_number == build_number;
    let build_identity_matches = identity.build_identity == expected_build_identity;
    let profile_class_matches = identity.profile_class == expected_profile_class;
    if !build_number_matches || !build_identity_matches || !profile_class_matches {
        eprintln!(
            "OOMU_BACKGROUND_WORKER_IDENTITY_REJECTED build_number_matches={build_number_matches} build_identity_matches={build_identity_matches} profile_class_matches={profile_class_matches}"
        );
        return true;
    }
    let process_id = std::process::id();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    let (shutdown_sender, shutdown_receiver) = mpsc::channel();
    thread::spawn(move || {
        let input = std::io::stdin();
        for line in input.lock().lines().map_while(Result::ok) {
            if shutdown_sender.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        if writeln!(
            output,
            "{HEARTBEAT_PREFIX}\t{nonce}\t{generation}\t{profile_generation}\t{build_number}\t{}\t{}\t{process_id}",
            identity.build_identity,
            identity.profile_class,
        )
        .and_then(|_| output.flush())
        .is_err()
        {
            break;
        }
        match shutdown_receiver.recv_timeout(HEARTBEAT_INTERVAL) {
            Ok(line) if shutdown_request_matches(&line, nonce, generation) => break,
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    true
}

fn shutdown_request_matches(line: &str, nonce: &str, generation: &str) -> bool {
    let mut fields = line.trim_end().split('\t');
    fields.next() == Some(SHUTDOWN_PREFIX)
        && fields.next() == Some(nonce)
        && fields.next() == Some(generation)
        && fields.next().is_none()
}

fn argument_value<'a>(arguments: &'a [String], prefix: &str) -> Option<&'a str> {
    arguments
        .iter()
        .find_map(|value| value.strip_prefix(prefix))
        .filter(|value| !value.is_empty())
}

fn valid_token(value: &str) -> bool {
    value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn parse_heartbeat(encoded: &str) -> Option<WorkerHeartbeat> {
    let mut parts = encoded.split('\t');
    if parts.next()? != HEARTBEAT_PREFIX {
        return None;
    }
    let heartbeat = WorkerHeartbeat {
        nonce: parts.next()?.to_string(),
        generation: parts.next()?.to_string(),
        profile_generation: parts.next()?.to_string(),
        build_number: parts.next()?.parse().ok()?,
        build_identity: parts.next()?.to_string(),
        profile_class: parts.next()?.to_string(),
        process_id: parts.next()?.parse().ok()?,
    };
    if parts.next().is_some()
        || !valid_token(&heartbeat.nonce)
        || !valid_token(&heartbeat.generation)
        || !valid_token(&heartbeat.profile_generation)
        || !valid_token(&heartbeat.build_identity)
        || !valid_token(&heartbeat.profile_class)
        || heartbeat.build_number < 0
        || heartbeat.process_id <= 0
    {
        return None;
    }
    Some(heartbeat)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned_test_worker(mut child: Child) -> OwnedWorker {
        let process_id = child.id();
        let stdin = child.stdin.take().expect("test child control pipe");
        OwnedWorker {
            child,
            stdin: Some(stdin),
            nonce: "nonce-302".to_string(),
            generation: "generation-302".to_string(),
            process_id,
        }
    }

    #[test]
    fn worker_heartbeat_requires_exact_bounded_identity_fields() {
        let encoded = format!(
            "{HEARTBEAT_PREFIX}\tnonce-1\tregistration-1\tprofile-1\t42\tbuild-42\tqualification\t731"
        );
        assert_eq!(
            parse_heartbeat(&encoded),
            Some(WorkerHeartbeat {
                nonce: "nonce-1".to_string(),
                generation: "registration-1".to_string(),
                profile_generation: "profile-1".to_string(),
                build_number: 42,
                build_identity: "build-42".to_string(),
                profile_class: "qualification".to_string(),
                process_id: 731,
            })
        );
        assert!(parse_heartbeat("garbage").is_none());
        assert!(parse_heartbeat(&format!(
            "{HEARTBEAT_PREFIX}\tbad token\tregistration-1\tprofile-1\t42\tbuild-42\tqualification\t731"
        ))
        .is_none());
    }

    #[test]
    fn supervisor_starts_without_claiming_a_worker() {
        let supervisor = BackgroundRuntimeSupervisor::default();
        assert_eq!(supervisor.running_generation(), None);
    }

    #[test]
    fn shutdown_control_requires_the_exact_worker_identity() {
        let encoded = format!("{SHUTDOWN_PREFIX}\tnonce-302\tgeneration-302");
        assert!(shutdown_request_matches(
            &encoded,
            "nonce-302",
            "generation-302"
        ));
        assert!(!shutdown_request_matches(
            &encoded,
            "different-nonce",
            "generation-302"
        ));
        assert!(!shutdown_request_matches(
            &format!("{encoded}\textra"),
            "nonce-302",
            "generation-302"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn worker_shutdown_prefers_the_cooperative_control_pipe() {
        let child = Command::new("/bin/sh")
            .args(["-c", "IFS= read -r request"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cooperative child starts");
        let mut worker = owned_test_worker(child);

        stop_child(&mut worker).expect("cooperative child exit is verified");
        assert!(child_has_exited(&mut worker.child).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn unresponsive_worker_uses_a_bounded_verified_force_fallback() {
        let child = Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("unresponsive child starts");
        let mut worker = owned_test_worker(child);
        let started = std::time::Instant::now();

        stop_child(&mut worker).expect("forced child exit is verified");

        assert!(child_has_exited(&mut worker.child).unwrap());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
