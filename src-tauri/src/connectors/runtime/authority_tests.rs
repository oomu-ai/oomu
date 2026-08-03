use super::*;
use crate::projects::{CreateProjectRequest, ProjectDataPolicy};
use rusqlite::params;

#[test]
fn planner_catalog_advertises_only_registered_granted_capabilities() {
    let root = std::env::temp_dir().join(format!(
        "oomu-connector-authority-catalog-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: "Authority catalog".into(),
            description: String::new(),
            data_policy: ProjectDataPolicy::AskBeforeCloud,
        },
    )
    .unwrap();
    let session = engine
        .ensure_chat_session(crate::db::CreateChatSessionRequest {
            agent_id: "agent-authority".into(),
            provider_id: "local_model".into(),
            model_id: "model-authority".into(),
            title: Some("Authority catalog".into()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    crate::projects::repository::bind_record(
        &engine,
        crate::projects::BindProjectRecordRequest {
            project_id: Some(project.project_id.clone()),
            record_kind: "chat_session".into(),
            record_id: session.id.clone(),
        },
    )
    .unwrap();

    let microsoft =
        repository::create_account(&engine, super::super::microsoft365::MANIFEST_ID, 1).unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE connector_accounts SET account_label='Work',connection_state='authorized',granted_scopes_json='[\"Mail.Read\"]' WHERE connector_id=?1",
            params![microsoft],
        )
        .unwrap();
    repository::set_project_binding(&engine, &microsoft, &project.project_id, true).unwrap();

    let mut hidden = Vec::new();
    for manifest in ["apple_apps", "google_workspace", "slack", "mcp_runtime"] {
        let connector = repository::create_account(&engine, manifest, 1).unwrap();
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE connector_accounts SET account_label=?2,connection_state='reachable' WHERE connector_id=?1",
                params![connector, manifest],
            )
            .unwrap();
        repository::set_project_binding(&engine, &connector, &project.project_id, true).unwrap();
        hidden.push(connector);
    }

    let context = planner_tool_context(&engine, &session.id).unwrap().unwrap();
    assert!(context.contains(&format!("connectorRef={microsoft}")));
    assert!(context
        .contains("capabilities=draft_calendar_event,draft_chat_message,find_email,read_email"));
    assert!(!context.contains("draft_email"));
    for connector in hidden {
        assert!(!context.contains(&format!("connectorRef={connector}")));
    }

    let account = repository::account_authority(&engine, &microsoft)
        .unwrap()
        .unwrap();
    let registered = adapter::for_manifest(&account.manifest_id).unwrap();
    for capability in executable_capabilities(&account.capability_grants, registered) {
        let operation = registered.operation_for_capability(capability).unwrap();
        registered.operation_policy(operation).unwrap();
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn pre_sign_authority_rejects_dead_or_ungranted_connector_pairs() {
    let root = std::env::temp_dir().join(format!(
        "oomu-connector-authority-matrix-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: "Authorized project".into(),
            description: String::new(),
            data_policy: ProjectDataPolicy::AskBeforeCloud,
        },
    )
    .unwrap();
    let other_project = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: "Other project".into(),
            description: String::new(),
            data_policy: ProjectDataPolicy::AskBeforeCloud,
        },
    )
    .unwrap();
    let microsoft =
        repository::create_account(&engine, super::super::microsoft365::MANIFEST_ID, 1).unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE connector_accounts SET account_label='Work',connection_state='authorized',granted_scopes_json='[\"Mail.Read\"]' WHERE connector_id=?1",
            params![microsoft],
        )
        .unwrap();
    repository::set_project_binding(&engine, &microsoft, &project.project_id, true).unwrap();

    assert!(engine
        .validate_planned_connector_authority(
            &microsoft,
            Some("microsoft_365"),
            Some("work"),
            Some(&project.project_id),
            "find_email",
        )
        .is_ok());
    assert_eq!(
        engine
            .validate_planned_connector_authority(
                &microsoft,
                Some("microsoft_365"),
                None,
                Some(&project.project_id),
                "draft_email",
            )
            .unwrap_err(),
        "connector_planned_capability_consent_required"
    );
    assert_eq!(
        engine
            .validate_planned_connector_authority(
                &microsoft,
                None,
                None,
                Some(&other_project.project_id),
                "find_email",
            )
            .unwrap_err(),
        "connector_planned_project_authorization_required"
    );
    assert_eq!(
        engine
            .validate_planned_connector_authority(&microsoft, None, None, None, "find_email")
            .unwrap_err(),
        "connector_planned_project_context_required"
    );
    assert_eq!(
        engine
            .validate_planned_connector_authority(
                &microsoft,
                None,
                None,
                Some(&project.project_id),
                "send_chat_message",
            )
            .unwrap_err(),
        "connector_planned_capability_unsupported"
    );

    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE connector_accounts SET connection_state='degraded' WHERE connector_id=?1",
            params![microsoft],
        )
        .unwrap();
    assert_eq!(
        engine
            .validate_planned_connector_authority(
                &microsoft,
                None,
                None,
                Some(&project.project_id),
                "find_email",
            )
            .unwrap_err(),
        "connector_planned_account_reconnect_required"
    );

    for manifest in ["apple_apps", "google_workspace", "slack", "mcp_runtime"] {
        let connector = repository::create_account(&engine, manifest, 1).unwrap();
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE connector_accounts SET account_label=?2,connection_state='reachable' WHERE connector_id=?1",
                params![connector, manifest],
            )
            .unwrap();
        repository::set_project_binding(&engine, &connector, &project.project_id, true).unwrap();
        assert_eq!(
            engine
                .validate_planned_connector_authority(
                    &connector,
                    Some(manifest),
                    None,
                    Some(&project.project_id),
                    "find_email",
                )
                .unwrap_err(),
            "connector_planned_adapter_unavailable"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn direct_and_planned_gates_share_all_projects_authority() {
    let root = std::env::temp_dir().join(format!(
        "oomu-connector-authority-shared-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let first = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: "First".into(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let later = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: "Later".into(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let connector =
        repository::create_account(&engine, super::super::microsoft365::MANIFEST_ID, 1).unwrap();
    engine.open_connection().unwrap().execute(
        "UPDATE connector_accounts SET account_label='Work',connection_state='authorized',granted_scopes_json='[\"Mail.Read\"]' WHERE connector_id=?1",
        params![connector],
    ).unwrap();
    repository::set_project_scope(
        &engine,
        super::super::SetConnectorProjectScopeRequest {
            connector_id: connector.clone(),
            all_projects_enabled: true,
            enabled_project_ids: vec![first.project_id.clone()],
        },
    )
    .unwrap();

    assert!(repository::require_project_enabled(&engine, &connector, &later.project_id).is_ok());
    assert!(engine
        .validate_planned_connector_authority(
            &connector,
            Some(super::super::microsoft365::MANIFEST_ID),
            Some("Work"),
            Some(&later.project_id),
            "find_email",
        )
        .is_ok());

    repository::set_project_scope(
        &engine,
        super::super::SetConnectorProjectScopeRequest {
            connector_id: connector.clone(),
            all_projects_enabled: false,
            enabled_project_ids: vec![first.project_id],
        },
    )
    .unwrap();
    assert_eq!(
        repository::require_project_enabled(&engine, &connector, &later.project_id).unwrap_err(),
        "connector_project_authorization_required"
    );
    assert_eq!(
        engine
            .validate_planned_connector_authority(
                &connector,
                None,
                None,
                Some(&later.project_id),
                "find_email",
            )
            .unwrap_err(),
        "connector_planned_project_authorization_required"
    );
    let _ = std::fs::remove_dir_all(root);
}
