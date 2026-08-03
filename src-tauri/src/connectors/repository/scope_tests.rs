use super::*;
use crate::connectors::SetConnectorProjectScopeRequest;
use crate::projects::{CreateProjectRequest, ProjectDataPolicy};

fn project(engine: &PersistenceEngine, name: &str) -> String {
    crate::projects::repository::create(
        engine,
        CreateProjectRequest {
            name: name.to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap()
    .project_id
}

fn ready_account(engine: &PersistenceEngine) -> String {
    let connector_id = create_account(engine, "google_workspace", 1).unwrap();
    engine.open_connection().unwrap().execute(
        "UPDATE connector_accounts SET account_label='person@example.com',connection_state='authorized' WHERE connector_id=?1",
        params![connector_id],
    ).unwrap();
    connector_id
}

fn scope(
    connector_id: &str,
    all_projects_enabled: bool,
    enabled_project_ids: Vec<String>,
) -> SetConnectorProjectScopeRequest {
    SetConnectorProjectScopeRequest {
        connector_id: connector_id.to_string(),
        all_projects_enabled,
        enabled_project_ids,
    }
}

#[test]
fn all_projects_covers_future_projects_and_narrowing_restores_the_saved_subset() {
    let root = std::env::temp_dir().join(format!("oomu-connector-scope-{}", unix_time_ms_i64()));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let connector_id = ready_account(&engine);
    let selected = project(&engine, "Selected");
    let unselected = project(&engine, "Unselected");

    let saved =
        set_project_scope(&engine, scope(&connector_id, true, vec![selected.clone()])).unwrap();
    assert!(saved.all_projects_enabled);
    assert!(require_project_enabled(&engine, &connector_id, &unselected).is_ok());
    let future = project(&engine, "Created later");
    assert!(require_project_enabled(&engine, &connector_id, &future).is_ok());

    let narrowed =
        set_project_scope(&engine, scope(&connector_id, false, vec![selected.clone()])).unwrap();
    assert!(!narrowed.all_projects_enabled);
    assert_eq!(narrowed.enabled_project_ids, vec![selected.clone()]);
    assert!(require_project_enabled(&engine, &connector_id, &selected).is_ok());
    assert_eq!(
        require_project_enabled(&engine, &connector_id, &future).unwrap_err(),
        "connector_project_authorization_required"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn scope_save_is_atomic_validated_and_never_authorizes_the_internal_project() {
    let root = std::env::temp_dir().join(format!(
        "oomu-connector-scope-validation-{}",
        unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let connector_id = ready_account(&engine);
    let selected = project(&engine, "Selected");
    set_project_scope(&engine, scope(&connector_id, false, vec![selected.clone()])).unwrap();

    assert_eq!(
        set_project_scope(
            &engine,
            scope(
                &connector_id,
                true,
                vec![selected.clone(), selected.clone()]
            )
        )
        .unwrap_err(),
        "connector_project_scope_duplicate_project"
    );
    assert!(set_project_scope(
        &engine,
        scope(
            &connector_id,
            true,
            vec![crate::projects::repository::INTERNAL_LOCAL_FILES_PROJECT_ID.to_string()]
        ),
    )
    .is_err());
    let account = list_accounts(&engine)
        .unwrap()
        .into_iter()
        .find(|item| item.connector_id == connector_id)
        .unwrap();
    assert!(!account.all_projects_enabled);
    assert_eq!(account.enabled_project_ids, vec![selected]);
    assert!(account.project_scope_reviewed_at_ms.is_some());
    assert!(require_project_enabled(
        &engine,
        &connector_id,
        crate::projects::repository::INTERNAL_LOCAL_FILES_PROJECT_ID,
    )
    .is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn non_ready_accounts_cannot_gain_project_authority() {
    let root =
        std::env::temp_dir().join(format!("oomu-connector-scope-state-{}", unix_time_ms_i64()));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let connector_id = ready_account(&engine);
    let project_id = project(&engine, "Denied");
    for state in [
        "degraded",
        "expired",
        "unsupported",
        "blocked",
        "disconnected",
    ] {
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE connector_accounts SET connection_state=?2 WHERE connector_id=?1",
                params![connector_id, state],
            )
            .unwrap();
        assert_eq!(
            set_project_scope(
                &engine,
                scope(&connector_id, true, vec![project_id.clone()])
            )
            .unwrap_err(),
            if state == "disconnected" {
                "connector_account_not_found"
            } else {
                "connector_project_scope_reconnect_required"
            }
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn account_listing_reconciles_identity_duplicates_without_merging_labels() {
    let root =
        std::env::temp_dir().join(format!("oomu-connector-reconcile-{}", unix_time_ms_i64()));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let ready = ready_account(&engine);
    let duplicate = create_account(&engine, "google_workspace", 1).unwrap();
    let same_label = create_account(&engine, "google_workspace", 1).unwrap();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE connector_accounts SET account_subject_hash='identity-a' WHERE connector_id=?1",
            params![ready],
        )
        .unwrap();
    connection.execute(
        "UPDATE connector_accounts SET account_label='Other label',account_subject_hash='identity-a',connection_state='degraded' WHERE connector_id=?1",
        params![duplicate],
    ).unwrap();
    connection.execute(
        "UPDATE connector_accounts SET account_label='person@example.com',account_subject_hash='identity-b',connection_state='degraded' WHERE connector_id=?1",
        params![same_label],
    ).unwrap();
    drop(connection);

    let accounts = list_accounts(&engine).unwrap();
    assert!(accounts.iter().any(|account| account.connector_id == ready));
    assert!(!accounts
        .iter()
        .any(|account| account.connector_id == duplicate));
    assert!(accounts
        .iter()
        .any(|account| account.connector_id == same_label));
    let duplicate_state: String = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT connection_state FROM connector_accounts WHERE connector_id=?1",
            params![duplicate],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(duplicate_state, "disconnected");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn account_listing_keeps_only_configured_shells_with_a_live_pending_attempt() {
    let root = std::env::temp_dir().join(format!(
        "oomu-connector-attempt-reconcile-{}",
        unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let live = create_account(&engine, "google_workspace", 1).unwrap();
    let expired = create_account(&engine, "google_workspace", 1).unwrap();
    let missing = create_account(&engine, "google_workspace", 1).unwrap();
    record_oauth_attempt(
        &engine,
        "oauth_live_shell",
        &live,
        "live-state",
        "http://127.0.0.1:4000/oauth/callback",
        unix_time_ms_i64() + 60_000,
    )
    .unwrap();
    record_oauth_attempt(
        &engine,
        "oauth_expired_shell",
        &expired,
        "expired-state",
        "http://127.0.0.1:4000/oauth/callback",
        unix_time_ms_i64() - 1,
    )
    .unwrap();

    let accounts = list_accounts(&engine).unwrap();
    assert!(accounts.iter().any(|account| account.connector_id == live));
    assert!(!accounts
        .iter()
        .any(|account| account.connector_id == expired));
    assert!(!accounts
        .iter()
        .any(|account| account.connector_id == missing));
    let disconnected_count: i64 = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM connector_accounts WHERE connector_id IN (?1,?2) AND connection_state='disconnected'",
            params![expired, missing],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(disconnected_count, 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn scope_request_rejects_unknown_fields() {
    let error = serde_json::from_value::<SetConnectorProjectScopeRequest>(serde_json::json!({
        "connectorId": "connector_11111111-1111-4111-8111-111111111111",
        "allProjectsEnabled": true,
        "enabledProjectIds": [],
        "optimisticSuccess": true
    }))
    .unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn reconnect_keeps_the_same_row_and_project_scope() {
    let root = std::env::temp_dir().join(format!(
        "oomu-connector-reconnect-scope-{}",
        unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let connector_id = ready_account(&engine);
    let project_id = project(&engine, "Persistent access");
    set_project_scope(
        &engine,
        scope(&connector_id, false, vec![project_id.clone()]),
    )
    .unwrap();
    record_oauth_attempt(
        &engine,
        "oauth_reconnect_scope",
        &connector_id,
        "state-hash",
        "http://127.0.0.1:4000/oauth/callback",
        unix_time_ms_i64() + 60_000,
    )
    .unwrap();
    finish_oauth(
        &engine,
        "oauth_reconnect_scope",
        &connector_id,
        "person@example.com",
        "stable-subject",
        &["openid".to_string()],
        None,
        None,
        None,
    )
    .unwrap();

    let accounts = list_accounts(&engine).unwrap();
    let matching = accounts
        .iter()
        .filter(|account| account.manifest_id == "google_workspace")
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].connector_id, connector_id);
    assert_eq!(matching[0].enabled_project_ids, vec![project_id]);
    assert!(!matching[0].all_projects_enabled);
    let _ = std::fs::remove_dir_all(root);
}
