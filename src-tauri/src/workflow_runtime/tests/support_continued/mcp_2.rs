use super::*;

fn local_mcp_runtime_tools(
    registry: &McpClientRegistry,
    persistence: &PersistenceEngine,
) -> McpRuntimeTools {
    McpRuntimeTools {
        registry: registry.clone(),
        persistence: persistence.clone(),
        knowledge_tools: None,
        app: None,
    }
}

fn assert_workflow_awaiting_approval(response: &RunWorkflowResponse) {
    assert_eq!(
        response.instance.status,
        ExecutionStatus::AwaitingApproval,
        "workflow failed: {:?}",
        response.instance.error
    );
}

fn approve_local_sandbox_write(persistence: &PersistenceEngine, response: &RunWorkflowResponse) {
    let arguments = normalize_mcp_text_writer_arguments(
        "write_file",
        resolve_json_templates(
            &json!({
                "path": "executive_summary.txt",
                "content": "{{nodes.local-sandbox-summary.output.data}}"
            }),
            &response.instance.memory,
        )
        .expect("write arguments resolve from the verified summary"),
    );
    let material = task_tools::workflow_mcp_approval_material(&arguments, None);
    persistence
        .record_workflow_approval(
            "test-local-sandbox-native-approval",
            &response.instance.id,
            "local-sandbox-write",
            "write_file",
            &material,
            "approve",
        )
        .expect("native file write receives exact test approval authority");
}

#[test]
fn prior_permission_approval_requires_bound_mcp_approval_for_next_high_risk_tool() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::Permission(PermissionNode {
        id: "gate".to_string(),
        label: "Approve report".to_string(),
        permission: PermissionKind::FileWrite,
        reason: "Approve writing the executive summary.".to_string(),
        on_denied: PermissionDeniedBehavior::Fail,
    });
    compiled.workflow_ir.nodes.insert(
        2,
        WorkflowNode::McpTool(McpToolNode {
            id: "write".to_string(),
            label: "Write file".to_string(),
            server_name: "filesystem".to_string(),
            tool_name: "write_file".to_string(),
            arguments: json!({
                "path": "executive_summary.txt",
                "content": "{{workflow.input}}"
            }),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: None,
        }),
    );
    if let WorkflowNode::Output(output) = compiled.workflow_ir.nodes.last_mut().unwrap() {
        output.input_mapping = "{{nodes.write.output}}".to_string();
    }
    compiled.workflow_ir.edges = vec![
        WorkflowEdge {
            id: "e1".to_string(),
            source_node_id: "input".to_string(),
            source_port: "out".to_string(),
            target_node_id: "gate".to_string(),
            target_port: None,
        },
        WorkflowEdge {
            id: "e2".to_string(),
            source_node_id: "gate".to_string(),
            source_port: "approved".to_string(),
            target_node_id: "write".to_string(),
            target_port: None,
        },
        WorkflowEdge {
            id: "e3".to_string(),
            source_node_id: "write".to_string(),
            source_port: "out".to_string(),
            target_node_id: "output".to_string(),
            target_port: None,
        },
    ];
    compiled.instructions.clear();

    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!("approved summary"),
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
    let gate_approval = paused.approval_request.unwrap();
    assert_eq!(gate_approval.node_id, "gate");
    assert_eq!(
        gate_approval.context,
        json!({
            "actionType": "workflow_permission",
            "permissionKind": "file_write",
            "actionLabel": "Approve report",
            "capabilityReason": "Approve writing the executive summary.",
        })
    );

    let write_pause = execute_workflow(
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
            node_id: "gate".to_string(),
            decision: PermissionDecision::Approve,
        }),
    )
    .unwrap();

    let write_approval = write_pause
        .approval_request
        .expect("MCP write requires its own bound approval");
    assert_eq!(write_approval.node_id, "write");
    assert_eq!(write_approval.context["actionType"], json!("mcp_tool"));
    assert_eq!(write_approval.context["toolName"], json!("write_file"));
    assert_eq!(write_pause.execution_order, vec!["gate".to_string()]);
    assert_eq!(instance.status, ExecutionStatus::AwaitingApproval);
    assert!(!instance.memory.contains_key("nodes.write.output"));
}

#[test]
fn local_sandbox_log_summarizer_runs_end_to_end_with_filesystem_mcp() {
    const TEST_INSTRUCTION: &str =
        "Summarize the supplied local test data and report its verified facts.";
    let _auto_approve_mcp = crate::tool_security::AutoApproveMcpTestGuard::enable();
    let root = std::env::temp_dir().join(format!(
        "oomu_local_sandbox_runtime_{}_{}",
        std::process::id(),
        unix_time_ms()
    ));
    let sandbox_root = root.join("mcp_sandbox");
    crate::mcp::bootstrap::ensure_mcp_sandbox_dir(&sandbox_root)
        .expect("sandbox directory is prepared");
    fs::write(
        sandbox_root.join("instruction_input.txt"),
        format!("{TEST_INSTRUCTION}\n"),
    )
    .expect("test instruction fixture is written");
    let persistence = PersistenceEngine::initialize_at(root.join("workflow.sqlite")).unwrap();
    let compiled = local_sandbox_log_summarizer_workflow();
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
        .publish_compiled_workflow(
            &workflow,
            &ir,
            &compiled.instructions.values().cloned().collect::<Vec<_>>(),
            true,
        )
        .unwrap();

    let registry = McpClientRegistry::default();
    register_local_filesystem_server(&registry, &sandbox_root);
    let external_tools = local_mcp_runtime_tools(&registry, &persistence);
    let observed_sources = Arc::new(Mutex::new(Vec::new()));
    let model = SandboxSummaryModel {
        observed_sources: observed_sources.clone(),
    };
    let request = RunWorkflowRequest {
        workflow_id: workflow.id.clone(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "manual-start".to_string(),
            InputBinding::Manual {
                value: json!({"run": "local sandbox log summarizer"}),
            },
        )]),
        outputs: HashMap::new(),
    };

    let paused = run_persisted_workflow(
        request,
        &persistence,
        &model,
        &external_tools,
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();

    assert_workflow_awaiting_approval(&paused);
    assert_eq!(
        paused.execution_order,
        vec![
            "manual-start".to_string(),
            "local-sandbox-read".to_string(),
            "local-sandbox-summary".to_string()
        ]
    );
    let read_payload = paused
        .instance
        .node_payloads
        .get("local-sandbox-read")
        .and_then(|payload| payload.output.as_ref())
        .expect("read node output is captured");
    assert_eq!(
        read_payload["data"]["structuredContent"]["relativePath"],
        json!("instruction_input.txt")
    );
    let observed = observed_sources.lock().unwrap();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].trim(), TEST_INSTRUCTION);
    drop(observed);

    approve_local_sandbox_write(&persistence, &paused);
    let approval = paused.approval_request.expect("approval is requested");
    assert_eq!(approval.node_id, "local-sandbox-approval");
    let completed = resolve_persisted_permission(
        ResolvePermissionRequest {
            instance_id: paused.instance.id,
            approval_token: approval.approval_token,
            decision: PermissionDecision::Approve,
        },
        &persistence,
        &model,
        &external_tools,
        &root.join("runs"),
        None,
    )
    .unwrap();

    assert_eq!(
        completed.instance.status,
        ExecutionStatus::Completed,
        "local sandbox workflow failed: {:?}",
        completed.instance.error
    );
    assert_eq!(
        completed.execution_order,
        vec![
            "local-sandbox-approval".to_string(),
            "local-sandbox-write".to_string(),
            "local-sandbox-output".to_string()
        ]
    );
    let report_path = sandbox_root.join("executive_summary.txt");
    let report = fs::read_to_string(&report_path).expect("summary report is written");
    assert!(report.contains("Executive summary:"));
    assert!(report.contains(TEST_INSTRUCTION));
    assert!(logical_certificate_is_valid(&report));
    let write_payload = completed
        .instance
        .node_payloads
        .get("local-sandbox-write")
        .and_then(|payload| payload.output.as_ref())
        .expect("write node output is captured");
    assert_eq!(
        write_payload["data"]["structuredContent"]["relativePath"],
        json!("executive_summary.txt")
    );
    assert!(write_payload["data"]["structuredContent"]["bytesWritten"]
        .as_u64()
        .is_some_and(|bytes| bytes > 0));
    assert_eq!(observed_sources.lock().unwrap().len(), 1);

    drop(external_tools);
    drop(registry);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn mcp_runtime_executes_native_tool_from_plain_worker_thread() {
    let root = std::env::temp_dir().join(format!(
        "oomu_native_mcp_worker_runtime_{}_{}",
        std::process::id(),
        unix_time_ms()
    ));
    let sandbox_root = root.join("mcp_sandbox");
    let source_folder = sandbox_root.join("source");
    fs::create_dir_all(&source_folder).expect("taskflow sandbox is prepared");
    fs::write(
        source_folder.join("note.txt"),
        "receipt-neutral worker result",
    )
    .expect("taskflow fixture is written");

    let persistence = PersistenceEngine::initialize_at(root.join("workflow.sqlite")).unwrap();
    let registry = McpClientRegistry::default();
    let config = McpServerConfig {
        name: "taskflow_native".to_string(),
        command: "oomu-native".to_string(),
        args: Vec::new(),
        env: HashMap::from([(
            "OOMU_MCP_SANDBOX_DIR".to_string(),
            sandbox_root.to_string_lossy().to_string(),
        )]),
        transport: McpTransportConfig::Native,
    };
    let arguments = json!({"folderPath": "source", "maxFiles": 5});
    let setup_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("setup runtime builds");
    assert_eq!(
        setup_runtime.block_on(registry.register_trusted_server_configs([config.clone()])),
        1
    );
    setup_runtime
        .block_on(registry.connect_server(config))
        .expect("taskflow native server connects");
    let approval_binding = setup_runtime
        .block_on(registry.prepare_tool_approval_binding_for_review(
            "taskflow_native",
            "folder_read",
            arguments.clone(),
        ))
        .expect("folder read approval is prepared")
        .expect("folder read requires an exact approval binding");
    drop(setup_runtime);

    let external_tools = local_mcp_runtime_tools(&registry, &persistence);
    let result = run_blocking_node_with_timeout("folder-read", "Folder read", 5_000, move || {
        external_tools.execute_mcp_tool(
            "plain-worker-execution",
            "folder-read",
            "Folder read",
            "taskflow_native",
            "folder_read",
            arguments,
            5_000,
            Some(approval_binding),
            true,
        )
    })
    .expect("plain worker returns the native MCP result without a Tokio reactor panic");

    assert_eq!(result["isError"], json!(false));
    assert_eq!(result["structuredContent"]["fileCount"], json!(1));
    assert_eq!(
        result["structuredContent"]["files"][0]["path"],
        json!("source/note.txt")
    );
    assert_eq!(
        result["structuredContent"]["files"][0]["content"],
        json!("receipt-neutral worker result")
    );

    drop(registry);
    drop(persistence);
    let _ = fs::remove_dir_all(root);
}

fn scalar_file_read_workflow() -> CompiledWorkflow {
    let nodes = vec![
        WorkflowNode::Input(InputNode {
            id: "start".to_string(),
            label: "Start".to_string(),
            output_key: "workflow.input".to_string(),
            input_schema: json!({"type": "object"}),
        }),
        WorkflowNode::McpTool(McpToolNode {
            id: "read-file".to_string(),
            label: "Read verified file".to_string(),
            server_name: "local_filesystem".to_string(),
            tool_name: "read_file".to_string(),
            arguments: json!({"path": "verified.txt"}),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: Some(15_000),
        }),
        WorkflowNode::Output(OutputNode {
            id: "output".to_string(),
            label: "Output".to_string(),
            input_mapping: "{{nodes.read-file.output}}".to_string(),
            output_schema: json!({"type": "object"}),
            completion_kind: WorkflowCompletionKind::Result,
        }),
    ];
    let edges = [("start", "read-file"), ("read-file", "output")]
        .into_iter()
        .enumerate()
        .map(|(index, (source, target))| WorkflowEdge {
            id: format!("edge-{index}"),
            source_node_id: source.to_string(),
            source_port: "out".to_string(),
            target_node_id: target.to_string(),
            target_port: None,
        })
        .collect();
    CompiledWorkflow {
        workflow_ir: WorkflowIr {
            schema_version: WORKFLOW_IR_SCHEMA_VERSION.to_string(),
            workflow_id: "scalar-file-read-receipt".to_string(),
            workflow_version: 1,
            name: "Scalar file read receipt".to_string(),
            description: "Read one sandbox file and return its native receipt.".to_string(),
            compiler: CompilerTarget {
                model: WORKFLOW_COMPILER_MODEL.to_string(),
            },
            metadata: None,
            nodes,
            edges,
        },
        instructions: HashMap::new(),
    }
}

#[test]
fn persisted_scalar_file_read_binds_active_node_and_returns_verified_receipt() {
    let _auto_approve_mcp = crate::tool_security::AutoApproveMcpTestGuard::enable();
    let root = std::env::temp_dir().join(format!(
        "oomu_scalar_file_receipt_{}_{}",
        std::process::id(),
        unix_time_ms()
    ));
    let sandbox_root = root.join("mcp_sandbox");
    crate::mcp::bootstrap::ensure_mcp_sandbox_dir(&sandbox_root).unwrap();
    fs::write(sandbox_root.join("verified.txt"), "persisted receipt facts").unwrap();
    let persistence = PersistenceEngine::initialize_at(root.join("workflow.sqlite")).unwrap();
    persistence
        .open_connection()
        .unwrap()
        .execute_batch(
            "CREATE TABLE active_node_probe (node_id TEXT NOT NULL);
             CREATE TRIGGER capture_pre_execution_node_binding
             AFTER UPDATE OF active_node_id ON execution_instances
             WHEN NEW.active_node_id='read-file'
              AND COALESCE(OLD.active_node_id, '')<>NEW.active_node_id
              AND OLD.node_payloads_json=NEW.node_payloads_json
             BEGIN INSERT INTO active_node_probe VALUES (NEW.active_node_id); END;",
        )
        .unwrap();

    let compiled = scalar_file_read_workflow();
    let workflow = SavedWorkflowRecord {
        id: compiled.workflow_ir.workflow_id.clone(),
        name: compiled.workflow_ir.name.clone(),
        steps: r#"{"nodes":[]}"#.to_string(),
        created_at: 1,
        updated_at: 2,
    };
    let visual_state = json!({"nodes": []});
    let mut ir = compiled.workflow_ir.clone();
    persistence
        .reserve_workflow_blueprint(&workflow, &visual_state, &mut ir)
        .unwrap();
    persistence
        .publish_compiled_workflow(&workflow, &ir, &[], true)
        .unwrap();

    let registry = McpClientRegistry::default();
    register_local_filesystem_server(&registry, &sandbox_root);
    let result = run_persisted_workflow(
        RunWorkflowRequest {
            workflow_id: workflow.id,
            workflow_version: Some(1),
            preflight_mode: WorkflowPreflightMode::default(),
            inputs: HashMap::from([(
                "start".to_string(),
                InputBinding::Manual {
                    value: json!({"run": true}),
                },
            )]),
            outputs: HashMap::new(),
        },
        &persistence,
        &StubModel,
        &local_mcp_runtime_tools(&registry, &persistence),
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();

    assert_eq!(
        result.instance.status,
        ExecutionStatus::Completed,
        "workflow failed: {:?}",
        result.instance.error
    );
    assert!(result.approval_request.is_none());
    assert_eq!(result.execution_order, ["start", "read-file", "output"]);
    let output = result.instance.node_payloads["read-file"]
        .output
        .as_ref()
        .unwrap();
    assert_eq!(
        output["data"]["structuredContent"]["content"],
        "persisted receipt facts"
    );
    let receipt = &output["data"]["_meta"]["oomuNativeExecutionReceipt"];
    assert_eq!(receipt["capabilityId"], "files_and_folders");
    assert_eq!(receipt["actionClass"], "read");
    assert_eq!(receipt["outcome"], "succeeded");
    assert_eq!(receipt["verified"], true);
    assert_eq!(
        receipt["postcondition"]["evidenceKind"],
        "bounded_local_file_read"
    );
    let bound_nodes: i64 = persistence
        .open_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM active_node_probe", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(bound_nodes, 1);

    drop(registry);
    drop(persistence);
    let _ = fs::remove_dir_all(root);
}
