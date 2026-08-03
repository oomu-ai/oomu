use super::*;

fn row() -> BackgroundRow {
    BackgroundRow {
        requested_enabled: true,
        runtime_state: "turning_on".to_string(),
        registration_state: "registered".to_string(),
        registration_backend: "supervised_process".to_string(),
        registration_generation: Some("registration-1".to_string()),
        process_state: "running".to_string(),
        process_id: Some(42),
        build_number: 7,
        build_identity: "build-7".to_string(),
        profile_class: "development".to_string(),
        profile_generation: "profile-1".to_string(),
        heartbeat_at_ms: Some(995),
        heartbeat_expires_at_ms: Some(1_100),
        menu_visible: false,
        last_error_code: None,
        updated_at_ms: 990,
    }
}

#[test]
fn background_on_requires_current_verified_heartbeat() {
    let identity = RuntimeIdentity {
        build_number: 7,
        build_identity: "build-7".to_string(),
        profile_class: "development".to_string(),
    };
    let mut verified = row();
    verified.menu_visible = true;
    assert_eq!(
        derived_runtime_state(&verified, &identity, 1_000),
        "on_verified"
    );

    let mut stale = row();
    stale.heartbeat_expires_at_ms = Some(999);
    stale.runtime_state = "on_verified".to_string();
    assert_eq!(
        derived_runtime_state(&stale, &identity, 1_000),
        "needs_attention"
    );

    let mut wrong_build = row();
    wrong_build.build_number = 6;
    wrong_build.runtime_state = "on_verified".to_string();
    assert_eq!(
        derived_runtime_state(&wrong_build, &identity, 1_000),
        "needs_attention"
    );

    let mut wrong_identity = row();
    wrong_identity.build_identity = "another-build".to_string();
    wrong_identity.runtime_state = "on_verified".to_string();
    assert_eq!(
        derived_runtime_state(&wrong_identity, &identity, 1_000),
        "needs_attention"
    );

    let mut wrong_profile = row();
    wrong_profile.profile_class = "production".to_string();
    wrong_profile.runtime_state = "on_verified".to_string();
    assert_eq!(
        derived_runtime_state(&wrong_profile, &identity, 1_000),
        "needs_attention"
    );
}

#[test]
fn close_to_background_stays_blocked_until_the_menu_is_verified() {
    let identity = RuntimeIdentity {
        build_number: 7,
        build_identity: "build-7".to_string(),
        profile_class: "development".to_string(),
    };
    let mut pre_menu = row();
    pre_menu.runtime_state = "on_verified".to_string();

    assert_eq!(
        derived_runtime_state(&pre_menu, &identity, 1_000),
        "turning_on"
    );

    pre_menu.menu_visible = true;
    assert_eq!(
        derived_runtime_state(&pre_menu, &identity, 1_000),
        "on_verified"
    );
}

#[test]
fn attention_is_terminal_for_the_expired_registration_generation() {
    let mut expired = row();
    expired.runtime_state = "needs_attention".to_string();
    expired.last_error_code = Some("background_runtime_heartbeat_expired".to_string());
    assert!(!heartbeat_matches_active_generation(
        &expired,
        "registration-1",
        "profile-1",
        7,
        "build-7",
        "development",
    ));
}

#[test]
fn requested_preference_is_not_erased_by_runtime_attention() {
    let identity = RuntimeIdentity {
        build_number: 7,
        build_identity: "build-7".to_string(),
        profile_class: "development".to_string(),
    };
    let mut missing_worker = row();
    missing_worker.runtime_state = "on_verified".to_string();
    missing_worker.process_state = "absent".to_string();
    missing_worker.process_id = None;
    assert_eq!(
        derived_runtime_state(&missing_worker, &identity, 1_000),
        "needs_attention"
    );
    assert!(missing_worker.requested_enabled);
}

#[test]
fn enabled_background_service_reconciles_after_update() {
    let identity = RuntimeIdentity {
        build_number: 8,
        build_identity: "build-8".to_string(),
        profile_class: "development".to_string(),
    };
    let mut previous_build = row();
    previous_build.runtime_state = "on_verified".to_string();
    assert_eq!(
        derived_runtime_state(&previous_build, &identity, 1_000),
        "needs_attention"
    );

    previous_build.build_number = identity.build_number;
    previous_build.runtime_state = "turning_on".to_string();
    previous_build.registration_generation = Some("2".to_string());
    previous_build.menu_visible = true;
    assert_eq!(
        derived_runtime_state(&previous_build, &identity, 1_000),
        "turning_on"
    );

    previous_build.build_identity = identity.build_identity.clone();
    assert_eq!(
        derived_runtime_state(&previous_build, &identity, 1_000),
        "on_verified"
    );
}

#[test]
fn verified_menu_finishes_only_the_pending_current_reconciliation() {
    let path = std::env::temp_dir().join(format!(
        "oomu-background-reconciliation-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine =
        crate::db::PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    let turning_on =
        begin_transition(&engine, true, "reconciliation_started").expect("reconciliation begins");
    let registered = observe_registration(&engine, RegistrationObservation::Registered, None)
        .expect("registration verifies");
    let generation = turning_on
        .registration_generation
        .as_deref()
        .expect("registration generation");
    record_heartbeat(
        &engine,
        generation,
        &registered.profile_generation,
        registered.build_number,
        &registered.build_identity,
        &registered.profile_class,
        302,
    )
    .expect("heartbeat verifies");
    assert_eq!(status(&engine).expect("status loads").state, "turning_on");

    record_menu_visibility(&engine, true).expect("menu verifies");
    let verified = status(&engine).expect("verified status loads");
    assert_eq!(verified.state, "on_verified");
    assert!(verified.recent_receipts.iter().any(|receipt| {
        receipt.kind == "reconciliation_verified"
            && receipt.outcome == "verified"
            && receipt.runtime_state == "on_verified"
    }));

    record_menu_visibility(&engine, false).expect("menu hides");
    record_menu_visibility(&engine, true).expect("menu reappears");
    let final_status = status(&engine).expect("final status loads");
    assert_eq!(
        final_status
            .recent_receipts
            .iter()
            .filter(|receipt| receipt.kind == "reconciliation_verified")
            .count(),
        1
    );

    drop(engine);
    let _ = std::fs::remove_file(path);
}

#[test]
fn sprint_304_off_requires_registration_worker_and_menu_to_be_absent() {
    let identity = RuntimeIdentity {
        build_number: 7,
        build_identity: "build-7".to_string(),
        profile_class: "development".to_string(),
    };
    let mut disabling = row();
    disabling.requested_enabled = false;
    disabling.runtime_state = "turning_off".to_string();
    disabling.registration_state = "unregistered".to_string();
    disabling.process_state = "absent".to_string();
    disabling.process_id = None;
    disabling.heartbeat_at_ms = None;
    disabling.heartbeat_expires_at_ms = None;
    disabling.menu_visible = true;

    assert_eq!(
        derived_runtime_state(&disabling, &identity, 10_000),
        "needs_attention"
    );
    disabling.menu_visible = false;
    assert_eq!(derived_runtime_state(&disabling, &identity, 10_000), "off");
}

#[test]
fn sprint_304_matching_menu_evidence_still_rederives_runtime_state() {
    let path = std::env::temp_dir().join(format!(
        "oomu-sprint-304-background-menu-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine =
        crate::db::PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    let turning_on =
        begin_transition(&engine, true, "requested_state_changed").expect("transition starts");
    let registered = observe_registration(&engine, RegistrationObservation::Registered, None)
        .expect("registration verifies");
    record_heartbeat(
        &engine,
        turning_on.registration_generation.as_deref().unwrap(),
        &registered.profile_generation,
        registered.build_number,
        &registered.build_identity,
        &registered.profile_class,
        304,
    )
    .expect("heartbeat verifies");
    record_menu_visibility(&engine, true).expect("menu verifies");
    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE background_service_state SET runtime_state='turning_on' WHERE singleton=1",
            [],
        )
        .unwrap();

    record_menu_visibility(&engine, true).expect("matching native menu evidence is reconciled");
    assert_eq!(status(&engine).unwrap().state, "on_verified");

    drop(engine);
    let _ = std::fs::remove_file(path);
}
