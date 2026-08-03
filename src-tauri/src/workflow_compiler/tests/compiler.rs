use super::*;

#[test]
fn sprint_304_instruction_free_workflow_skips_model_compiler() {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
        "schemaVersion": "1.0.0",
        "workflowId": "wf-sprint-304-local-action",
        "workflowVersion": 1,
        "name": "Background file action",
        "description": "Write one test-owned file without model inference.",
        "compiler": { "model": "gemma-4-e2b-qat" },
        "nodes": [
            {
                "kind": "input",
                "id": "input",
                "label": "Input",
                "outputKey": "workflow.input",
                "inputSchema": { "type": "object" }
            },
            {
                "kind": "system_action",
                "id": "write-marker",
                "label": "Write marker",
                "actionType": "shell",
                "command": "echo",
                "args": ["done"]
            },
            {
                "kind": "output",
                "id": "output",
                "label": "Done",
                "inputMapping": "done",
                "outputSchema": { "type": "string" }
            }
        ],
        "edges": [
            {
                "id": "e1",
                "sourceNodeId": "input",
                "sourcePort": "out",
                "targetNodeId": "write-marker"
            },
            {
                "id": "e2",
                "sourceNodeId": "write-marker",
                "sourcePort": "out",
                "targetNodeId": "output"
            }
        ]
    }))
    .unwrap();

    let output = deterministic_instruction_free_output(&workflow_ir)
        .expect("instruction-free workflows compile deterministically");

    assert_eq!(output.compiler_version, COMPILER_VERSION);
    assert!(output.instructions.is_empty());
    validate_compiler_output(&output, &workflow_ir).unwrap();
}

#[test]
fn trusted_builtin_collection_contracts_do_not_depend_on_connection_timing() {
    for (server_name, tool_name, collection_name) in [
        ("local_filesystem", "list_directory", "files"),
        ("local_search", "search_web", "results"),
        ("macos_applescript", "read_system_calendar", "events"),
        ("macos_applescript", "read_system_emails", "emails"),
        ("macos_applescript", "read_system_notes", "notes"),
        ("macos_applescript", "read_system_contacts", "contacts"),
        ("macos_applescript", "read_system_reminders", "reminders"),
        ("macos_applescript", "read_apple_app_ui", "uiText"),
    ] {
        let schema = trusted_builtin_mcp_output_schema(server_name, tool_name)
            .unwrap_or_else(|| panic!("missing trusted contract for {server_name}/{tool_name}"));
        assert_eq!(
            schema["x-oomu-result-contract"]["path"],
            json!(format!("/structuredContent/{collection_name}"))
        );
        assert_eq!(
            schema["properties"]["structuredContent"]["properties"][collection_name]["type"],
            json!("array")
        );
    }
    assert!(trusted_builtin_mcp_output_schema("custom_mcp", "read_items").is_none());
    assert!(trusted_builtin_mcp_output_schema("macos_applescript", "read_system_music").is_none());
}

#[test]
fn email_template_remains_saveable_while_apple_helper_is_connecting() {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion":"1.0.0","workflowId":"wf-email-offline-save","workflowVersion":1,
            "name":"Email Responder","description":"Read unread mail and prepare a reviewed draft.",
            "compiler":{"model":"gemma-4-e2b-qat"},
            "nodes":[
                {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
                {"kind":"mcp_tool","id":"read-unread-emails","label":"Read macOS Emails","serverName":"macos_applescript","toolName":"read_system_emails","arguments":{"max_messages":5,"unread_only":true},"outputSchema":{"clientValue":"not authoritative"}},
                {"kind":"conditional","id":"mail-has-messages","label":"Check mail","condition":"$ != []","inputMapping":"{{nodes.read-unread-emails.output.data.structuredContent.emails}}"},
                {"kind":"agent","id":"draft-reply","label":"Draft reply","objective":"Draft a professional reply.","inputMappings":{"context":"{{nodes.read-unread-emails.output}}"},"outputKey":"nodes.draft-reply.output"},
                {"kind":"output","id":"output","label":"Ready","inputMapping":"{{nodes.draft-reply.output}}","outputSchema":{"type":"object"}},
                {"kind":"output","id":"empty-output","label":"Nothing found","inputMapping":"{{nodes.read-unread-emails.output.data.structuredContent.emails}}","outputSchema":{"type":"array"},"completionKind":"empty_collection"}
            ],
            "edges":[
                {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read-unread-emails"},
                {"id":"e2","sourceNodeId":"read-unread-emails","sourcePort":"out","targetNodeId":"mail-has-messages"},
                {"id":"e3","sourceNodeId":"mail-has-messages","sourcePort":"true","targetNodeId":"draft-reply"},
                {"id":"e4","sourceNodeId":"mail-has-messages","sourcePort":"false","targetNodeId":"empty-output"},
                {"id":"e5","sourceNodeId":"draft-reply","sourcePort":"out","targetNodeId":"output"}
            ]
        }))
        .unwrap();
    let catalog = CapabilityCatalog {
        version: "test".to_string(),
        authoring_enabled: true,
        generated_at_ms: 1,
        actions: known_mcp_capabilities(&HashMap::new()),
        templates: Vec::new(),
    };

    let root = std::env::temp_dir().join(format!(
        "oomu_email_responder_save_{}_{}",
        std::process::id(),
        unix_time_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let persistence = PersistenceEngine::initialize_at(root.join("workflow.sqlite")).unwrap();
    let identity = SovereignIdentity::initialize_ephemeral();
    let response = compile_and_save_workflow(
        SaveWorkflowRequest {
            project_id: None,
            workflow: SavedWorkflowRecord {
                id: workflow_ir.workflow_id.clone(),
                name: workflow_ir.name.clone(),
                steps: r#"{"nodes":[],"edges":[]}"#.to_string(),
                created_at: 1,
                updated_at: 1,
            },
            visual_state: json!({"nodes": [], "edges": []}),
            workflow_ir,
            activate: true,
        },
        &catalog,
        &persistence,
        &DeterministicTestCompiler,
        &identity,
    )
    .expect("Email Responder saves while the Apple helper is connecting");

    assert_eq!(response.compilation_status, "Compiled");
    assert_eq!(response.compiled_node_count, 1);
    let compiled = persistence
        .load_compiled_workflow("wf-email-offline-save", Some(response.workflow_version))
        .unwrap();
    let WorkflowNode::McpTool(read_mail) = &compiled.workflow_ir.nodes[1] else {
        panic!("expected Mail reader");
    };
    assert_eq!(
        read_mail.output_schema.as_ref().unwrap()["x-oomu-result-contract"]["path"],
        json!("/structuredContent/emails")
    );
    assert_eq!(compiled.instructions.len(), 1);
    let _ = std::fs::remove_dir_all(root);
}

fn scenario_five_request(project_id: Option<String>) -> ComposeWorkflowRequest {
    ComposeWorkflowRequest {
        prompt: r#"At each run, read `/tmp/testing/supplier_proposals.json` and `/tmp/testing/project_milestones.json` from the testing folder. Retrieve current information from at least two relevant primary or official public web sources, including one current energy/fuel source and one transport, logistics, or government source. Record each URL and access time. Reconcile supplier rate variances, identify unfinished milestones, and explain only evidence-backed changes since the local fixture dates. Create `/tmp/testing/ship_test_05/operations_brief_<YYYY-MM-DD_HH-mm>.md` and a matching PDF. Both must include a one-paragraph executive summary, data table, exceptions, milestone risks, current web evidence, source links, and next actions. Read both files back or validate them before completion. Deliver a concise summary to the Routine's configured channel with the two exact filenames and a truthful success/failure status. Never report a file as created unless it exists."#.to_string(),
        capability_catalog: CapabilityCatalog {
            version: "test".to_string(),
            authoring_enabled: true,
            generated_at_ms: 1,
            actions: registered_task_capabilities::catalog_actions().unwrap(),
            templates: Vec::new(),
        },
        project_id,
        workflow_id: Some("wf-scenario-five-project-binding".to_string()),
        name: Some("Ship Test 05 — Morning Operations Brief".to_string()),
    }
}

#[test]
fn scenario_five_save_persists_exact_project_and_authoritative_review_projection() {
    registered_task_capabilities::register_test_tools();
    let root = std::env::temp_dir().join(format!(
        "oomu_scenario_five_project_save_{}_{}",
        std::process::id(),
        unix_time_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let persistence = PersistenceEngine::initialize_at(root.join("workflow.sqlite")).unwrap();
    let project = crate::projects::repository::create(
        &persistence,
        crate::projects::CreateProjectRequest {
            name: "Scenario five".to_string(),
            description: String::new(),
            data_policy: crate::projects::ProjectDataPolicy::AllowConfiguredCloud,
        },
    )
    .unwrap();
    let identity = SovereignIdentity::initialize_ephemeral();

    let save = |request: ComposeWorkflowRequest, updated_at: i64| {
        let workflow_ir = specialist_composer::compose_supported_workflow(&request)
            .unwrap()
            .expect("Scenario 5 uses the registered specialist");
        compile_and_save_workflow(
            SaveWorkflowRequest {
                project_id: request.project_id,
                workflow: SavedWorkflowRecord {
                    id: workflow_ir.workflow_id.clone(),
                    name: workflow_ir.name.clone(),
                    steps: "{}".to_string(),
                    created_at: 1,
                    updated_at,
                },
                visual_state: json!({"prompt":"scenario five"}),
                workflow_ir,
                activate: true,
            },
            &request.capability_catalog,
            &persistence,
            &DeterministicTestCompiler,
            &identity,
        )
        .unwrap()
    };

    let first = save(scenario_five_request(Some(project.project_id.clone())), 10);
    assert_eq!(
        first.project_id.as_deref(),
        Some(project.project_id.as_str())
    );
    assert_eq!(first.review_capabilities.status, "ready");
    assert!(first.review_capabilities.project_file_read);
    assert!(first.review_capabilities.project_file_write);
    assert!(first.review_capabilities.official_web);

    let second = save(scenario_five_request(None), 20);
    assert_eq!(second.workflow_version, 2);
    assert_eq!(
        second.project_id.as_deref(),
        Some(project.project_id.as_str())
    );
    let listed = persistence.select_workflows().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].project_id.as_deref(),
        Some(project.project_id.as_str())
    );
    assert_eq!(listed[0].workflow_version, Some(2));
    assert_eq!(listed[0].compilation_status.as_deref(), Some("Compiled"));
    assert_eq!(
        listed[0].review_capabilities.as_ref().unwrap().status,
        "ready"
    );
    let saved_steps: Value = serde_json::from_str(&listed[0].steps).unwrap();
    assert_eq!(saved_steps["projectId"], project.project_id);
    assert_eq!(saved_steps["workflowVersion"], 2);
    assert_eq!(saved_steps["compilationStatus"], "Compiled");
    assert_eq!(saved_steps["reviewCapabilities"]["status"], "ready");
    assert_eq!(saved_steps["workflowIr"]["workflowVersion"], 2);
    let connection = persistence.open_connection().unwrap();
    let mut statement = connection
        .prepare("SELECT project_id FROM workflow_blueprints WHERE workflow_id=?1 ORDER BY version")
        .unwrap();
    let bindings = statement
        .query_map(rusqlite::params![first.workflow_id], |row| {
            row.get::<_, Option<String>>(0)
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(bindings, vec![Some(project.project_id.clone()); 2]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn compiler_guard_converts_worker_panic_to_structured_error() {
    let error =
        run_workflow_compiler_guard("unit_test", || -> Result<(), WorkflowCompilerError> {
            panic!("synthetic compiler panic")
        })
        .unwrap_err();

    assert_eq!(error.code, "workflow_compiler_worker_failed");
    assert!(error.message.contains("unit_test"));
    assert!(error.message.contains("synthetic compiler panic"));
}

#[test]
fn local_app_match_arms_reject_technical_false_positives() {
    let cases = [
        (
            test_mcp_capability("macos_applescript", "read_system_calendar"),
            "schedule asynchronous data packets",
        ),
        (
            test_mcp_capability("macos_applescript", "read_system_emails"),
            "inspect the ipc message reply path",
        ),
        (
            test_mcp_capability("macos_applescript", "read_system_reminders"),
            "inspect the cpu task queue",
        ),
    ];

    for (action, prompt) in cases {
        assert!(!action_matches_prompt(&action, prompt), "{prompt}");
    }
}

#[test]
fn accepts_exact_agent_instruction_contract() {
    let output = json!({
        "compilerVersion": "1.0.0",
        "instructions": [{
            "nodeId": "agent",
            "systemPrompt": "Draft a response from the supplied request and return JSON.",
            "inputVariableMappings": [{
                "name": "request",
                "template": "{{workflow.input}}"
            }],
            "evaluationProtocol": {
                "successCriteria": ["Output is valid JSON."],
                "failureAction": "fail",
                "maxRetries": 0
            }
        }]
    });

    let parsed = parse_compiler_output(&output.to_string(), &workflow_ir()).unwrap();
    assert_eq!(parsed.instructions.len(), 1);
}

#[test]
fn rejects_missing_agent_instruction() {
    let output = r#"{"compilerVersion":"1.0.0","instructions":[]}"#;
    let error = parse_compiler_output(output, &workflow_ir()).unwrap_err();
    assert_eq!(error.code, "workflow_compiler_contract_invalid");
}
