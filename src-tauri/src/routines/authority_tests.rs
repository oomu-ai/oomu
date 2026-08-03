use super::*;
use crate::{
    db::PersistenceEngine,
    projects::{CreateProjectRequest, ProjectDataPolicy},
};
use rusqlite::params;
use serde_json::{json, Value};
use std::{fs, path::PathBuf};

struct Fixture {
    root: PathBuf,
    engine: PersistenceEngine,
    project_id: String,
    workflow_id: String,
    routine_id: String,
    instance_id: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn request(project_id: &str, workflow_id: &str, delivery: Value) -> CreateRoutineRequest {
    CreateRoutineRequest {
        confirmed: true,
        label: "Reviewed morning brief".to_string(),
        project_id: project_id.to_string(),
        workflow_id: workflow_id.to_string(),
        workflow_version: 1,
        schedule_expression: "0 9 * * *".to_string(),
        schedule_kind: "recurring".to_string(),
        timezone: "UTC".to_string(),
        active_window_start_minute: None,
        active_window_end_minute: None,
        end_boundary: None,
        run_once_after_create: false,
        missed_run_policy: "skip".to_string(),
        missed_run_cap: 3,
        task_template: json!({}),
        model_route: json!({"mode":"workflow_default"}),
        delivery_target: delivery,
        authority: json!({"mode":"reviewed_workflow_scope"}),
    }
}

fn fixture(label: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "oomu-routine-authority-{label}-{}",
        crate::p0_contracts::TaskId::new()
    ));
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let input = root.join("reviewed_project_source.json");
    fs::write(&input, br#"{"supplier":"A"}"#).unwrap();
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: format!("Authority {label}"),
            description: String::new(),
            data_policy: ProjectDataPolicy::AllowConfiguredCloud,
        },
    )
    .unwrap();
    let project_id = project.project_id;
    let workflow_id = format!("workflow-authority-{}", crate::p0_contracts::TaskId::new());
    let ir = json!({
        "schemaVersion": crate::workflow_ir::WORKFLOW_IR_SCHEMA_VERSION,
        "workflowId": workflow_id,
        "workflowVersion": 1,
        "name": "Reviewed Workflow",
        "compiler": {"model": crate::workflow_ir::WORKFLOW_COMPILER_MODEL},
        "nodes": [
            {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
            {"kind":"mcp_tool","id":"read-project-file","label":"Read","serverName":"local_filesystem","toolName":"read_file","arguments":{"path":input}},
            {"kind":"mcp_tool","id":"create-markdown","label":"Write Markdown","serverName":"oomu_task_tools","toolName":"create_file","arguments":{"file":{"title":"Brief","content":"{{nodes.summary.output}}","locale":"en-US","format":"md","destinationPath":"{{workflow.input.markdownPath}}"}}},
            {"kind":"mcp_tool","id":"create-pdf","label":"Write PDF","serverName":"oomu_task_tools","toolName":"create_file","arguments":{"file":{"title":"Brief","content":"{{nodes.summary.output}}","locale":"en-US","format":"pdf","destinationPath":"{{workflow.input.pdfPath}}"}}},
            {"kind":"mcp_tool","id":"timestamped-report","label":"Write timestamped report","serverName":"oomu_task_tools","toolName":"create_file","arguments":{"file":{"title":"Supplier exception","content":"{{nodes.summary.output}}","locale":"en-US","format":"md","destinationPath":"ship_test_06/supplier_exception_<YYYY-MM-DD_HH-mm>.md"}}},
            {"kind":"mcp_tool","id":"official-source","label":"Official source","serverName":"oomu_task_tools","toolName":"fetch_official_page","arguments":{"url":"{{workflow.input.url}}","maxContentChars":4000}},
            {"kind":"mcp_tool","id":"read-mail","label":"Read Mail","serverName":"macos_applescript","toolName":"read_system_emails","arguments":{"max_messages":5,"unread_only":true}},
            {"kind":"mcp_tool","id":"read-calendar","label":"Read Calendar","serverName":"macos_applescript","toolName":"read_system_calendar","arguments":{"hours_ahead":24}},
            {"kind":"permission","id":"draft-permission","label":"Approve draft","permission":"mcp_tool","reason":"Open the exact Mail draft.","onDenied":"fail"},
            {"kind":"mcp_tool","id":"draft-email","label":"Draft","serverName":"macos_applescript","toolName":"draft_system_email","arguments":{"to":"tester@example.com","subject":"Reviewed brief","body":"{{nodes.summary.output}}"}},
            {"kind":"permission","id":"calendar-permission","label":"Approve Calendar","permission":"mcp_tool","reason":"Create the exact event.","onDenied":"branch"},
            {"kind":"mcp_tool","id":"calendar","label":"Calendar","serverName":"oomu_task_tools","toolName":"create_conflict_free_calendar_event","arguments":{"calendarName":"OOMU Test"}},
            {"kind":"permission","id":"email-permission","label":"Approve email","permission":"mcp_tool","reason":"Send the exact email.","onDenied":"branch"},
            {"kind":"mcp_tool","id":"send-email","label":"Send","serverName":"oomu_task_tools","toolName":"send_system_email","arguments":{"to":"tester@example.com"}},
            {"kind":"output","id":"output","label":"Done","inputMapping":"{{nodes.create-pdf.output}}","outputSchema":{"type":"object"}}
        ],
        "edges": [
            {"id":"calendar-approved","sourceNodeId":"calendar-permission","sourcePort":"approved","targetNodeId":"calendar"},
            {"id":"calendar-denied","sourceNodeId":"calendar-permission","sourcePort":"denied","targetNodeId":"output"},
            {"id":"draft-approved","sourceNodeId":"draft-permission","sourcePort":"approved","targetNodeId":"draft-email"},
            {"id":"email-approved","sourceNodeId":"email-permission","sourcePort":"approved","targetNodeId":"send-email"},
            {"id":"email-denied","sourceNodeId":"email-permission","sourcePort":"denied","targetNodeId":"output"}
        ]
    });
    let connection = engine.open_connection().unwrap();
    connection.execute(
        "INSERT INTO project_sources(source_id,project_id,source_kind,canonical_path,grant_reference,grant_state,indexing_state,file_count,created_at_ms,updated_at_ms) VALUES (?1,?2,'local_folder',?3,'grant-test','active','ready',1,1,1)",
        params![format!("source-{label}"), project_id, root.to_string_lossy()],
    ).unwrap();
    connection.execute(
        "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,workflow_ir_json,compilation_status,is_active,created_at_ms,updated_at_ms,encryption_state,project_id) VALUES (?1,1,'Reviewed Workflow','','{}',?2,'Compiled',1,1,1,'test',?3)",
        params![workflow_id, ir.to_string(), project_id],
    ).unwrap();
    drop(connection);
    let routine = repository::create(
        &engine,
        request(
            &project_id,
            &workflow_id,
            json!({"platform":"slack","destination":"C123ALLOWED"}),
        ),
    )
    .unwrap();
    let instance_id = format!("instance-{label}");
    let connection = engine.open_connection().unwrap();
    connection.execute(
        "INSERT INTO execution_instances(id,workflow_id,workflow_version,status,created_at_ms,updated_at_ms,encryption_state,project_id) VALUES (?1,?2,1,'Running',1,1,'test',?3)",
        params![instance_id, workflow_id, project_id],
    ).unwrap();
    connection.execute(
        "INSERT INTO routine_runs(schedule_id,execution_instance_id,scheduled_for_ms,created_at_ms) VALUES (?1,?2,1,1)",
        params![routine.routine_id, instance_id],
    ).unwrap();
    drop(connection);
    Fixture {
        root,
        engine,
        project_id,
        workflow_id,
        routine_id: routine.routine_id,
        instance_id,
    }
}

fn stored_manifest(fixture: &Fixture) -> Value {
    let raw: String = fixture
        .engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT authority_json FROM workflow_schedules WHERE id=?1",
            params![fixture.routine_id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn create_persists_only_server_derived_reviewed_scope() {
    let fixture = fixture("persisted");
    let manifest = stored_manifest(&fixture);
    assert_eq!(manifest["mode"], "reviewed_workflow_scope");
    assert_eq!(manifest["scheduleId"], fixture.routine_id);
    assert_eq!(manifest["workflowId"], fixture.workflow_id);
    assert_eq!(manifest["workflowVersion"], 1);
    assert_eq!(manifest["projectId"], fixture.project_id);
    assert_eq!(
        manifest["projectRoots"],
        json!([fs::canonicalize(&fixture.root).unwrap()])
    );
    let node_ids = manifest["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|node| node["nodeId"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        node_ids,
        vec![
            "read-project-file",
            "create-markdown",
            "create-pdf",
            "timestamped-report",
            "official-source",
            "read-mail",
            "read-calendar"
        ]
    );
    assert!(!node_ids.contains(&"calendar"));
    assert!(!node_ids.contains(&"draft-email"));
    assert!(!node_ids.contains(&"send-email"));
    assert!(!node_ids.contains(&"untrusted-source"));
    assert!(!manifest.to_string().contains("+15551234567"));
}

#[test]
fn reviewed_recurring_routine_can_run_now_and_still_stop_at_midnight() {
    let fixture = fixture("run-now-until-midnight");
    let mut request = request(&fixture.project_id, &fixture.workflow_id, json!({}));
    request.schedule_expression = "every 1 hour".to_string();
    request.end_boundary = Some(RoutineEndBoundary::Midnight);
    request.run_once_after_create = true;
    let before = crate::foundation::clock::unix_time_ms_i64();

    let routine = repository::create(&fixture.engine, request).unwrap();
    let after = crate::foundation::clock::unix_time_ms_i64();
    let saved = fixture
        .engine
        .load_workflow_schedule(&routine.routine_id)
        .unwrap();
    let next = saved.next_run_at_ms.unwrap();
    let end = control::end_at_ms(&saved.run_request).unwrap().unwrap();

    assert!((before..=after).contains(&next));
    assert!(end > next);
    assert!(end <= after + 25 * 60 * 60 * 1_000);
}

#[test]
fn recurring_routine_without_a_run_before_midnight_is_rejected_truthfully() {
    let fixture = fixture("no-run-before-midnight");
    let mut request = request(&fixture.project_id, &fixture.workflow_id, json!({}));
    request.schedule_expression = "every 1 week".to_string();
    request.end_boundary = Some(RoutineEndBoundary::Midnight);

    let error = repository::create(&fixture.engine, request).unwrap_err();
    assert_eq!(
        error,
        "The recurring schedule has no run before its end boundary."
    );
}

#[test]
fn verifier_binds_exact_bounded_apple_mail_and_calendar_reads() {
    let fixture = fixture("apple-reads");
    assert!(verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "read-mail",
        "read_system_emails",
        &json!({"max_messages":5,"unread_only":true})
    )
    .unwrap());
    assert!(!verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "read-mail",
        "read_system_emails",
        &json!({"max_messages":50,"unread_only":false})
    )
    .unwrap());
    assert!(verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "read-calendar",
        "read_system_calendar",
        &json!({"hours_ahead":24})
    )
    .unwrap());
    assert!(!verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "read-calendar",
        "read_system_calendar",
        &json!({"hours_ahead":720})
    )
    .unwrap());
}

#[test]
fn exact_mail_draft_accepts_authored_fail_or_branch_but_never_unguarded() {
    let fixture = fixture("draft-denial-policies");
    let connection = fixture.engine.open_connection().unwrap();
    let raw: String = connection
        .query_row(
            "SELECT workflow_ir_json FROM workflow_blueprints WHERE workflow_id=?1 AND version=1",
            params![fixture.workflow_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut ir: Value = serde_json::from_str(&raw).unwrap();

    // The fixture's onDenied=fail draft has already passed Routine creation.
    let fail_routine = repository::create(
        &fixture.engine,
        request(&fixture.project_id, &fixture.workflow_id, json!({})),
    )
    .expect("an exact fail-on-denial Mail draft remains schedulable");
    assert!(!fail_routine.routine_id.is_empty());

    let draft_permission = ir["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node["id"] == "draft-permission")
        .unwrap();
    draft_permission["onDenied"] = json!("branch");
    ir["edges"].as_array_mut().unwrap().push(json!({
        "id":"draft-denied",
        "sourceNodeId":"draft-permission",
        "sourcePort":"denied",
        "targetNodeId":"output"
    }));
    connection
        .execute(
            "UPDATE workflow_blueprints SET workflow_ir_json=?2 WHERE workflow_id=?1 AND version=1",
            params![fixture.workflow_id, ir.to_string()],
        )
        .unwrap();
    let branch_routine = repository::create(
        &fixture.engine,
        request(&fixture.project_id, &fixture.workflow_id, json!({})),
    )
    .expect("an exact branch-on-denial Mail draft remains schedulable");
    assert!(!branch_routine.routine_id.is_empty());

    ir["edges"]
        .as_array_mut()
        .unwrap()
        .retain(|edge| edge["id"] != "draft-approved");
    connection
        .execute(
            "UPDATE workflow_blueprints SET workflow_ir_json=?2 WHERE workflow_id=?1 AND version=1",
            params![fixture.workflow_id, ir.to_string()],
        )
        .unwrap();
    let error = repository::create(
        &fixture.engine,
        request(&fixture.project_id, &fixture.workflow_id, json!({})),
    )
    .expect_err("an unbound Mail draft cannot be scheduled");
    assert!(error.contains("direct approval step"));
}

#[test]
fn verifier_binds_nested_create_file_paths_templates_and_trusted_server() {
    let fixture = fixture("verify");
    let markdown = fixture.root.join("ship_test_05/brief.md");
    let pdf = fixture.root.join("ship_test_05/brief.pdf");
    let markdown_arguments = json!({"file":{"title":"Brief","content":"evidence","locale":"en-US","format":"md","destinationPath":markdown}});
    let pdf_arguments = json!({"file":{"title":"Brief","content":"evidence","locale":"en-US","format":"pdf","destinationPath":pdf}});
    assert!(verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "create-markdown",
        "create_file",
        &markdown_arguments
    )
    .unwrap());
    assert!(verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "create-pdf",
        "create_file",
        &pdf_arguments
    )
    .unwrap());
    let outside = json!({"file":{"title":"Brief","content":"evidence","locale":"en-US","format":"md","destinationPath":"/tmp/outside.md"}});
    assert!(!verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "create-markdown",
        "create_file",
        &outside
    )
    .unwrap());
    assert!(!verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "untrusted-source",
        "fetch_official_page",
        &json!({"url":"https://example.com/report","maxContentChars":4000})
    )
    .unwrap());
}

#[test]
fn verifier_strictly_binds_resolved_relative_task_timestamp_paths() {
    let fixture = fixture("timestamp-path");
    let arguments = |name: &str| {
        json!({"file":{
            "title":"Supplier exception",
            "content":"evidence",
            "locale":"en-US",
            "format":"md",
            "destinationPath":fixture.root.join("ship_test_06").join(name)
        }})
    };
    assert!(verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "timestamped-report",
        "create_file",
        &arguments("supplier_exception_2026-07-21_10-30.md")
    )
    .unwrap());
    assert!(verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "timestamped-report",
        "create_file",
        &arguments("supplier_exception_<YYYY-MM-DD_HH-mm>.md")
    )
    .unwrap());
    for invalid in [
        "supplier_exception_2026-13-21_10-30.md",
        "supplier_exception_2026-07-21_10-30-extra.md",
    ] {
        assert!(!verify_reviewed_workflow_scope(
            &fixture.engine,
            &fixture.instance_id,
            "timestamped-report",
            "create_file",
            &arguments(invalid)
        )
        .unwrap());
    }
    assert!(!verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "timestamped-report",
        "create_file",
        &json!({"file":{
            "title":"Supplier exception","content":"evidence","locale":"en-US","format":"md",
            "destinationPath":"/tmp/ship_test_06/supplier_exception_2026-07-21_10-30.md"
        }})
    )
    .unwrap());
}

#[test]
fn relative_outputs_fail_review_when_project_root_is_ambiguous() {
    let fixture = fixture("ambiguous-relative-output");
    let second = fixture.root.join("second-root");
    fs::create_dir_all(&second).unwrap();
    let second = fs::canonicalize(second).unwrap();
    fixture
        .engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO project_sources(source_id,project_id,source_kind,canonical_path,grant_reference,grant_state,indexing_state,file_count,created_at_ms,updated_at_ms) VALUES ('source-second-root',?1,'knowledge_directory',?2,'grant-second','active','ready',0,2,2)",
            params![fixture.project_id, second.to_string_lossy()],
        )
        .unwrap();
    let error = repository::create(
        &fixture.engine,
        request(&fixture.project_id, &fixture.workflow_id, json!({})),
    )
    .unwrap_err();
    assert!(error.contains("exactly one approved Project folder"));
}

#[test]
fn non_folder_project_sources_do_not_expand_reviewed_file_authority() {
    let fixture = fixture("non-folder-source");
    let error = fixture
        .engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO project_sources(source_id,project_id,source_kind,canonical_path,grant_reference,grant_state,indexing_state,file_count,created_at_ms,updated_at_ms) VALUES ('source-web',?1,'web_source','https://example.com/official','grant-web','active','ready',0,2,2)",
            params![fixture.project_id],
        )
        .unwrap_err();
    assert!(error.to_string().contains("CHECK constraint failed"));
    let routine = repository::create(
        &fixture.engine,
        request(&fixture.project_id, &fixture.workflow_id, json!({})),
    )
    .unwrap();
    let authority: String = fixture
        .engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT authority_json FROM workflow_schedules WHERE id=?1",
            params![routine.routine_id],
            |row| row.get(0),
        )
        .unwrap();
    let authority: Value = serde_json::from_str(&authority).unwrap();
    assert_eq!(
        authority["projectRoots"],
        json!([fs::canonicalize(&fixture.root).unwrap()])
    );
    assert!(!authority.to_string().contains("example.com"));
}

#[test]
fn verifier_allows_public_official_reads_and_exact_terminal_delivery_only() {
    let fixture = fixture("network-delivery");
    assert!(verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "official-source",
        "fetch_official_page",
        &json!({"url":"https://www.energy.gov/report","maxContentChars":4000})
    )
    .unwrap());
    assert!(!verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "official-source",
        "fetch_official_page",
        &json!({"url":"http://www.energy.gov/report","maxContentChars":4000})
    )
    .unwrap());
    assert!(!verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "official-source",
        "fetch_official_page",
        &json!({"url":"https://127.0.0.1/report","maxContentChars":4000})
    )
    .unwrap());
    assert!(verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        TERMINAL_DELIVERY_NODE_ID,
        TERMINAL_DELIVERY_TOOL,
        &json!({"platform":"slack","destination":"C123ALLOWED"})
    )
    .unwrap());
    assert!(!verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        TERMINAL_DELIVERY_NODE_ID,
        TERMINAL_DELIVERY_TOOL,
        &json!({"platform":"slack","destination":"C999OTHER"})
    )
    .unwrap());
    for (node, tool) in [
        ("calendar", "create_conflict_free_calendar_event"),
        ("send-email", "send_system_email"),
        ("missing", "create_file"),
    ] {
        assert!(!verify_reviewed_workflow_scope(
            &fixture.engine,
            &fixture.instance_id,
            node,
            tool,
            &json!({})
        )
        .unwrap());
    }
}

#[test]
fn client_cannot_supply_actions_and_project_changes_fail_closed() {
    let fixture = fixture("fail-closed");
    let mut bad = request(&fixture.project_id, &fixture.workflow_id, json!({}));
    bad.authority = json!({"mode":"reviewed_workflow_scope","actions":["send_system_email"]});
    let error = repository::create(&fixture.engine, bad).unwrap_err();
    assert!(error.contains("must contain only"));
    fixture
        .engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE project_sources SET grant_state='revoked' WHERE project_id=?1",
            params![fixture.project_id],
        )
        .unwrap();
    let result = verify_reviewed_workflow_scope(
        &fixture.engine,
        &fixture.instance_id,
        "create-markdown",
        "create_file",
        &json!({"file":{"title":"Brief","content":"evidence","locale":"en-US","format":"md","destinationPath":fixture.root.join("brief.md")}}),
    );
    assert!(result.is_err() || result == Ok(false));
}

#[test]
fn unsupported_or_unguarded_mcp_actions_block_routine_creation() {
    let fixture = fixture("unsupported");
    let connection = fixture.engine.open_connection().unwrap();
    let raw: String = connection
        .query_row(
            "SELECT workflow_ir_json FROM workflow_blueprints WHERE workflow_id=?1 AND version=1",
            params![fixture.workflow_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut ir: Value = serde_json::from_str(&raw).unwrap();
    ir["nodes"].as_array_mut().unwrap().push(json!({
        "kind":"mcp_tool",
        "id":"unsupported",
        "label":"Unsupported",
        "serverName":"untrusted_server",
        "toolName":"fetch_official_page",
        "arguments":{"url":"https://example.com"}
    }));
    connection
        .execute(
            "UPDATE workflow_blueprints SET workflow_ir_json=?2 WHERE workflow_id=?1 AND version=1",
            params![fixture.workflow_id, ir.to_string()],
        )
        .unwrap();
    drop(connection);
    let error = repository::create(
        &fixture.engine,
        request(&fixture.project_id, &fixture.workflow_id, json!({})),
    )
    .unwrap_err();
    assert!(error.contains("cannot be included"));

    ir["nodes"]
        .as_array_mut()
        .unwrap()
        .retain(|node| node["id"] != "unsupported");
    ir["edges"]
        .as_array_mut()
        .unwrap()
        .retain(|edge| edge["id"] != "calendar-approved");
    fixture
        .engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE workflow_blueprints SET workflow_ir_json=?2 WHERE workflow_id=?1 AND version=1",
            params![fixture.workflow_id, ir.to_string()],
        )
        .unwrap();
    let error = repository::create(
        &fixture.engine,
        request(&fixture.project_id, &fixture.workflow_id, json!({})),
    )
    .unwrap_err();
    assert!(error.contains("direct approval step"));
}

#[test]
fn linked_reviewed_routine_requires_the_manifest_for_every_mcp_call() {
    let fixture = fixture("required");
    assert!(reviewed_workflow_scope_required(&fixture.engine, &fixture.instance_id).unwrap());
    assert!(!reviewed_workflow_scope_required(&fixture.engine, "not-a-routine-run").unwrap());
}

#[test]
fn routine_creation_distinguishes_unbound_mismatched_and_unavailable_workflows() {
    let fixture = fixture("binding-errors");
    let connection = fixture.engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE workflow_blueprints SET project_id=NULL WHERE workflow_id=?1 AND version=1",
            params![fixture.workflow_id],
        )
        .unwrap();
    drop(connection);
    assert_eq!(
        repository::create(
            &fixture.engine,
            request(&fixture.project_id, &fixture.workflow_id, json!({})),
        )
        .unwrap_err(),
        "routine_workflow_project_binding_required"
    );

    let second = crate::projects::repository::create(
        &fixture.engine,
        CreateProjectRequest {
            name: "Different Project".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::AllowConfiguredCloud,
        },
    )
    .unwrap();
    fixture
        .engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE workflow_blueprints SET project_id=?2 WHERE workflow_id=?1 AND version=1",
            params![fixture.workflow_id, second.project_id],
        )
        .unwrap();
    assert_eq!(
        repository::create(
            &fixture.engine,
            request(&fixture.project_id, &fixture.workflow_id, json!({})),
        )
        .unwrap_err(),
        "routine_workflow_project_mismatch"
    );

    fixture
        .engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE workflow_blueprints SET project_id=?2,compilation_status='Failed' WHERE workflow_id=?1 AND version=1",
            params![fixture.workflow_id, fixture.project_id],
        )
        .unwrap();
    assert_eq!(
        repository::create(
            &fixture.engine,
            request(&fixture.project_id, &fixture.workflow_id, json!({})),
        )
        .unwrap_err(),
        "routine_workflow_version_unavailable"
    );
}
