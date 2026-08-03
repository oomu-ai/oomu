use super::{
    background_registration_error, macos::MAIN_APP_SERVICE_SELECTOR,
    reconcile_disabled_registration, registration_backend_for, registration_failure_observation,
    registration_observation, registration_status_error, scheduled_work_allowed, state,
    state::RegistrationObservation, RegistrationBackend, ScheduledWorkOwner,
};
use std::cell::Cell;

#[test]
fn raw_objective_c_selector_uses_the_sdk_getter_name() {
    assert_eq!(MAIN_APP_SERVICE_SELECTOR.to_bytes(), b"mainAppService");
}

#[test]
fn native_service_states_map_to_typed_registration_states() {
    assert_eq!(
        registration_observation("active"),
        RegistrationObservation::Registered
    );
    assert_eq!(
        registration_observation("paused"),
        RegistrationObservation::Unregistered
    );
    assert_eq!(
        registration_observation("requires_approval"),
        RegistrationObservation::RequiresApproval
    );
}

#[test]
fn service_failures_map_to_truthful_registration_states() {
    assert_eq!(
        registration_failure_observation("background_requires_approval"),
        RegistrationObservation::RequiresApproval
    );
    assert_eq!(
        registration_failure_observation("background_requires_signed_install"),
        RegistrationObservation::Unavailable
    );
    assert_eq!(
        registration_failure_observation("background_registration_rejected"),
        RegistrationObservation::Failed
    );
}

#[test]
fn revoked_native_registration_requires_visible_recovery() {
    assert_eq!(
        registration_status_error(true, RegistrationObservation::Unregistered),
        Some("background_registration_lost")
    );
    assert_eq!(
        registration_status_error(true, RegistrationObservation::Registered),
        None
    );
}

#[test]
fn disabled_reconciliation_mutates_only_when_registration_is_observed() {
    let clean_unregister_calls = Cell::new(0);
    let clean = reconcile_disabled_registration(
        || Ok("paused".to_string()),
        || {
            clean_unregister_calls.set(clean_unregister_calls.get() + 1);
            Ok("paused".to_string())
        },
    );
    assert_eq!(clean, (RegistrationObservation::Unregistered, None));
    assert_eq!(clean_unregister_calls.get(), 0);

    let drift_status_calls = Cell::new(0);
    let drift_unregister_calls = Cell::new(0);
    let drift = reconcile_disabled_registration(
        || {
            let call = drift_status_calls.get();
            drift_status_calls.set(call + 1);
            Ok(if call == 0 { "active" } else { "paused" }.to_string())
        },
        || {
            drift_unregister_calls.set(drift_unregister_calls.get() + 1);
            Ok("paused".to_string())
        },
    );
    assert_eq!(drift, (RegistrationObservation::Unregistered, None));
    assert_eq!(drift_unregister_calls.get(), 1);
}

#[test]
fn foreground_scheduling_survives_detached_runtime_attention() {
    let path = std::env::temp_dir().join(format!(
        "oomu-background-owner-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine =
        crate::db::PersistenceEngine::initialize_at(path.clone()).expect("database initializes");
    state::ensure_state(&engine).expect("background state initializes");
    state::begin_transition(&engine, true, "requested_state_changed")
        .expect("background is requested");
    state::set_attention(&engine, "background_runtime_worker_stopped")
        .expect("detached runtime needs attention");
    let now = crate::foundation::clock::unix_time_ms_i64();
    let connection = engine.open_connection().expect("database opens");
    connection
        .execute(
            "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,workflow_ir_json,is_active,created_at_ms,updated_at_ms,compiled_at_ms) VALUES ('foreground-due',1,'Foreground due','','{}',NULL,1,?1,?1,?1)",
            rusqlite::params![now],
        )
        .expect("workflow inserts");
    connection
        .execute(
            "INSERT INTO workflow_schedules(id,workflow_id,workflow_version,label,schedule_expression,run_request_json,is_active,next_run_at_ms,created_at_ms,updated_at_ms) VALUES ('foreground-due','foreground-due',1,'Foreground due','manual','{}',1,?1,?1,?1)",
            rusqlite::params![now],
        )
        .expect("due schedule inserts");
    drop(connection);

    let foreground_allowed =
        scheduled_work_allowed(&engine, ScheduledWorkOwner::ForegroundApplication);
    let detached_allowed = scheduled_work_allowed(&engine, ScheduledWorkOwner::DetachedRuntime);
    assert!(foreground_allowed);
    assert!(!detached_allowed);
    let detached_claims = if detached_allowed {
        engine
            .claim_due_workflow_schedules(now, 1, 60_000)
            .expect("detached claim")
    } else {
        Vec::new()
    };
    assert!(detached_claims.is_empty());
    let foreground_claims = engine
        .claim_due_workflow_schedules(now, 1, 60_000)
        .expect("foreground claim");
    assert_eq!(foreground_claims.len(), 1);

    let turning_on = state::begin_transition(&engine, true, "registration_started")
        .expect("verified transition starts");
    let registered =
        state::observe_registration(&engine, RegistrationObservation::Registered, None)
            .expect("registration verifies");
    let generation = turning_on
        .registration_generation
        .as_deref()
        .expect("registration generation");
    state::record_heartbeat(
        &engine,
        generation,
        &registered.profile_generation,
        registered.build_number,
        &registered.build_identity,
        &registered.profile_class,
        302,
    )
    .expect("heartbeat verifies");
    state::record_menu_visibility(&engine, true).expect("menu verifies");
    assert!(scheduled_work_allowed(
        &engine,
        ScheduledWorkOwner::DetachedRuntime,
    ));

    drop(engine);
    let _ = std::fs::remove_file(path);
}

#[test]
fn only_bare_validated_nonproduction_profiles_use_process_registration() {
    use crate::runtime_profile::RuntimeProfileClass;

    assert_eq!(
        registration_backend_for(RuntimeProfileClass::Development, false),
        RegistrationBackend::SupervisedProcess
    );
    assert_eq!(
        registration_backend_for(RuntimeProfileClass::Qualification, false),
        RegistrationBackend::SupervisedProcess
    );
    for class in [
        RuntimeProfileClass::Production,
        RuntimeProfileClass::Development,
        RuntimeProfileClass::Qualification,
    ] {
        assert_eq!(
            registration_backend_for(class, true),
            RegistrationBackend::SmAppService
        );
    }
}

#[test]
fn sprint_304_invalid_installed_identity_has_one_actionable_background_error() {
    assert_eq!(
        background_registration_error(crate::runtime_profile::INVALID_PRODUCTION_IDENTITY),
        "background_requires_signed_install"
    );
    assert_eq!(
        background_registration_error("background_registration_rejected"),
        "background_registration_rejected"
    );
}
