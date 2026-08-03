use super::*;
use rusqlite::params;

#[test]
fn calendar_and_mail_require_exact_extractable_authority_fields() {
    let objective = "Create a Calendar event titled `Recovery Review` in my `Executive Calendar` calendar. Schedule it on 2026-07-22 at 3:15 PM for 45 minutes at location `Board Room` with notes `Review release risks` and mark it tentative. Send an email to lead@example.com, cc reviewer@example.com, and bcc audit@example.com with subject `Go / No-Go` and body `Proceed only after verification`.";
    let mut draft = action_draft(vec![
        registered_step(
            "create_system_calendar_event",
            serde_json::json!({
                "calendarName":"Executive Calendar",
                "title":"Recovery Review",
                "startDate":"2026-07-23T15:00:00-04:00",
                "endDate":"2026-07-23T16:00:00-04:00",
                "location":"Wrong Room",
                "notes":"Review release risks",
                "availability":"tentative"
            }),
        ),
        registered_step(
            "send_system_email",
            serde_json::json!({
                "to":"lead@example.com,reviewer@example.com",
                "cc":"audit@example.com",
                "subject":"Go / No-Go",
                "body":"Proceed only after verification"
            }),
        ),
    ]);
    assert!(validate_objective_coverage(objective, &draft).is_err());

    let GeneratedToolDraft::RegisteredTaskTool { arguments, .. } = &mut draft.steps[0].tool else {
        panic!("Calendar test step")
    };
    arguments["startDate"] = serde_json::json!("2026-07-22T15:15:00-04:00");
    arguments["endDate"] = serde_json::json!("2026-07-22T16:00:00-04:00");
    arguments["location"] = serde_json::json!("Board Room");
    let GeneratedToolDraft::RegisteredTaskTool { arguments, .. } = &mut draft.steps[1].tool else {
        panic!("Mail test step")
    };
    arguments["to"] = serde_json::json!("lead@example.com");
    arguments["cc"] = serde_json::json!("reviewer@example.com");
    arguments["bcc"] = serde_json::json!("audit@example.com");
    validate_objective_coverage(objective, &draft).unwrap();
}

#[test]
fn named_connector_requires_the_requested_semantic_action() {
    let objective = "Use the MCP server named CRM to retrieve the release owner record.";
    let mut draft = action_draft(vec![registered_step(
        "connected_work",
        serde_json::json!({
            "connector_ref":"connector_11111111-1111-4111-8111-111111111111",
            "capability":"list_releases",
            "arguments":{}
        }),
    )]);
    assert!(validate_objective_coverage(objective, &draft).is_err());
    set_registered_argument(&mut draft, 0, "capability", "retrieve_release_owner_record");
    validate_objective_coverage(objective, &draft).unwrap();
}

#[test]
fn natural_service_reads_reject_unrelated_or_write_capabilities() {
    assert_read_only_service(
        "Use Apple Mail to check whether I have any unread emails.",
        "find_email",
        "draft_email",
    );
    for alias in [
        "Do I have any unread emails in my inbox?",
        "Check my mail for unread messages.",
        "Use the mail app to find unread email.",
    ] {
        assert_read_only_service(alias, "find_email", "draft_email");
    }
    assert_read_only_service(
        "Use Apple Calendar to check my schedule.",
        "read_calendar",
        "draft_calendar_event",
    );
    for alias in [
        "Check my calendar for meetings.",
        "Use the calendar app to show my schedule.",
    ] {
        assert_read_only_service(alias, "read_calendar", "draft_calendar_event");
    }
}

#[test]
fn persistence_binding_rejects_the_wrong_manifest_or_named_account() {
    let root = std::env::temp_dir().join(format!(
        "oomu-plan-connector-binding-{}",
        crate::p0_contracts::ConnectorId::new()
    ));
    let persistence = crate::db::PersistenceEngine::initialize_at(root.join("state.sqlite"))
        .expect("test persistence");
    let apple = crate::p0_contracts::ConnectorId::new().to_string();
    let slack = crate::p0_contracts::ConnectorId::new().to_string();
    let now = crate::foundation::clock::unix_time_ms_i64();
    let connection = persistence.open_connection().unwrap();
    for (connector, manifest, label) in [
        (&apple, "apple_apps", "CRM"),
        (&slack, "slack", "Workspace"),
    ] {
        connection.execute(
            "INSERT INTO connector_accounts (connector_id,manifest_id,credential_ref,account_label,connection_state,schema_version,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,?4,'configured',1,?5,?5)",
            params![connector, manifest, format!("credential_{connector}"), label, now],
        ).unwrap();
    }
    assert_eq!(
        persistence
            .validate_planned_connector_authority(
                &apple,
                Some("apple_apps"),
                Some("CRM"),
                None,
                "find_email"
            )
            .unwrap_err(),
        "connector_planned_adapter_unavailable"
    );
    assert_eq!(
        persistence
            .validate_planned_connector_authority(
                &slack,
                Some("apple_apps"),
                None,
                None,
                "find_email"
            )
            .unwrap_err(),
        "connector_planned_manifest_mismatch"
    );
    assert_eq!(
        persistence
            .validate_planned_connector_authority(
                &apple,
                Some("apple_apps"),
                Some("ERP"),
                None,
                "find_email"
            )
            .unwrap_err(),
        "connector_planned_account_mismatch"
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unquoted_mail_subject_and_body_are_exact_authority() {
    let objective = "Send an email to reviewer@example.com with subject OOMU Test — Supplier Exception and body Proceed only after verification and attach the report.";
    let mut draft = action_draft(vec![registered_step(
        "send_system_email",
        serde_json::json!({
            "to":"reviewer@example.com",
            "subject":"Wrong subject",
            "body":"Proceed only after verification"
        }),
    )]);
    assert!(validate_objective_coverage(objective, &draft).is_err());
    set_registered_argument(&mut draft, 0, "subject", "OOMU Test — Supplier Exception");
    validate_objective_coverage(objective, &draft).unwrap();
    set_registered_argument(&mut draft, 0, "body", "Wrong body");
    assert!(validate_objective_coverage(objective, &draft).is_err());

    let scenario_six = "Send an email to reviewer@example.com with subject OOMU Test — Supplier Exception and the report attached or linked.";
    let scenario_draft = action_draft(vec![registered_step(
        "send_system_email",
        serde_json::json!({
            "to":"reviewer@example.com",
            "subject":"OOMU Test — Supplier Exception",
            "body":"The verified report is attached."
        }),
    )]);
    validate_objective_coverage(scenario_six, &scenario_draft).unwrap();
}

#[test]
fn connector_and_channel_require_exact_extractable_authority_fields() {
    let objective = "Use the MCP server named CRM with capability `get_owner` and arguments {\"id\":\"A1\"}; disable the Slack channel for owner `operations`.";
    let mut draft = action_draft(vec![
        registered_step(
            "connected_work",
            serde_json::json!({
                "connector_ref":"connector_11111111-1111-4111-8111-111111111111",
                "capability":"get_owner",
                "arguments":{"id":"A1"}
            }),
        ),
        registered_step(
            "configure_channel",
            serde_json::json!({
                "platform":"slack",
                "is_active":true,
                "owner_id":"operations"
            }),
        ),
    ]);
    assert!(validate_objective_coverage(objective, &draft).is_err());
    let GeneratedToolDraft::RegisteredTaskTool { arguments, .. } = &mut draft.steps[1].tool else {
        panic!("channel test step")
    };
    arguments["is_active"] = serde_json::json!(false);
    validate_objective_coverage(objective, &draft).unwrap();
}

fn assert_read_only_service(objective: &str, read: &str, write: &str) {
    let mut draft = action_draft(vec![registered_step(
        "connected_work",
        serde_json::json!({
            "connector_ref":"connector_11111111-1111-4111-8111-111111111111",
            "capability":"find_chat_messages",
            "arguments":{}
        }),
    )]);
    assert!(validate_objective_coverage(objective, &draft).is_err());
    set_registered_argument(&mut draft, 0, "capability", write);
    assert!(validate_objective_coverage(objective, &draft).is_err());
    set_registered_argument(&mut draft, 0, "capability", read);
    validate_objective_coverage(objective, &draft).unwrap();
}

fn action_draft(steps: Vec<GeneratedPlanStepDraft>) -> GeneratedActionPlanDraft {
    GeneratedActionPlanDraft {
        steps,
        exit_condition: "Verify every requested action.".to_string(),
        generated_text: "{}".to_string(),
        source: IntentSource::Deterministic,
        degraded_reason: None,
    }
}

fn registered_step(operation: &str, arguments: Value) -> GeneratedPlanStepDraft {
    GeneratedPlanStepDraft {
        step: operation.to_string(),
        tool: GeneratedToolDraft::RegisteredTaskTool {
            operation: operation.to_string(),
            arguments,
        },
        risk_level: GeneratedRiskLevel::High,
    }
}

fn set_registered_argument(
    draft: &mut GeneratedActionPlanDraft,
    step: usize,
    field: &str,
    value: &str,
) {
    let GeneratedToolDraft::RegisteredTaskTool { arguments, .. } = &mut draft.steps[step].tool
    else {
        panic!("registered test step")
    };
    arguments[field] = Value::String(value.to_string());
}
