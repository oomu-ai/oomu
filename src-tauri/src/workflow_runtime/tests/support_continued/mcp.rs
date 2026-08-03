use super::*;

#[test]
fn mcp_error_with_empty_collection_evidence_remains_a_failure() {
    let (result, instance, model_calls, tool_calls, _) = execute_indexed_collection_fixture(
        json!({
            "content": [{"type": "text", "text": "mail access failed"}],
            "structuredContent": {"emails": []},
            "isError": true
        }),
        0,
    );
    let error = result.unwrap_err();

    assert!(error.message.contains("returned an error result"));
    assert_eq!(instance.status, ExecutionStatus::Failed);
    assert!(instance.output_payload.is_none());
    assert_eq!(model_calls, 0);
    assert_eq!(tool_calls, 1);
}

#[test]
fn mcp_text_writer_extracts_content_from_agent_output_envelope() {
    let _auto_approve_mcp = crate::tool_security::AutoApproveMcpTestGuard::enable();
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes.insert(
        2,
        WorkflowNode::McpTool(McpToolNode {
            id: "write-report".to_string(),
            label: "Write Report".to_string(),
            server_name: "taskflow_native".to_string(),
            tool_name: "write_markdown_report".to_string(),
            arguments: json!({
                "reportPath": "workspace/report.md",
                "content": "{{nodes.agent.output}}",
            }),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: Some(15_000),
        }),
    );
    if let Some(WorkflowNode::Output(output)) = compiled.workflow_ir.nodes.last_mut() {
        output.input_mapping = "{{nodes.write-report.output}}".to_string();
    }
    compiled.workflow_ir.edges = vec![
        WorkflowEdge {
            id: "e1".to_string(),
            source_node_id: "input".to_string(),
            source_port: "out".to_string(),
            target_node_id: "agent".to_string(),
            target_port: None,
        },
        WorkflowEdge {
            id: "e2".to_string(),
            source_node_id: "agent".to_string(),
            source_port: "out".to_string(),
            target_node_id: "write-report".to_string(),
            target_port: None,
        },
        WorkflowEdge {
            id: "e3".to_string(),
            source_node_id: "write-report".to_string(),
            source_port: "out".to_string(),
            target_node_id: "output".to_string(),
            target_port: None,
        },
    ];
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!({"message": "hello"}),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &StubExternalTools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();

    let content = &instance.memory["nodes.write-report.output"]["data"]["structuredContent"]
        ["arguments"]["content"];
    assert!(content.as_str().is_some_and(|value| {
        value.starts_with("handled:") && value.contains("\"message\":\"hello\"")
    }));
}

#[test]
fn native_calendar_and_notification_steps_do_not_require_applescript_readiness() {
    for tool_name in ["read_system_calendar", "trigger_system_notification"] {
        let node = McpToolNode {
            id: tool_name.to_string(),
            label: tool_name.to_string(),
            server_name: "macos_applescript".to_string(),
            tool_name: tool_name.to_string(),
            arguments: json!({}),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: Some(10_000),
        };
        let mut ready_servers = HashSet::new();
        ensure_mcp_server_ready_once(&node, &FailingPreflightTools, &mut ready_servers)
            .expect("native workflow steps must not depend on AppleScript startup");
        assert!(ready_servers.is_empty());
    }
}

#[test]
fn high_risk_mcp_tool_pauses_then_resumes_after_approval() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "agent".to_string(),
        label: "Write file".to_string(),
        server_name: "filesystem".to_string(),
        tool_name: "write_file".to_string(),
        arguments: json!({"path": "workspace/out.txt", "content": "{{workflow.input}}"}),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: None,
    });
    compiled.instructions.clear();
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("hello"),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    let paused = execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &StubExternalTools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();

    assert_eq!(instance.status, ExecutionStatus::AwaitingApproval);
    let approval = paused
        .approval_request
        .expect("approval request is emitted");
    assert_eq!(approval.node_id, "agent");
    assert_eq!(approval.context["actionType"], json!("mcp_tool"));
    assert_eq!(approval.context["toolName"], json!("write_file"));

    let completed = execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &StubExternalTools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        Some(ResumePermission {
            node_id: "agent".to_string(),
            decision: PermissionDecision::Approve,
        }),
    )
    .unwrap();

    assert_eq!(
        completed.execution_order,
        vec!["agent".to_string(), "output".to_string()]
    );
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert_eq!(
        instance.memory["nodes.agent.output"]["data"]["structuredContent"]["toolName"],
        json!("write_file")
    );
}

#[test]
fn remote_read_only_mcp_tool_still_requires_fresh_exact_approval() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "agent".to_string(),
        label: "Read report".to_string(),
        server_name: "filesystem".to_string(),
        tool_name: "read_file".to_string(),
        arguments: json!({"path": "workspace/report.txt"}),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: None,
    });
    compiled.instructions.clear();
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("hello"),
            },
        )]),
        outputs: HashMap::new(),
    };
    let executions = Arc::new(AtomicUsize::new(0));
    let tools = RemoteReviewTools {
        binding: remote_review_binding("filesystem", "read_file", "reports.example.com"),
        executions: executions.clone(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    let paused = execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &tools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();

    assert_eq!(instance.status, ExecutionStatus::AwaitingApproval);
    assert_eq!(executions.load(AtomicOrdering::SeqCst), 0);
    let approval = paused.approval_request.expect("remote read must pause");
    assert_eq!(approval.context["actionType"], json!("mcp_tool"));
    assert_eq!(
        approval.context["mcpApprovalBinding"]["canonicalOrigin"],
        json!("https://reports.example.com")
    );
    assert!(!routine_authority_can_satisfy_mcp_review(Some(
        &tools.binding
    )));

    let completed = execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &tools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        Some(ResumePermission {
            node_id: "agent".to_string(),
            decision: PermissionDecision::Approve,
        }),
    )
    .unwrap();
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert_eq!(executions.load(AtomicOrdering::SeqCst), 1);
    assert_eq!(completed.execution_order, vec!["agent", "output"]);
}

#[test]
fn persisted_mcp_approval_records_ledger_and_resumes() {
    let root = std::env::temp_dir().join(format!("oomu_mcp_approval_{}", unix_time_ms()));
    let persistence = PersistenceEngine::initialize_at(root.join("workflow.sqlite")).unwrap();
    let mut compiled = compiled_workflow(false);
    let tool_arguments = json!({
        "path": "workspace/out.txt",
        "content": "approved content"
    });
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "agent".to_string(),
        label: "Write file".to_string(),
        server_name: "filesystem".to_string(),
        tool_name: "write_file".to_string(),
        arguments: tool_arguments.clone(),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: None,
    });
    compiled.instructions.clear();
    let workflow = SavedWorkflowRecord {
        id: compiled.workflow_ir.workflow_id.clone(),
        name: compiled.workflow_ir.name.clone(),
        steps: r#"{"nodes":[]}"#.to_string(),
        created_at: 1,
        updated_at: 2,
    };
    let mut ir = compiled.workflow_ir.clone();
    persistence
        .reserve_workflow_blueprint(&workflow, &json!({"nodes": []}), &mut ir)
        .unwrap();
    persistence
        .publish_compiled_workflow(&workflow, &ir, &[], true)
        .unwrap();

    let paused = run_persisted_workflow(
        RunWorkflowRequest {
            workflow_id: workflow.id.clone(),
            workflow_version: Some(1),
            preflight_mode: WorkflowPreflightMode::default(),
            inputs: HashMap::from([(
                "input".to_string(),
                InputBinding::Manual {
                    value: json!("hello"),
                },
            )]),
            outputs: HashMap::new(),
        },
        &persistence,
        &StubModel,
        &StubExternalTools,
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();

    assert_eq!(paused.instance.status, ExecutionStatus::AwaitingApproval);
    let approval = paused.approval_request.expect("MCP approval is requested");
    assert_eq!(approval.node_id, "agent");
    let recovered = pending_workflow_approvals(&persistence).unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].instance_id, paused.instance.id);
    assert_eq!(recovered[0].approval_token, approval.approval_token);
    assert_eq!(recovered[0].message, approval.message);
    assert_eq!(recovered[0].context, approval.context);
    assert!(recovered[0].context.get("approvalToken").is_none());
    let approval_material = task_tools::workflow_mcp_approval_material(&tool_arguments, None);
    assert!(!persistence
        .verify_workflow_approval(
            &paused.instance.id,
            "agent",
            "write_file",
            &approval_material,
        )
        .unwrap());

    let completed = resolve_persisted_permission(
        ResolvePermissionRequest {
            instance_id: paused.instance.id.clone(),
            approval_token: approval.approval_token.clone(),
            decision: PermissionDecision::Approve,
        },
        &persistence,
        &StubModel,
        &StubExternalTools,
        &root.join("runs"),
        None,
    )
    .unwrap();

    assert_eq!(completed.instance.status, ExecutionStatus::Completed);
    assert_eq!(
        completed.instance.memory["nodes.agent.output"]["data"]["structuredContent"]["toolName"],
        json!("write_file")
    );
    assert!(persistence
        .verify_workflow_approval(
            &paused.instance.id,
            "agent",
            "write_file",
            &approval_material,
        )
        .unwrap());
    let workflow_version_material = task_tools::workflow_version_mcp_approval_material(
        match &compiled.workflow_ir.nodes[1] {
            WorkflowNode::McpTool(tool) => tool,
            _ => panic!("fixture node must remain an MCP tool"),
        },
        &tool_arguments,
        None,
    )
    .unwrap()
    .expect("generic MCP calls have an exact reusable scope");
    assert!(persistence
        .verify_workflow_version_approval(
            &workflow.id,
            1,
            "agent",
            "filesystem",
            "write_file",
            &workflow_version_material,
        )
        .unwrap());
    assert!(!persistence
        .verify_workflow_version_approval(
            &workflow.id,
            2,
            "agent",
            "filesystem",
            "write_file",
            &workflow_version_material,
        )
        .unwrap());

    let repeated = run_persisted_workflow(
        RunWorkflowRequest {
            workflow_id: workflow.id.clone(),
            workflow_version: Some(1),
            preflight_mode: WorkflowPreflightMode::default(),
            inputs: HashMap::from([(
                "input".to_string(),
                InputBinding::Manual {
                    value: json!("hello again"),
                },
            )]),
            outputs: HashMap::new(),
        },
        &persistence,
        &StubModel,
        &StubExternalTools,
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();
    assert_eq!(repeated.instance.status, ExecutionStatus::Completed);
    assert!(repeated.approval_request.is_none());
    assert!(pending_workflow_approvals(&persistence).unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unchanged_project_read_and_analysis_reviews_are_reused_on_the_second_run() {
    let _ = crate::tools::project_file::register_task_tool();
    let _ = crate::tools::supplier_exception::register_task_tool();
    let root = std::env::temp_dir().join(format!(
        "oomu_project_read_analysis_workflow_review_{}",
        unix_time_ms()
    ));
    let persistence = PersistenceEngine::initialize_at(root.join("workflow.sqlite")).unwrap();
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "read-suppliers".to_string(),
        label: "Read supplier evidence".to_string(),
        server_name: task_tools::SERVER_NAME.to_string(),
        tool_name: "read_project_file".to_string(),
        arguments: json!({
            "path": "/Project/mock_data/supplier_proposals.json",
            "maxBytes": 2_097_152
        }),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: None,
    });
    compiled.workflow_ir.nodes.insert(
        2,
        WorkflowNode::McpTool(McpToolNode {
            id: "analyze-suppliers".to_string(),
            label: "Analyze supplier evidence".to_string(),
            server_name: task_tools::SERVER_NAME.to_string(),
            tool_name: "analyze_supplier_exceptions".to_string(),
            arguments: json!({
                "content": serde_json::to_string(&json!({
                    "audit_year": 2026,
                    "quarter": "Q3",
                    "suppliers": [{
                        "name": "Apex",
                        "historical_settled_rate": 100.0,
                        "active_quote": 101.0,
                        "status": "active"
                    }]
                }))
                .unwrap()
            }),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: None,
        }),
    );
    compiled.workflow_ir.edges = vec![
        WorkflowEdge {
            id: "e1".to_string(),
            source_node_id: "input".to_string(),
            source_port: "out".to_string(),
            target_node_id: "read-suppliers".to_string(),
            target_port: None,
        },
        WorkflowEdge {
            id: "e2".to_string(),
            source_node_id: "read-suppliers".to_string(),
            source_port: "out".to_string(),
            target_node_id: "analyze-suppliers".to_string(),
            target_port: None,
        },
        WorkflowEdge {
            id: "e3".to_string(),
            source_node_id: "analyze-suppliers".to_string(),
            source_port: "out".to_string(),
            target_node_id: "output".to_string(),
            target_port: None,
        },
    ];
    if let Some(WorkflowNode::Output(output)) = compiled.workflow_ir.nodes.last_mut() {
        output.input_mapping = "{{nodes.analyze-suppliers.output}}".to_string();
    }
    compiled.instructions.clear();
    let workflow = SavedWorkflowRecord {
        id: compiled.workflow_ir.workflow_id.clone(),
        name: compiled.workflow_ir.name.clone(),
        steps: r#"{"nodes":[]}"#.to_string(),
        created_at: 1,
        updated_at: 2,
    };
    let mut ir = compiled.workflow_ir.clone();
    persistence
        .reserve_workflow_blueprint(&workflow, &json!({"nodes": []}), &mut ir)
        .unwrap();
    persistence
        .publish_compiled_workflow(&workflow, &ir, &[], true)
        .unwrap();
    let request = RunWorkflowRequest {
        workflow_id: workflow.id.clone(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("run"),
            },
        )]),
        outputs: HashMap::new(),
    };

    let first_pause = run_persisted_workflow(
        request.clone(),
        &persistence,
        &StubModel,
        &StubExternalTools,
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();
    let read_approval = first_pause
        .approval_request
        .expect("the first Project read must request one review");
    assert_eq!(read_approval.node_id, "read-suppliers");
    assert_eq!(
        read_approval.context.pointer("/approvalReuse/scope"),
        Some(&json!("workflow_version"))
    );

    let second_pause = resolve_persisted_permission(
        ResolvePermissionRequest {
            instance_id: first_pause.instance.id,
            approval_token: read_approval.approval_token,
            decision: PermissionDecision::Approve,
        },
        &persistence,
        &StubModel,
        &StubExternalTools,
        &root.join("runs"),
        None,
    )
    .unwrap();
    let analysis_approval = second_pause
        .approval_request
        .expect("the first deterministic analysis must request one review");
    assert_eq!(analysis_approval.node_id, "analyze-suppliers");
    assert_eq!(
        analysis_approval.context.pointer("/approvalReuse/scope"),
        Some(&json!("workflow_version"))
    );

    let completed = resolve_persisted_permission(
        ResolvePermissionRequest {
            instance_id: second_pause.instance.id,
            approval_token: analysis_approval.approval_token,
            decision: PermissionDecision::Approve,
        },
        &persistence,
        &StubModel,
        &StubExternalTools,
        &root.join("runs"),
        None,
    )
    .unwrap();
    assert_eq!(completed.instance.status, ExecutionStatus::Completed);

    let repeated = run_persisted_workflow(
        request,
        &persistence,
        &StubModel,
        &StubExternalTools,
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();
    assert_eq!(repeated.instance.status, ExecutionStatus::Completed);
    assert!(repeated.approval_request.is_none());
    assert!(pending_workflow_approvals(&persistence).unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn persisted_official_page_review_is_reused_for_the_unchanged_workflow_version() {
    let _ = crate::tools::official_page::register_task_tool();
    let root = std::env::temp_dir().join(format!(
        "oomu_official_page_workflow_review_{}",
        unix_time_ms()
    ));
    let persistence = PersistenceEngine::initialize_at(root.join("workflow.sqlite")).unwrap();
    let mut compiled = compiled_workflow(false);
    let arguments = json!({
        "url": "https://www.eia.gov/petroleum/gasdiesel/",
        "fallbackUrls": [],
        "maxContentChars": 3_000
    });
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "agent".to_string(),
        label: "Read the official fuel source".to_string(),
        server_name: task_tools::SERVER_NAME.to_string(),
        tool_name: "fetch_official_page".to_string(),
        arguments,
        input_schema: None,
        output_schema: None,
        system_timeout_ms: None,
    });
    compiled.instructions.clear();
    let workflow = SavedWorkflowRecord {
        id: compiled.workflow_ir.workflow_id.clone(),
        name: compiled.workflow_ir.name.clone(),
        steps: r#"{"nodes":[]}"#.to_string(),
        created_at: 1,
        updated_at: 2,
    };
    let mut ir = compiled.workflow_ir.clone();
    persistence
        .reserve_workflow_blueprint(&workflow, &json!({"nodes": []}), &mut ir)
        .unwrap();
    persistence
        .publish_compiled_workflow(&workflow, &ir, &[], true)
        .unwrap();
    let request = RunWorkflowRequest {
        workflow_id: workflow.id.clone(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("run"),
            },
        )]),
        outputs: HashMap::new(),
    };

    let paused = run_persisted_workflow(
        request.clone(),
        &persistence,
        &StubModel,
        &StubExternalTools,
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();
    assert_eq!(paused.instance.status, ExecutionStatus::AwaitingApproval);
    let approval = paused
        .approval_request
        .expect("the first official-page run must request review");
    assert_eq!(
        approval.context.pointer("/approvalReuse/scope"),
        Some(&json!("workflow_version"))
    );
    assert_eq!(
        approval.context.pointer("/approvalReuse/workflowVersion"),
        Some(&json!(1))
    );

    let completed = resolve_persisted_permission(
        ResolvePermissionRequest {
            instance_id: paused.instance.id,
            approval_token: approval.approval_token,
            decision: PermissionDecision::Approve,
        },
        &persistence,
        &StubModel,
        &StubExternalTools,
        &root.join("runs"),
        None,
    )
    .unwrap();
    assert_eq!(completed.instance.status, ExecutionStatus::Completed);

    let repeated = run_persisted_workflow(
        request,
        &persistence,
        &StubModel,
        &StubExternalTools,
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();
    assert_eq!(repeated.instance.status, ExecutionStatus::Completed);
    assert!(repeated.approval_request.is_none());
    assert!(pending_workflow_approvals(&persistence).unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn persisted_remote_review_survives_restart_but_rejects_changed_destination() {
    let root =
        std::env::temp_dir().join(format!("oomu_remote_mcp_review_restart_{}", unix_time_ms()));
    let persistence = PersistenceEngine::initialize_at(root.join("workflow.sqlite")).unwrap();
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "agent".to_string(),
        label: "Read report".to_string(),
        server_name: "filesystem".to_string(),
        tool_name: "read_file".to_string(),
        arguments: json!({"path": "workspace/report.txt"}),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: None,
    });
    compiled.instructions.clear();
    let workflow = SavedWorkflowRecord {
        id: compiled.workflow_ir.workflow_id.clone(),
        name: compiled.workflow_ir.name.clone(),
        steps: r#"{"nodes":[]}"#.to_string(),
        created_at: 1,
        updated_at: 2,
    };
    let mut ir = compiled.workflow_ir.clone();
    persistence
        .reserve_workflow_blueprint(&workflow, &json!({"nodes": []}), &mut ir)
        .unwrap();
    persistence
        .publish_compiled_workflow(&workflow, &ir, &[], true)
        .unwrap();
    let request = RunWorkflowRequest {
        workflow_id: workflow.id.clone(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("hello"),
            },
        )]),
        outputs: HashMap::new(),
    };
    let approved_binding = remote_review_binding("filesystem", "read_file", "reports.example.com");
    let before_restart = RemoteReviewTools {
        binding: approved_binding.clone(),
        executions: Arc::new(AtomicUsize::new(0)),
    };

    let paused = run_persisted_workflow(
        request.clone(),
        &persistence,
        &StubModel,
        &before_restart,
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();
    let approval = paused
        .approval_request
        .expect("remote call should wait for review");

    // A fresh runtime instance models an application restart. The stable
    // review binding remains comparable, while the MCP registry will still
    // mint fresh process-bound one-use authority at execution time.
    let restarted_executions = Arc::new(AtomicUsize::new(0));
    let after_restart = RemoteReviewTools {
        binding: approved_binding,
        executions: restarted_executions.clone(),
    };
    let completed = resolve_persisted_permission(
        ResolvePermissionRequest {
            instance_id: paused.instance.id,
            approval_token: approval.approval_token,
            decision: PermissionDecision::Approve,
        },
        &persistence,
        &StubModel,
        &after_restart,
        &root.join("runs"),
        None,
    )
    .unwrap();
    assert_eq!(completed.instance.status, ExecutionStatus::Completed);
    assert_eq!(restarted_executions.load(AtomicOrdering::SeqCst), 1);

    let repeated = run_persisted_workflow(
        request.clone(),
        &persistence,
        &StubModel,
        &before_restart,
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();
    assert_eq!(repeated.instance.status, ExecutionStatus::Completed);
    assert!(repeated.approval_request.is_none());
    assert_eq!(before_restart.executions.load(AtomicOrdering::SeqCst), 1);

    let changed_executions = Arc::new(AtomicUsize::new(0));
    let changed_runtime = RemoteReviewTools {
        binding: remote_review_binding("filesystem", "read_file", "other.example.com"),
        executions: changed_executions.clone(),
    };
    let changed = run_persisted_workflow(
        request,
        &persistence,
        &StubModel,
        &changed_runtime,
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();
    assert_eq!(changed.instance.status, ExecutionStatus::AwaitingApproval);
    assert!(changed.approval_request.is_some());
    assert_eq!(changed_executions.load(AtomicOrdering::SeqCst), 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sync_knowledge_vault_mcp_tool_executes_as_local_runtime_tool() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "agent".to_string(),
        label: "Sync Knowledge Vault".to_string(),
        server_name: "system".to_string(),
        tool_name: "sync_knowledge_vault".to_string(),
        arguments: json!({"path": "/tmp/vault", "maxFiles": 12}),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: Some(1_000),
    });
    compiled.instructions.clear();
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("run"),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    let outcome = execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &SyncExternalTools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();

    assert_eq!(
        outcome.execution_order,
        vec![
            "input".to_string(),
            "agent".to_string(),
            "output".to_string()
        ]
    );
    assert_eq!(instance.status, ExecutionStatus::Completed);
    let sync_output = &instance.memory["nodes.agent.output"];
    assert_eq!(sync_output["data"]["indexedFiles"], json!(2));
    assert_eq!(
        sync_output["data"]["arguments"]["path"],
        json!("/tmp/vault")
    );
    assert_eq!(sync_output["data"]["arguments"]["maxFiles"], json!(12));
    assert_eq!(sync_output["metadata"]["serverName"], json!("system"));
}

#[test]
fn mcp_readiness_failure_occurs_only_when_the_node_is_reached() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "agent".to_string(),
        label: "Read file".to_string(),
        server_name: "local_filesystem".to_string(),
        tool_name: "read_file".to_string(),
        arguments: json!({"path": "instruction_input.txt"}),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: Some(1_000),
    });
    compiled.instructions.clear();
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("hello"),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    let error = match execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &FailingPreflightTools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    ) {
        Ok(_) => panic!("preflight failure should halt the workflow"),
        Err(error) => error,
    };

    assert_eq!(error.code, "workflow_runtime_mcp_preflight_failed");
    assert_eq!(instance.status, ExecutionStatus::Failed);
    assert_eq!(
        instance.node_payloads["input"].status,
        ExecutionStatus::Completed
    );
    assert_eq!(
        instance.node_payloads["agent"].status,
        ExecutionStatus::Failed
    );
    assert!(instance.output_payload.is_none());
    assert!(!error.message.contains("before any nodes executed"));
}

#[test]
fn mcp_server_checked_before_approval_gate() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "agent".to_string(),
        label: "Run remote diagnostic".to_string(),
        server_name: "remote_shell".to_string(),
        tool_name: "execute_command".to_string(),
        arguments: json!({"command": "uptime"}),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: None,
    });
    compiled.instructions.clear();
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("ship it"),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();
    let tools = CountingReadinessTools::fail_on_check(1);

    let error = match execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &tools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    ) {
        Ok(_) => panic!("unreachable MCP server should fail before approval"),
        Err(error) => error,
    };

    assert_eq!(tools.checks.load(AtomicOrdering::SeqCst), 1);
    assert_eq!(tools.executions.load(AtomicOrdering::SeqCst), 0);
    assert_eq!(error.code, "workflow_runtime_mcp_preflight_failed");
    assert!(error
        .message
        .contains("MCP Server 'remote_shell' is offline or unreachable"));
    assert!(!error.message.contains("before any nodes executed"));
    assert_eq!(instance.status, ExecutionStatus::Failed);
    assert_eq!(
        instance.node_payloads["agent"].error.as_ref().unwrap()["message"],
        json!(error.message.clone())
    );
}

#[test]
fn mcp_server_checked_before_tool_execution() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "agent".to_string(),
        label: "Read file".to_string(),
        server_name: "local_filesystem".to_string(),
        tool_name: "read_file".to_string(),
        arguments: json!({"path": "instruction_input.txt"}),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: Some(1_000),
    });
    compiled.instructions.clear();
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("hello"),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();
    let tools = CountingReadinessTools::fail_on_check(1);

    let error = match execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &tools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    ) {
        Ok(_) => panic!("unreachable MCP server should fail before execution"),
        Err(error) => error,
    };

    assert_eq!(tools.checks.load(AtomicOrdering::SeqCst), 1);
    assert_eq!(tools.executions.load(AtomicOrdering::SeqCst), 0);
    assert_eq!(error.code, "workflow_runtime_mcp_preflight_failed");
    assert!(error
        .message
        .contains("MCP Server 'local_filesystem' is offline or unreachable"));
    assert_eq!(instance.status, ExecutionStatus::Failed);
}

#[test]
fn successful_mcp_readiness_is_cached_per_server_for_the_run() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "agent".to_string(),
        label: "First read".to_string(),
        server_name: "local_filesystem".to_string(),
        tool_name: "read_file".to_string(),
        arguments: json!({"path": "first.txt"}),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: Some(1_000),
    });
    compiled.workflow_ir.nodes.insert(
        2,
        WorkflowNode::McpTool(McpToolNode {
            id: "second".to_string(),
            label: "Second read".to_string(),
            server_name: "local_filesystem".to_string(),
            tool_name: "read_file".to_string(),
            arguments: json!({"path": "second.txt"}),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: Some(1_000),
        }),
    );
    compiled.workflow_ir.edges[1].target_node_id = "second".to_string();
    compiled.workflow_ir.edges.push(WorkflowEdge {
        id: "e3".to_string(),
        source_node_id: "second".to_string(),
        source_port: "out".to_string(),
        target_node_id: "output".to_string(),
        target_port: None,
    });
    if let Some(WorkflowNode::Output(output)) = compiled.workflow_ir.nodes.get_mut(3) {
        output.input_mapping = "{{nodes.second.output}}".to_string();
    }
    compiled.instructions.clear();
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("hello"),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();
    let tools = CountingReadinessTools {
        checks: Arc::new(AtomicUsize::new(0)),
        executions: Arc::new(AtomicUsize::new(0)),
        fail_on_check: None,
    };

    let outcome = execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &tools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();

    assert_eq!(
        outcome.execution_order,
        vec!["input", "agent", "second", "output"]
    );
    assert_eq!(tools.checks.load(AtomicOrdering::SeqCst), 1);
    assert_eq!(tools.executions.load(AtomicOrdering::SeqCst), 2);
    assert_eq!(instance.status, ExecutionStatus::Completed);
}

#[test]
fn system_exec_mcp_tool_always_pauses_for_gateway() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "agent".to_string(),
        label: "Run remote diagnostic".to_string(),
        server_name: "remote_shell".to_string(),
        tool_name: "execute_command".to_string(),
        arguments: json!({"command": "uptime"}),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: None,
    });
    compiled.instructions.clear();
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("ship it"),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    let paused = execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &StubExternalTools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();

    assert_eq!(instance.status, ExecutionStatus::AwaitingApproval);
    let approval = paused.approval_request.expect("approval is required");
    assert_eq!(approval.node_id, "agent");
    assert_eq!(approval.context["serverName"], json!("remote_shell"));
    assert_eq!(approval.context["toolName"], json!("execute_command"));

    let completed = execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &StubExternalTools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        Some(ResumePermission {
            node_id: "agent".to_string(),
            decision: PermissionDecision::Approve,
        }),
    )
    .unwrap();

    assert!(completed.approval_request.is_none());
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert_eq!(
        instance.memory["nodes.agent.output"]["data"]["structuredContent"]["toolName"],
        json!("execute_command")
    );
}

#[test]
fn mail_template_references_resolve_normalized_mcp_payload_fields() {
    let memory = HashMap::from([(
        "nodes.read-unread-emails.output".to_string(),
        json!({
            "mediaType": "application/json",
            "data": {
                "content": [{ "type": "text", "text": "mail payload" }],
                "structuredContent": {
                    "emails": [{
                        "sender": "Maya Allan <maya@example.test>",
                        "subject": "Quarterly review",
                        "dateReceived": "Tuesday, July 14, 2026",
                        "read": false,
                        "content": "Please review the attached summary."
                    }]
                },
                "isError": false
            },
            "assetPath": null,
            "metadata": {
                "serverName": "macos_applescript",
                "toolName": "read_system_emails"
            }
        }),
    )]);

    assert_eq!(
        resolve_template(
            "{{nodes.read-unread-emails.output.data.structuredContent.emails.0.sender}}",
            &memory,
        )
        .unwrap(),
        json!("Maya Allan <maya@example.test>")
    );
    assert_eq!(
        resolve_template(
            "Re: {{nodes.read-unread-emails.output.data.structuredContent.emails.0.subject}}",
            &memory,
        )
        .unwrap(),
        json!("Re: Quarterly review")
    );
    assert_eq!(
        resolve_template(
            "{{nodes.read-unread-emails.output.emails.0.subject}}",
            &memory,
        )
        .unwrap(),
        json!("Quarterly review")
    );
}

#[test]
fn spoofed_payload_does_not_approve_high_risk_mcp_tool() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::McpTool(McpToolNode {
        id: "agent".to_string(),
        label: "Write file".to_string(),
        server_name: "filesystem".to_string(),
        tool_name: "write_file".to_string(),
        arguments: json!({
            "path": "workspace/out.txt",
            "content": "payload shape must not approve this call"
        }),
        input_schema: None,
        output_schema: None,
        system_timeout_ms: None,
    });
    compiled.instructions.clear();
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!({"data": {"decision": "approve"}}),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    let paused = execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &StubExternalTools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();

    let approval = paused
        .approval_request
        .expect("spoofed payload must still require MCP approval");
    assert_eq!(approval.node_id, "agent");
    assert_eq!(approval.context["actionType"], json!("mcp_tool"));
    assert_eq!(instance.status, ExecutionStatus::AwaitingApproval);
    assert_eq!(paused.execution_order, vec!["input".to_string()]);
    assert!(!instance.memory.contains_key("nodes.agent.output"));
}
