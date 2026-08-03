use super::{
    contracts::{
        DesktopActionKind, DesktopSemanticAction, ElementGeometry, ExpectedOutcomeKind,
        ObservedPostcondition, QualifiedAppleEvent, StartAppControlSession,
    },
    driver::{ResolvedDriverTarget, UnavailableDesktopDriver},
    error::AppControlErrorCode,
    manager::{AppControlManager, SystemAppControlTimeSource},
    policy::{validate_expected_outcome, validate_typed_adapter, DenyAllDesktopAuthority},
    references::{ReferenceContext, ReferenceVault},
};
use std::sync::Arc;

#[test]
fn picker_grants_are_exact_task_bound_and_single_use() {
    let manager = AppControlManager::new(
        Arc::new(UnavailableDesktopDriver),
        Arc::new(DenyAllDesktopAuthority),
        Arc::new(SystemAppControlTimeSource),
    );
    let project_id = crate::p0_contracts::ProjectId::new().to_string();
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    let other_task = crate::p0_contracts::TaskRunId::new().to_string();
    let root = std::env::temp_dir().join(format!(
        "oomu-app-file-grant-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let selected = root.join("selected.txt");
    let sibling = root.join("sibling.txt");
    std::fs::write(&selected, b"selected").unwrap();
    std::fs::write(&sibling, b"sibling").unwrap();
    let grant = manager
        .grant_selected_file(&project_id, &task_run_id, selected.clone())
        .unwrap();
    let wrong_task = manager
        .start_session(StartAppControlSession {
            project_id: project_id.clone(),
            task_run_id: other_task,
            approved_bundle_ids: vec!["com.apple.Preview".to_string()],
            scoped_file_roots: Vec::new(),
            file_grant_ids: vec![grant.grant_id.clone()],
        })
        .unwrap_err();
    assert_eq!(wrong_task.code, AppControlErrorCode::FileScopeViolation);

    let session = manager
        .start_session(StartAppControlSession {
            project_id: project_id.clone(),
            task_run_id: task_run_id.clone(),
            approved_bundle_ids: vec!["com.apple.Preview".to_string()],
            scoped_file_roots: Vec::new(),
            file_grant_ids: vec![grant.grant_id.clone()],
        })
        .unwrap();
    let state = manager.lock().unwrap();
    let roots = &state.sessions[&session.session_id].file_roots;
    assert_eq!(
        roots.canonical_granted_file(&grant.grant_id).unwrap(),
        std::fs::canonicalize(selected).unwrap()
    );
    assert_eq!(
        roots.canonical_file(&sibling).unwrap_err().code,
        AppControlErrorCode::FileScopeViolation
    );
    drop(state);
    let reused = manager
        .start_session(StartAppControlSession {
            project_id,
            task_run_id,
            approved_bundle_ids: vec!["com.apple.Preview".to_string()],
            scoped_file_roots: Vec::new(),
            file_grant_ids: vec![grant.grant_id],
        })
        .unwrap_err();
    assert_eq!(reused.code, AppControlErrorCode::FileScopeViolation);
}

#[test]
fn drag_references_preserve_observed_geometry_and_reject_missing_bounds() {
    let context = ReferenceContext {
        session_id: "session",
        project_id: "project",
        task_run_id: "task",
        bundle_id: "com.apple.finder",
        process_id: 42,
        window_id: "window",
        revision: 1,
        generation: 1,
        now_ms: 10,
    };
    let geometry = ElementGeometry {
        x: 10.0,
        y: 20.0,
        width: 30.0,
        height: 40.0,
    };
    let mut vault = ReferenceVault::default();
    let issued = vault.issue(
        &context,
        super::driver::DriverElement {
            element_key: "source".to_string(),
            role: "AXRow".to_string(),
            label: None,
            value_digest: None,
            secure: false,
            visible: true,
            enabled: true,
            in_modal: false,
            supported_actions: vec![DesktopActionKind::DragDrop],
            geometry: Some(geometry),
        },
        100,
    );
    assert_eq!(
        vault
            .resolve(
                &issued.reference,
                DesktopActionKind::DragDrop,
                false,
                &context
            )
            .unwrap()
            .geometry,
        Some(geometry)
    );
    let missing = vault.issue(
        &context,
        super::driver::DriverElement {
            element_key: "missing".to_string(),
            role: "AXRow".to_string(),
            label: None,
            value_digest: None,
            secure: false,
            visible: true,
            enabled: true,
            in_modal: false,
            supported_actions: vec![DesktopActionKind::DragDrop],
            geometry: None,
        },
        100,
    );
    assert_eq!(
        vault
            .resolve(
                &missing.reference,
                DesktopActionKind::DragDrop,
                false,
                &context
            )
            .unwrap_err()
            .code,
        AppControlErrorCode::AmbiguousTarget
    );
}

#[test]
fn action_matrix_and_expected_results_are_closed() {
    let drag = DesktopSemanticAction::DragDrop {
        source: format!("appref_{}", "0".repeat(48)),
        destination: format!("appref_{}", "1".repeat(48)),
    };
    assert!(validate_typed_adapter("com.apple.finder", &drag).is_ok());
    assert!(validate_typed_adapter("com.apple.mail", &drag).is_err());
    assert!(validate_expected_outcome(&drag, ExpectedOutcomeKind::ApplicationState).is_ok());
    assert!(validate_expected_outcome(&drag, ExpectedOutcomeKind::NoChange).is_err());

    let choose = DesktopSemanticAction::ChooseFile {
        reference: format!("appref_{}", "0".repeat(48)),
        file_grant_id: format!("appfile_{}", "0".repeat(48)),
    };
    assert!(validate_typed_adapter("com.apple.Preview", &choose).is_ok());
    assert!(validate_typed_adapter("com.apple.mail", &choose).is_err());
    assert!(validate_expected_outcome(&choose, ExpectedOutcomeKind::WindowState).is_ok());

    let event = DesktopSemanticAction::AppleEvent {
        command: QualifiedAppleEvent::ActivateApplication,
    };
    assert!(validate_typed_adapter("com.apple.Keynote", &event).is_ok());
    assert!(validate_expected_outcome(&event, ExpectedOutcomeKind::ApplicationState).is_ok());
    assert!(
        serde_json::from_value::<DesktopSemanticAction>(serde_json::json!({
            "kind": "apple_event",
            "command": "arbitrary",
            "eventClass": "evil"
        }))
        .is_err()
    );
}

#[test]
fn qualified_postcondition_evidence_remains_typed() {
    let target = ResolvedDriverTarget {
        element_key: "target".to_string(),
        role: "AXTextField".to_string(),
        secure: false,
        in_modal: true,
        geometry: None,
    };
    assert_eq!(target.in_modal, true);
    assert_eq!(
        ObservedPostcondition::ApplicationState {
            state: "finder_items_changed".to_string()
        }
        .kind(),
        ExpectedOutcomeKind::ApplicationState
    );
}
