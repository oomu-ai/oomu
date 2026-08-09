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
fn watchdog_uses_worker_liveness_without_database_polling() {
    let liveness = WorkerLiveness {
        last_persisted_heartbeat: Mutex::new(Instant::now() - state::HEARTBEAT_VALID_FOR),
        monitor_finished: AtomicBool::new(false),
    };

    assert!(liveness.heartbeat_expired());
    assert!(!liveness.monitor_finished());

    liveness.record_persisted_heartbeat();
    assert!(!liveness.heartbeat_expired());

    liveness.finish_monitor();
    assert!(liveness.monitor_finished());
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
