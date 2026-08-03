use super::*;

#[test]
fn autonomic_recycle_policy_allows_displaylink_helpers_only_with_restart() {
    let displaylink = autonomic_recycle_policy_for_process(
        "CrashRestartHelper",
        "/Library/Application Support/DisplayLink/CrashRestartHelper",
    )
    .expect("DisplayLink helper should be allowlisted");

    assert_eq!(displaylink.category, "display_utility");
    assert!(displaylink.restart_available);
    assert_eq!(
        displaylink.restart_strategy,
        Some(DISPLAYLINK_MANAGER_RESTART_LABEL)
    );

    assert!(autonomic_recycle_policy_for_process("WindowServer", "WindowServer").is_none());

    let dev_helper = autonomic_recycle_policy_for_process("node", "node next-server --turbo")
        .expect("Turbopack helper should be observed by policy");
    assert_eq!(dev_helper.category, "development_helper");
    assert!(!dev_helper.restart_available);
}

#[test]
fn autonomic_recycle_validation_rejects_below_threshold_processes() {
    let request = AutonomicRecycleRequest {
        pid: 42,
        process_name: "CrashRestartHelper".to_string(),
        expected_resident_memory_bytes: None,
        dry_run: true,
    };
    let observation = AutonomicProcessObservation {
        pid: 42,
        process_name: "CrashRestartHelper".to_string(),
        command: "CrashRestartHelper".to_string(),
        resident_memory_bytes: AUTONOMIC_RECYCLE_MEMORY_THRESHOLD_BYTES - 1,
    };

    let error = validate_autonomic_recycle_candidate(&request, &observation)
        .expect_err("below-threshold helper should not recycle");

    assert_eq!(error.code, "autonomic_recycle_threshold_not_breached");
}

#[test]
fn autonomic_recycle_validation_accepts_leaking_displaylink_dry_run() {
    let request = AutonomicRecycleRequest {
        pid: 42,
        process_name: "CrashRestartHelper".to_string(),
        expected_resident_memory_bytes: None,
        dry_run: true,
    };
    let observation = AutonomicProcessObservation {
        pid: 42,
        process_name: "CrashRestartHelper".to_string(),
        command: "CrashRestartHelper".to_string(),
        resident_memory_bytes: AUTONOMIC_RECYCLE_MEMORY_THRESHOLD_BYTES,
    };

    let policy = validate_autonomic_recycle_candidate(&request, &observation)
        .expect("threshold breach should validate");

    assert_eq!(policy.canonical_name, "CrashRestartHelper");
    assert_eq!(
        policy.restart.map(autonomic_restart_strategy_label),
        Some(DISPLAYLINK_MANAGER_RESTART_LABEL)
    );
}
