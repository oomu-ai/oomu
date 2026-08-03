use super::{
    contracts::{
        AccessibilityPermission, AppControlControl, AppControlPauseReason, AppControlState,
        ControlAppControlSessionRequest, DesktopActionKind, DesktopApplicationObservation,
        DesktopSemanticAction, DesktopWindowObservation, ExecuteDesktopActionRequest,
        ExpectedOutcomeKind, ObservedPostcondition, StartAppControlSession,
    },
    driver::{
        DesktopDriver, DriverActionRequest, DriverActionResult, DriverElement, DriverObservation,
        DriverObservationRequest,
    },
    error::{AppControlErrorCode, AppControlResult},
    manager::{AppControlManager, AppControlTimeSource},
    policy::{
        AuthorityDecision, AuthorityRequest, DesktopAuthorityEvaluator,
        ReviewedScopeDesktopAuthority, ScopedFileRoots,
    },
    references::{ReferenceContext, ReferenceVault},
    verification::normalize_action,
};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

#[derive(Default)]
struct FixtureDriver {
    observations: Mutex<VecDeque<DriverObservation>>,
    results: Mutex<VecDeque<DriverActionResult>>,
    performed: Mutex<Vec<DesktopActionKind>>,
}

impl FixtureDriver {
    fn push_observation(&self, observation: DriverObservation) {
        self.observations.lock().unwrap().push_back(observation);
    }

    fn push_result(&self, result: DriverActionResult) {
        self.results.lock().unwrap().push_back(result);
    }

    fn performed_count(&self) -> usize {
        self.performed.lock().unwrap().len()
    }
}

impl DesktopDriver for FixtureDriver {
    fn observe(&self, _request: &DriverObservationRequest) -> AppControlResult<DriverObservation> {
        self.observations
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| {
                super::error::AppControlError::new(
                    AppControlErrorCode::DriverUnavailable,
                    "fixture observation unavailable",
                )
            })
    }

    fn perform(&self, request: &DriverActionRequest) -> AppControlResult<DriverActionResult> {
        if request.cancellation.cancelled() {
            return Err(super::error::AppControlError::new(
                AppControlErrorCode::StaleReference,
                "fixture action cancelled",
            ));
        }
        self.performed.lock().unwrap().push(request.action.kind());
        self.results.lock().unwrap().pop_front().ok_or_else(|| {
            super::error::AppControlError::new(
                AppControlErrorCode::DriverUnavailable,
                "fixture result unavailable",
            )
        })
    }
}

#[derive(Default)]
struct AllowAuthority;

impl DesktopAuthorityEvaluator for AllowAuthority {
    fn evaluate(&self, _request: &AuthorityRequest) -> AppControlResult<AuthorityDecision> {
        Ok(AuthorityDecision {
            authorized: true,
            decision_id: "fixture_authority".to_string(),
        })
    }
}

struct FixtureClock(AtomicI64);

impl FixtureClock {
    fn new(now: i64) -> Self {
        Self(AtomicI64::new(now))
    }

    fn advance(&self, millis: i64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}

impl AppControlTimeSource for FixtureClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct Fixture {
    manager: AppControlManager,
    driver: Arc<FixtureDriver>,
    clock: Arc<FixtureClock>,
    project_id: String,
    task_run_id: String,
    session_id: String,
}

fn fixture(bundle_id: &str) -> Fixture {
    let driver = Arc::new(FixtureDriver::default());
    let clock = Arc::new(FixtureClock::new(10_000));
    let manager = AppControlManager::new(driver.clone(), Arc::new(AllowAuthority), clock.clone());
    let project_id = crate::p0_contracts::ProjectId::new().to_string();
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    let session = manager
        .start_session(StartAppControlSession {
            project_id: project_id.clone(),
            task_run_id: task_run_id.clone(),
            approved_bundle_ids: vec![bundle_id.to_string()],
            scoped_file_roots: Vec::new(),
            file_grant_ids: Vec::new(),
        })
        .unwrap();
    Fixture {
        manager,
        driver,
        clock,
        project_id,
        task_run_id,
        session_id: session.session_id,
    }
}

fn observation(
    bundle_id: &str,
    window_id: &str,
    visible: bool,
    secure: bool,
    value_digest: Option<String>,
) -> DriverObservation {
    DriverObservation {
        permission: AccessibilityPermission::Granted,
        application: DesktopApplicationObservation {
            bundle_id: bundle_id.to_string(),
            display_name: "Fixture".to_string(),
            process_id: 42,
        },
        window: DesktopWindowObservation {
            window_id: window_id.to_string(),
            title: "Document".to_string(),
            visible,
            modal: false,
        },
        focused_element_key: Some("element-1".to_string()),
        elements: vec![DriverElement {
            element_key: "element-1".to_string(),
            role: if secure {
                "AXSecureTextField"
            } else {
                "AXTextField"
            }
            .to_string(),
            label: Some("Title".to_string()),
            value_digest: (!secure).then_some(value_digest).flatten(),
            secure,
            visible: true,
            enabled: true,
            in_modal: false,
            supported_actions: vec![
                DesktopActionKind::Focus,
                DesktopActionKind::Press,
                DesktopActionKind::Select,
                DesktopActionKind::TypeText,
                DesktopActionKind::Scroll,
            ],
            geometry: None,
        }],
        screenshot: None,
    }
}

fn text_request(
    fixture: &Fixture,
    revision: u64,
    reference: String,
    text: &str,
) -> ExecuteDesktopActionRequest {
    ExecuteDesktopActionRequest {
        session_id: fixture.session_id.clone(),
        task_run_id: fixture.task_run_id.clone(),
        observation_revision: revision,
        action: DesktopSemanticAction::TypeText {
            reference,
            text: text.to_string(),
        },
        expected_outcome: ExpectedOutcomeKind::ElementValue,
    }
}

fn sha(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

#[test]
fn qualified_action_requires_fresh_reference_and_records_verified_receipt() {
    let fixture = fixture("com.apple.mail");
    fixture.driver.push_observation(observation(
        "com.apple.mail",
        "window-1",
        true,
        false,
        Some(sha("old")),
    ));
    let before = fixture
        .manager
        .observe(&fixture.session_id, &fixture.task_run_id)
        .unwrap();
    fixture.driver.push_result(DriverActionResult {
        receipt_token: "native-receipt".to_string(),
        postcondition: ObservedPostcondition::ElementValue {
            element_key: "element-1".to_string(),
            value_sha256: sha("hello"),
        },
    });
    fixture.driver.push_observation(observation(
        "com.apple.mail",
        "window-1",
        true,
        false,
        Some(sha("hello")),
    ));
    let outcome = fixture
        .manager
        .execute(text_request(
            &fixture,
            before.revision,
            before.elements[0].reference.clone(),
            "hello",
        ))
        .unwrap();
    assert_eq!(outcome.session.state, AppControlState::Running);
    assert_eq!(
        outcome.receipt.status,
        super::contracts::AppControlOutcomeStatus::Verified
    );
    assert_eq!(outcome.receipt.before_observation_hash.len(), 64);
    assert_eq!(outcome.receipt.after_observation_hash.len(), 64);
    assert_eq!(fixture.driver.performed_count(), 1);
}

#[test]
fn fresh_observation_invalidates_old_revision_and_reference() {
    let fixture = fixture("com.apple.mail");
    fixture.driver.push_observation(observation(
        "com.apple.mail",
        "window-1",
        true,
        false,
        Some(sha("a")),
    ));
    fixture.driver.push_observation(observation(
        "com.apple.mail",
        "window-1",
        true,
        false,
        Some(sha("a")),
    ));
    let old = fixture
        .manager
        .observe(&fixture.session_id, &fixture.task_run_id)
        .unwrap();
    fixture
        .manager
        .observe(&fixture.session_id, &fixture.task_run_id)
        .unwrap();
    let error = fixture
        .manager
        .execute(text_request(
            &fixture,
            old.revision,
            old.elements[0].reference.clone(),
            "never",
        ))
        .unwrap_err();
    assert_eq!(error.code, AppControlErrorCode::StaleReference);
    assert_eq!(fixture.driver.performed_count(), 0);
}

#[test]
fn takeover_and_paused_handback_require_reobservation_without_replay() {
    let fixture = fixture("com.apple.mail");
    for _ in 0..3 {
        fixture.driver.push_observation(observation(
            "com.apple.mail",
            "window-1",
            true,
            false,
            Some(sha("a")),
        ));
    }
    let old = fixture
        .manager
        .observe(&fixture.session_id, &fixture.task_run_id)
        .unwrap();
    fixture
        .manager
        .control(ControlAppControlSessionRequest {
            session_id: fixture.session_id.clone(),
            task_run_id: fixture.task_run_id.clone(),
            control: AppControlControl::TakeControl,
        })
        .unwrap();
    let pending = fixture
        .manager
        .control(ControlAppControlSessionRequest {
            session_id: fixture.session_id.clone(),
            task_run_id: fixture.task_run_id.clone(),
            control: AppControlControl::ReturnToOomu,
        })
        .unwrap();
    assert_eq!(pending.state, AppControlState::ReturnPending);
    fixture
        .manager
        .observe(&fixture.session_id, &fixture.task_run_id)
        .unwrap();
    let error = fixture
        .manager
        .execute(text_request(
            &fixture,
            old.revision,
            old.elements[0].reference.clone(),
            "never",
        ))
        .unwrap_err();
    assert_eq!(error.code, AppControlErrorCode::StaleReference);

    fixture
        .manager
        .notify_user_input(&fixture.session_id, &fixture.task_run_id)
        .unwrap();
    fixture
        .manager
        .control(ControlAppControlSessionRequest {
            session_id: fixture.session_id.clone(),
            task_run_id: fixture.task_run_id.clone(),
            control: AppControlControl::ReturnToOomu,
        })
        .unwrap();
    let resumed = fixture
        .manager
        .observe(&fixture.session_id, &fixture.task_run_id)
        .unwrap();
    assert!(resumed.generation > old.generation);
    assert_eq!(fixture.driver.performed_count(), 0);
}

#[test]
fn physical_input_and_monitor_readiness_fail_closed() {
    let driver = Arc::new(FixtureDriver::default());
    let epoch = Arc::new(AtomicU64::new(1));
    let ready = Arc::new(AtomicBool::new(false));
    let manager = AppControlManager::new(
        driver.clone(),
        Arc::new(AllowAuthority),
        Arc::new(FixtureClock::new(20_000)),
    )
    .attach_test_input_monitor(epoch.clone(), ready.clone());
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    let session = manager
        .start_session(StartAppControlSession {
            project_id: crate::p0_contracts::ProjectId::new().to_string(),
            task_run_id: task_run_id.clone(),
            approved_bundle_ids: vec!["com.apple.mail".to_string()],
            scoped_file_roots: Vec::new(),
            file_grant_ids: Vec::new(),
        })
        .unwrap();
    let unavailable = manager.get_status(Some(&task_run_id)).unwrap().unwrap();
    assert_eq!(
        unavailable.pause_reason,
        Some(AppControlPauseReason::DriverUnavailable)
    );
    ready.store(true, Ordering::SeqCst);
    driver.push_observation(observation(
        "com.apple.mail",
        "window-1",
        true,
        false,
        Some(sha("a")),
    ));
    let recovered = manager.observe(&session.session_id, &task_run_id).unwrap();
    assert_eq!(
        manager
            .get_status(Some(&task_run_id))
            .unwrap()
            .unwrap()
            .state,
        AppControlState::Running
    );
    epoch.fetch_add(1, Ordering::SeqCst);
    let paused = manager.get_status(Some(&task_run_id)).unwrap().unwrap();
    assert_eq!(paused.pause_reason, Some(AppControlPauseReason::UserInput));
    let request = ExecuteDesktopActionRequest {
        session_id: session.session_id,
        task_run_id,
        observation_revision: recovered.revision,
        action: DesktopSemanticAction::Focus {
            reference: recovered.elements[0].reference.clone(),
        },
        expected_outcome: ExpectedOutcomeKind::ElementState,
    };
    assert_eq!(
        manager.execute(request).unwrap_err().code,
        AppControlErrorCode::NotRunning
    );
    assert_eq!(driver.performed_count(), 0);
}

#[test]
fn secure_hidden_missing_unknown_and_browser_states_fail_closed() {
    let secure = fixture("com.apple.mail");
    secure
        .driver
        .push_observation(observation("com.apple.mail", "window-1", true, true, None));
    secure
        .manager
        .observe(&secure.session_id, &secure.task_run_id)
        .unwrap();
    assert_eq!(
        secure
            .manager
            .get_status(Some(&secure.task_run_id))
            .unwrap()
            .unwrap()
            .pause_reason,
        Some(AppControlPauseReason::SecureField)
    );

    let hidden = fixture("com.apple.mail");
    hidden.driver.push_observation(observation(
        "com.apple.mail",
        "window-1",
        false,
        false,
        Some(sha("a")),
    ));
    hidden
        .manager
        .observe(&hidden.session_id, &hidden.task_run_id)
        .unwrap();
    assert_eq!(
        hidden
            .manager
            .get_status(Some(&hidden.task_run_id))
            .unwrap()
            .unwrap()
            .pause_reason,
        Some(AppControlPauseReason::HiddenWindow)
    );

    let missing = fixture("com.apple.mail");
    let mut missing_observation =
        observation("com.apple.mail", "window-1", true, false, Some(sha("a")));
    missing_observation.permission = AccessibilityPermission::Missing;
    missing.driver.push_observation(missing_observation);
    assert_eq!(
        missing
            .manager
            .observe(&missing.session_id, &missing.task_run_id)
            .unwrap_err()
            .code,
        AppControlErrorCode::AccessibilityPermissionMissing
    );

    let unknown = fixture("com.example.Unknown");
    unknown.driver.push_observation(observation(
        "com.example.Unknown",
        "window-1",
        true,
        false,
        Some(sha("a")),
    ));
    unknown
        .manager
        .observe(&unknown.session_id, &unknown.task_run_id)
        .unwrap();
    assert_eq!(
        unknown
            .manager
            .get_status(Some(&unknown.task_run_id))
            .unwrap()
            .unwrap()
            .state,
        AppControlState::Observing
    );

    let browser = AppControlManager::new(
        Arc::new(FixtureDriver::default()),
        Arc::new(AllowAuthority),
        Arc::new(FixtureClock::new(1)),
    );
    let error = browser
        .start_session(StartAppControlSession {
            project_id: crate::p0_contracts::ProjectId::new().to_string(),
            task_run_id: crate::p0_contracts::TaskRunId::new().to_string(),
            approved_bundle_ids: vec!["com.apple.Safari".to_string()],
            scoped_file_roots: Vec::new(),
            file_grant_ids: Vec::new(),
        })
        .unwrap_err();
    assert_eq!(error.code, AppControlErrorCode::BrowserRouteRequired);
}

#[test]
fn file_actions_reject_unissued_grants() {
    let root = std::env::temp_dir().join(format!(
        "oomu-app-control-root-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let roots = ScopedFileRoots::new(vec![root]).unwrap();
    let error = normalize_action(
        DesktopSemanticAction::ChooseFile {
            reference: format!("appref_{}", "0".repeat(48)),
            file_grant_id: format!("appfile_{}", "0".repeat(48)),
        },
        &roots,
    )
    .unwrap_err();
    assert_eq!(error.code, AppControlErrorCode::FileScopeViolation);
}

#[test]
fn references_are_task_app_window_revision_and_ttl_bound() {
    let mut vault = ReferenceVault::default();
    let context = ReferenceContext {
        session_id: "session-a",
        project_id: "project-a",
        task_run_id: "task-a",
        bundle_id: "com.apple.mail",
        process_id: 1,
        window_id: "window-a",
        revision: 1,
        generation: 1,
        now_ms: 100,
    };
    let issued = vault.issue(
        &context,
        DriverElement {
            element_key: "key".to_string(),
            role: "AXButton".to_string(),
            label: Some("Send".to_string()),
            value_digest: None,
            secure: false,
            visible: true,
            enabled: true,
            in_modal: false,
            supported_actions: vec![DesktopActionKind::Press],
            geometry: None,
        },
        200,
    );
    let other_app = ReferenceContext {
        bundle_id: "com.apple.iCal",
        ..context.clone()
    };
    assert_eq!(
        vault
            .resolve(
                &issued.reference,
                DesktopActionKind::Press,
                false,
                &other_app
            )
            .unwrap_err()
            .code,
        AppControlErrorCode::CrossApplicationReference
    );
    let expired = ReferenceContext {
        now_ms: 201,
        ..context
    };
    assert_eq!(
        vault
            .resolve(&issued.reference, DesktopActionKind::Press, false, &expired)
            .unwrap_err()
            .code,
        AppControlErrorCode::StaleReference
    );
}

#[test]
fn reviewed_direct_approval_and_task_evidence_are_bound_and_retrievable() {
    crate::tasks::register_runtime_bridge().unwrap();
    let root = std::env::temp_dir().join(format!(
        "oomu-app-control-evidence-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine =
        crate::db::PersistenceEngine::initialize_for_integration_test(root.join("state.sqlite"))
            .unwrap();
    let project = crate::projects::repository::create(
        &engine,
        crate::projects::CreateProjectRequest {
            name: "App control evidence".to_string(),
            description: String::new(),
            data_policy: crate::projects::ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let task_id = crate::p0_contracts::TaskId::new().to_string();
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    let now = crate::foundation::clock::unix_time_ms_i64();
    engine.open_connection().unwrap().execute(
        "INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'taskflow','app-control-test','running','test',?2,'App control',?4,?4,'not_required')",
        rusqlite::params![task_run_id, task_id, project.project_id, now],
    ).unwrap();

    let driver = Arc::new(FixtureDriver::default());
    let manager = AppControlManager::new(
        driver.clone(),
        Arc::new(ReviewedScopeDesktopAuthority::new(engine.clone())),
        Arc::new(FixtureClock::new(now)),
    )
    .attach_test_evidence_engine(engine.clone());
    let session = manager
        .start_session(StartAppControlSession {
            project_id: project.project_id.clone(),
            task_run_id: task_run_id.clone(),
            approved_bundle_ids: vec!["com.apple.mail".to_string()],
            scoped_file_roots: Vec::new(),
            file_grant_ids: Vec::new(),
        })
        .unwrap();
    driver.push_observation(observation(
        "com.apple.mail",
        "window-1",
        true,
        false,
        Some(sha("old")),
    ));
    let before = manager.observe(&session.session_id, &task_run_id).unwrap();
    let request = ExecuteDesktopActionRequest {
        session_id: session.session_id,
        task_run_id: task_run_id.clone(),
        observation_revision: before.revision,
        action: DesktopSemanticAction::TypeText {
            reference: before.elements[0].reference.clone(),
            text: "approved".to_string(),
        },
        expected_outcome: ExpectedOutcomeKind::ElementValue,
    };
    let authority = manager.authority_request_for(&request).unwrap();
    let binding = AppControlManager::approval_binding(&authority);
    assert_eq!(binding.action_class, "app_control");
    assert!(binding.argument_class.starts_with("exact:"));
    manager.register_direct_approval(&authority).unwrap();
    driver.push_result(DriverActionResult {
        receipt_token: "native".to_string(),
        postcondition: ObservedPostcondition::ElementValue {
            element_key: "element-1".to_string(),
            value_sha256: sha("approved"),
        },
    });
    driver.push_observation(observation(
        "com.apple.mail",
        "window-1",
        true,
        false,
        Some(sha("approved")),
    ));
    manager.execute(request).unwrap();

    let connection = engine.open_connection().unwrap();
    let mut statement = connection
        .prepare("SELECT event_json FROM task_events WHERE task_run_id=?1 ORDER BY sequence")
        .unwrap();
    let events = statement
        .query_map(rusqlite::params![task_run_id], |row| {
            row.get::<_, String>(0)
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(events
        .iter()
        .any(|event| event.contains("app_control.session_started")));
    assert!(events
        .iter()
        .any(|event| event.contains("app_control.observation")));
    assert!(events
        .iter()
        .any(|event| event.contains("app_control.action_receipt")));
    assert!(events.iter().all(|event| event.contains(&task_run_id)));

    let wrong_project = manager
        .start_session(StartAppControlSession {
            project_id: crate::p0_contracts::ProjectId::new().to_string(),
            task_run_id,
            approved_bundle_ids: vec!["com.apple.mail".to_string()],
            scoped_file_roots: Vec::new(),
            file_grant_ids: Vec::new(),
        })
        .unwrap_err();
    assert_eq!(wrong_project.code, AppControlErrorCode::TaskBindingMismatch);
}

#[test]
fn terminal_global_status_expires_but_task_history_remains() {
    let fixture = fixture("com.apple.mail");
    fixture
        .manager
        .control(ControlAppControlSessionRequest {
            session_id: fixture.session_id.clone(),
            task_run_id: fixture.task_run_id.clone(),
            control: AppControlControl::Stop,
        })
        .unwrap();
    assert!(fixture.manager.get_status(None).unwrap().is_some());
    fixture.clock.advance(15_001);
    assert!(fixture.manager.get_status(None).unwrap().is_none());
    assert!(fixture
        .manager
        .get_status(Some(&fixture.task_run_id))
        .unwrap()
        .is_some());
    assert!(!fixture.project_id.is_empty());
}
