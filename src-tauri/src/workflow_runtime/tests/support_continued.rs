use super::*;

fn nested_contact_collection_workflow() -> CompiledWorkflow {
    let mut compiled = indexed_collection_workflow(0);
    if let Some(WorkflowNode::McpTool(read)) = compiled.workflow_ir.nodes.get_mut(1) {
        read.output_schema = Some(json!({
            "type": "object",
            "x-oomu-result-contract": {
                "kind": "collection",
                "path": "/structuredContent/contacts",
                "emptyIsSuccess": true
            }
        }));
    }
    if let Some(WorkflowNode::McpTool(downstream)) = compiled.workflow_ir.nodes.get_mut(3) {
        downstream.arguments = json!({
            "path": "{{nodes.read.output.data.structuredContent.contacts.0.emails.0.address}}"
        });
    }
    compiled
}

fn whole_collection_workflow() -> CompiledWorkflow {
    let mut compiled = indexed_collection_workflow(0);
    let collection_mapping = "{{nodes.read.output.data.structuredContent.emails}}".to_string();
    if let Some(WorkflowNode::Agent(consumer)) = compiled.workflow_ir.nodes.get_mut(2) {
        consumer.input_mappings =
            HashMap::from([("context".to_string(), collection_mapping.clone())]);
    }
    if let Some(instruction) = compiled.instructions.get_mut("consumer") {
        instruction.input_variable_mappings =
            HashMap::from([("context".to_string(), collection_mapping)]);
    }
    compiled
}

fn whole_envelope_collection_workflow() -> CompiledWorkflow {
    let mut compiled = indexed_collection_workflow(0);
    if let Some(WorkflowNode::McpTool(downstream)) = compiled.workflow_ir.nodes.get_mut(3) {
        downstream.arguments = json!({
            "path": "{{nodes.consumer.output.data}}"
        });
    }
    compiled
}

fn writer_warning_envelope_workflow() -> CompiledWorkflow {
    let mut compiled = whole_envelope_collection_workflow();
    if let Some(WorkflowNode::McpTool(producer)) = compiled.workflow_ir.nodes.get_mut(1) {
        producer.label = "Write report".to_string();
        producer.tool_name = "write_file".to_string();
        producer.arguments = json!({
            "path": "report.md",
            "content": "complete"
        });
    }
    if let Some(WorkflowNode::McpTool(downstream)) = compiled.workflow_ir.nodes.get_mut(3) {
        downstream.tool_name = "write_file".to_string();
        downstream.arguments = json!({
            "path": "downstream.md",
            "content": "{{nodes.consumer.output}}"
        });
    }
    compiled
}

fn custom_server_read_spoof_workflow() -> CompiledWorkflow {
    let mut compiled = whole_envelope_collection_workflow();
    if let Some(WorkflowNode::McpTool(producer)) = compiled.workflow_ir.nodes.get_mut(1) {
        producer.server_name = "custom_server".to_string();
        producer.tool_name = "read_file".to_string();
    }
    if let Some(WorkflowNode::McpTool(downstream)) = compiled.workflow_ir.nodes.get_mut(3) {
        downstream.tool_name = "write_file".to_string();
        downstream.arguments = json!({
            "path": "downstream.md",
            "content": "{{nodes.consumer.output}}"
        });
    }
    compiled
}

fn selective_readiness_workflow() -> CompiledWorkflow {
    let mut compiled = whole_collection_workflow();
    if let Some(WorkflowNode::McpTool(read)) = compiled.workflow_ir.nodes.get_mut(1) {
        read.server_name = "local_filesystem".to_string();
    }
    if let Some(WorkflowNode::McpTool(downstream)) = compiled.workflow_ir.nodes.get_mut(3) {
        downstream.server_name = "offline_writer".to_string();
    }
    compiled
}

fn mixed_migrated_and_legacy_collection_workflow() -> CompiledWorkflow {
    let mut compiled = whole_collection_workflow();
    compiled.workflow_ir.nodes.extend([
        WorkflowNode::McpTool(McpToolNode {
            id: "migrated-read".to_string(),
            label: "Read migrated collection".to_string(),
            server_name: "local_filesystem".to_string(),
            tool_name: "read_file".to_string(),
            arguments: json!({"path": "migrated.json"}),
            input_schema: None,
            output_schema: Some(json!({
                "type": "object",
                "x-oomu-result-contract": {
                    "kind": "collection",
                    "path": "/structuredContent/items",
                    "emptyIsSuccess": true
                }
            })),
            system_timeout_ms: Some(1_000),
        }),
        WorkflowNode::Conditional(ConditionalNode {
            id: "migrated-condition".to_string(),
            label: "Migrated collection has items".to_string(),
            condition: "Continue through the migrated result path".to_string(),
            input_mapping: Some("true".to_string()),
            system_timeout_ms: None,
        }),
        WorkflowNode::Output(OutputNode {
            id: "migrated-result".to_string(),
            label: "Migrated result".to_string(),
            input_mapping: "{{workflow.input}}".to_string(),
            output_schema: json!({"type": "object"}),
            completion_kind: WorkflowCompletionKind::Result,
        }),
        WorkflowNode::Output(OutputNode {
            id: "migrated-empty".to_string(),
            label: "Migrated empty".to_string(),
            input_mapping: "{{nodes.migrated-read.output.data.structuredContent.items}}"
                .to_string(),
            output_schema: json!({"type": "array"}),
            completion_kind: WorkflowCompletionKind::EmptyCollection,
        }),
    ]);
    compiled.workflow_ir.edges.extend([
        WorkflowEdge {
            id: "m1".to_string(),
            source_node_id: "input".to_string(),
            source_port: "out".to_string(),
            target_node_id: "migrated-read".to_string(),
            target_port: None,
        },
        WorkflowEdge {
            id: "m2".to_string(),
            source_node_id: "migrated-read".to_string(),
            source_port: "out".to_string(),
            target_node_id: "migrated-condition".to_string(),
            target_port: None,
        },
        WorkflowEdge {
            id: "m3".to_string(),
            source_node_id: "migrated-condition".to_string(),
            source_port: "true".to_string(),
            target_node_id: "migrated-result".to_string(),
            target_port: None,
        },
        WorkflowEdge {
            id: "m4".to_string(),
            source_node_id: "migrated-condition".to_string(),
            source_port: "false".to_string(),
            target_node_id: "migrated-empty".to_string(),
            target_port: None,
        },
    ]);
    compiled
}

fn two_source_legacy_collection_workflow() -> CompiledWorkflow {
    let nodes = vec![
        WorkflowNode::Input(InputNode {
            id: "input".to_string(),
            label: "Input".to_string(),
            output_key: "workflow.input".to_string(),
            input_schema: json!({"type": "object"}),
        }),
        WorkflowNode::McpTool(McpToolNode {
            id: "mail-read".to_string(),
            label: "Read mail".to_string(),
            server_name: "local_filesystem".to_string(),
            tool_name: "read_file".to_string(),
            arguments: json!({"path": "mail.json"}),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: Some(1_000),
        }),
        WorkflowNode::McpTool(McpToolNode {
            id: "reminders-read".to_string(),
            label: "Read reminders".to_string(),
            server_name: "local_filesystem".to_string(),
            tool_name: "read_file".to_string(),
            arguments: json!({"path": "reminders.json"}),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: Some(1_000),
        }),
        WorkflowNode::Agent(AgentNode {
            id: "consumer".to_string(),
            label: "Build daily brief".to_string(),
            objective: "Build the brief from mail and reminders.".to_string(),
            input_mappings: HashMap::from([
                ("mail".to_string(), "{{nodes.mail-read.output}}".to_string()),
                (
                    "reminders".to_string(),
                    "{{nodes.reminders-read.output}}".to_string(),
                ),
            ]),
            output_key: "nodes.consumer.output".to_string(),
            system_timeout_ms: None,
        }),
        WorkflowNode::McpTool(McpToolNode {
            id: "downstream-tool".to_string(),
            label: "Write brief".to_string(),
            server_name: "local_filesystem".to_string(),
            tool_name: "read_file".to_string(),
            arguments: json!({"path": "{{nodes.consumer.output.data}}"}),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: Some(1_000),
        }),
        WorkflowNode::Output(OutputNode {
            id: "output".to_string(),
            label: "Output".to_string(),
            input_mapping: "{{nodes.downstream-tool.output}}".to_string(),
            output_schema: json!({"type": "object"}),
            completion_kind: WorkflowCompletionKind::Result,
        }),
    ];
    let edges = [
        ("t1", "input", "mail-read"),
        ("t2", "mail-read", "reminders-read"),
        ("t3", "reminders-read", "consumer"),
        ("t4", "consumer", "downstream-tool"),
        ("t5", "downstream-tool", "output"),
    ]
    .into_iter()
    .map(|(id, source, target)| WorkflowEdge {
        id: id.to_string(),
        source_node_id: source.to_string(),
        source_port: "out".to_string(),
        target_node_id: target.to_string(),
        target_port: None,
    })
    .collect();
    CompiledWorkflow {
        workflow_ir: WorkflowIr {
            schema_version: WORKFLOW_IR_SCHEMA_VERSION.to_string(),
            workflow_id: "two-source-legacy-collection-workflow".to_string(),
            workflow_version: 1,
            name: "Two source legacy collection workflow".to_string(),
            description: String::new(),
            compiler: CompilerTarget {
                model: WORKFLOW_COMPILER_MODEL.to_string(),
            },
            metadata: None,
            nodes,
            edges,
        },
        instructions: HashMap::from([(
            "consumer".to_string(),
            CompiledInstruction {
                id: "instruction-consumer".to_string(),
                workflow_id: "two-source-legacy-collection-workflow".to_string(),
                workflow_version: 1,
                node_id: "consumer".to_string(),
                node_kind: WorkflowNodeKind::Agent,
                system_prompt: "Build the daily brief.".to_string(),
                input_variable_mappings: HashMap::from([
                    ("mail".to_string(), "{{nodes.mail-read.output}}".to_string()),
                    (
                        "reminders".to_string(),
                        "{{nodes.reminders-read.output}}".to_string(),
                    ),
                ]),
                evaluation_protocol: json!({
                    "successCriteria": ["exists"],
                    "failureAction": "fail",
                    "maxRetries": 0
                }),
                compiler_model: WORKFLOW_COMPILER_MODEL.to_string(),
                compiler_version: "1.0.0".to_string(),
                created_at_ms: 1,
            },
        )]),
    }
}

fn unrelated_prefix_collection_workflow() -> CompiledWorkflow {
    let mut compiled = two_source_legacy_collection_workflow();
    let reminder_mapping = HashMap::from([(
        "reminders".to_string(),
        "{{nodes.reminders-read.output}}".to_string(),
    )]);
    if let Some(WorkflowNode::Agent(consumer)) = compiled.workflow_ir.nodes.get_mut(3) {
        consumer.input_mappings = reminder_mapping.clone();
    }
    if let Some(instruction) = compiled.instructions.get_mut("consumer") {
        instruction.input_variable_mappings = reminder_mapping;
    }
    compiled
}

fn execute_indexed_collection_fixture(
    mcp_result: Value,
    index: usize,
) -> (
    Result<ExecutionOutcome, WorkflowRuntimeError>,
    ExecutionInstance,
    usize,
    usize,
    Vec<ExecutionInstance>,
) {
    execute_collection_workflow_fixture(indexed_collection_workflow(index), mcp_result)
}

fn execute_collection_workflow_fixture(
    compiled: CompiledWorkflow,
    mcp_result: Value,
) -> (
    Result<ExecutionOutcome, WorkflowRuntimeError>,
    ExecutionInstance,
    usize,
    usize,
    Vec<ExecutionInstance>,
) {
    execute_collection_workflow_fixture_with_delay(compiled, mcp_result, 0)
}

fn execute_collection_workflow_fixture_with_delay(
    compiled: CompiledWorkflow,
    mcp_result: Value,
    delay_ms: u64,
) -> (
    Result<ExecutionOutcome, WorkflowRuntimeError>,
    ExecutionInstance,
    usize,
    usize,
    Vec<ExecutionInstance>,
) {
    let request = RunWorkflowRequest {
        workflow_id: compiled.workflow_ir.workflow_id.clone(),
        workflow_version: Some(compiled.workflow_ir.workflow_version),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual { value: json!({}) },
        )]),
        outputs: HashMap::new(),
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let model = CountingCollectionModel {
        calls: calls.clone(),
    };
    let tool_executions = Arc::new(AtomicUsize::new(0));
    let tools = FixedMcpResultTools {
        result: mcp_result,
        executions: tool_executions.clone(),
        delay_ms,
    };
    let checkpoints = Arc::new(Mutex::new(Vec::<ExecutionInstance>::new()));
    let checkpoint_log = checkpoints.clone();
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();
    let result = execute_workflow(
        &compiled,
        &request,
        &model,
        &tools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |current| {
            checkpoint_log.lock().unwrap().push(current.clone());
            Ok(())
        },
        &mut |_, _, _, _, _| {},
        None,
        None,
    );
    let model_calls = calls.load(AtomicOrdering::SeqCst);
    let tool_executions = tool_executions.load(AtomicOrdering::SeqCst);
    let checkpoints = checkpoints.lock().unwrap().clone();
    (result, instance, model_calls, tool_executions, checkpoints)
}

fn execute_selective_readiness_fixture(
    source_result: Value,
) -> (
    Result<ExecutionOutcome, WorkflowRuntimeError>,
    ExecutionInstance,
    usize,
    Vec<String>,
    Vec<String>,
) {
    let compiled = selective_readiness_workflow();
    let request = RunWorkflowRequest {
        workflow_id: compiled.workflow_ir.workflow_id.clone(),
        workflow_version: Some(compiled.workflow_ir.workflow_version),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual { value: json!({}) },
        )]),
        outputs: HashMap::new(),
    };
    let model_calls = Arc::new(AtomicUsize::new(0));
    let model = CountingCollectionModel {
        calls: model_calls.clone(),
    };
    let readiness_checks = Arc::new(Mutex::new(Vec::new()));
    let executions = Arc::new(Mutex::new(Vec::new()));
    let tools = SelectiveReadinessTools {
        source_result,
        offline_server: "offline_writer".to_string(),
        readiness_checks: readiness_checks.clone(),
        executions: executions.clone(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();
    let result = execute_workflow(
        &compiled,
        &request,
        &model,
        &tools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    );
    let model_calls = model_calls.load(AtomicOrdering::SeqCst);
    let readiness_checks = readiness_checks.lock().unwrap().clone();
    let executions = executions.lock().unwrap().clone();
    (result, instance, model_calls, readiness_checks, executions)
}

fn conditional_workflow() -> CompiledWorkflow {
    let nodes = vec![
        WorkflowNode::Input(InputNode {
            id: "input".to_string(),
            label: "Input".to_string(),
            output_key: "workflow.input".to_string(),
            input_schema: json!({"type": "object"}),
        }),
        WorkflowNode::Conditional(ConditionalNode {
            id: "condition".to_string(),
            label: "Compile succeeded?".to_string(),
            condition: "Did the compilation succeed?".to_string(),
            input_mapping: Some("{{workflow.input.data.status}}".to_string()),
            system_timeout_ms: None,
        }),
        WorkflowNode::Agent(AgentNode {
            id: "then_agent".to_string(),
            label: "Then".to_string(),
            objective: "Handle success.".to_string(),
            input_mappings: HashMap::from([(
                "context".to_string(),
                "{{workflow.input}}".to_string(),
            )]),
            output_key: "nodes.then_agent.output".to_string(),
            system_timeout_ms: None,
        }),
        WorkflowNode::Agent(AgentNode {
            id: "else_agent".to_string(),
            label: "Else".to_string(),
            objective: "Handle failure.".to_string(),
            input_mappings: HashMap::from([(
                "context".to_string(),
                "{{workflow.input}}".to_string(),
            )]),
            output_key: "nodes.else_agent.output".to_string(),
            system_timeout_ms: None,
        }),
        WorkflowNode::Output(OutputNode {
            id: "output".to_string(),
            label: "Output".to_string(),
            input_mapping: "{{workflow.output}}".to_string(),
            output_schema: json!({"type": "object"}),
            completion_kind: WorkflowCompletionKind::Result,
        }),
    ];
    let edges = vec![
        ("e1", "input", "out", "condition"),
        ("e2", "condition", "true", "then_agent"),
        ("e3", "condition", "false", "else_agent"),
        ("e4", "then_agent", "out", "output"),
        ("e5", "else_agent", "out", "output"),
    ]
    .into_iter()
    .map(|(id, source, port, target)| WorkflowEdge {
        id: id.to_string(),
        source_node_id: source.to_string(),
        source_port: port.to_string(),
        target_node_id: target.to_string(),
        target_port: None,
    })
    .collect();
    CompiledWorkflow {
        workflow_ir: WorkflowIr {
            schema_version: "1.0.0".to_string(),
            workflow_id: "conditional-workflow".to_string(),
            workflow_version: 1,
            name: "Conditional Workflow".to_string(),
            description: String::new(),
            compiler: CompilerTarget {
                model: "gemma-4-e2b-qat".to_string(),
            },
            metadata: None,
            nodes,
            edges,
        },
        instructions: HashMap::from([
            (
                "then_agent".to_string(),
                instruction("conditional-workflow", "then_agent", "{{workflow.input}}"),
            ),
            (
                "else_agent".to_string(),
                instruction("conditional-workflow", "else_agent", "{{workflow.input}}"),
            ),
        ]),
    }
}

fn loop_workflow() -> CompiledWorkflow {
    let nodes = vec![
        WorkflowNode::Input(InputNode {
            id: "input".to_string(),
            label: "Input".to_string(),
            output_key: "workflow.input".to_string(),
            input_schema: json!({"type": "object"}),
        }),
        WorkflowNode::Loop(LoopNode {
            id: "loop".to_string(),
            label: "For Each".to_string(),
            items_mapping: "{{workflow.input.data.files}}".to_string(),
            item_variable: "item".to_string(),
            system_timeout_ms: None,
        }),
        WorkflowNode::Agent(AgentNode {
            id: "summarize".to_string(),
            label: "Summarize".to_string(),
            objective: "Summarize one item.".to_string(),
            input_mappings: HashMap::from([("context".to_string(), "{{item}}".to_string())]),
            output_key: "nodes.summarize.output".to_string(),
            system_timeout_ms: None,
        }),
        WorkflowNode::Output(OutputNode {
            id: "output".to_string(),
            label: "Output".to_string(),
            input_mapping: "{{nodes.summarize.output}}".to_string(),
            output_schema: json!({"type": "object"}),
            completion_kind: WorkflowCompletionKind::Result,
        }),
    ];
    let edges = vec![
        ("e1", "input", "out", "loop"),
        ("e2", "loop", "item", "summarize"),
        ("e3", "loop", "done", "output"),
        ("e4", "summarize", "out", "output"),
    ]
    .into_iter()
    .map(|(id, source, port, target)| WorkflowEdge {
        id: id.to_string(),
        source_node_id: source.to_string(),
        source_port: port.to_string(),
        target_node_id: target.to_string(),
        target_port: None,
    })
    .collect();
    CompiledWorkflow {
        workflow_ir: WorkflowIr {
            schema_version: "1.0.0".to_string(),
            workflow_id: "loop-workflow".to_string(),
            workflow_version: 1,
            name: "Loop Workflow".to_string(),
            description: String::new(),
            compiler: CompilerTarget {
                model: "gemma-4-e2b-qat".to_string(),
            },
            metadata: None,
            nodes,
            edges,
        },
        instructions: HashMap::from([(
            "summarize".to_string(),
            instruction("loop-workflow", "summarize", "{{item}}"),
        )]),
    }
}

fn sprint_61_parallel_workflow() -> CompiledWorkflow {
    let nodes = vec![
        WorkflowNode::Input(InputNode {
            id: "input".to_string(),
            label: "Input".to_string(),
            output_key: "workflow.input".to_string(),
            input_schema: json!({"type": "object"}),
        }),
        WorkflowNode::Agent(AgentNode {
            id: "agent_a".to_string(),
            label: "Parallel A".to_string(),
            objective: "Handle the left branch.".to_string(),
            input_mappings: HashMap::from([(
                "context".to_string(),
                "{{workflow.input.data.left}}".to_string(),
            )]),
            output_key: "nodes.agent_a.output".to_string(),
            system_timeout_ms: None,
        }),
        WorkflowNode::Agent(AgentNode {
            id: "agent_b".to_string(),
            label: "Parallel B".to_string(),
            objective: "Handle the right branch.".to_string(),
            input_mappings: HashMap::from([(
                "context".to_string(),
                "{{workflow.input.data.right}}".to_string(),
            )]),
            output_key: "nodes.agent_b.output".to_string(),
            system_timeout_ms: None,
        }),
        WorkflowNode::Output(OutputNode {
            id: "output".to_string(),
            label: "Output".to_string(),
            input_mapping: "{{nodes.agent_a.output.data}} + {{nodes.agent_b.output.data}}"
                .to_string(),
            output_schema: json!({"type": "string"}),
            completion_kind: WorkflowCompletionKind::Result,
        }),
    ];
    let edges = vec![
        ("e1", "input", "out", "agent_a"),
        ("e2", "input", "out", "agent_b"),
        ("e3", "agent_a", "out", "output"),
        ("e4", "agent_b", "out", "output"),
    ]
    .into_iter()
    .map(|(id, source, port, target)| WorkflowEdge {
        id: id.to_string(),
        source_node_id: source.to_string(),
        source_port: port.to_string(),
        target_node_id: target.to_string(),
        target_port: None,
    })
    .collect();

    CompiledWorkflow {
        workflow_ir: WorkflowIr {
            schema_version: "1.0.0".to_string(),
            workflow_id: "sprint-61-parallel".to_string(),
            workflow_version: 1,
            name: "Sprint 61 Parallel DAG".to_string(),
            description: String::new(),
            compiler: CompilerTarget {
                model: "gemma-4-e2b-qat".to_string(),
            },
            metadata: None,
            nodes,
            edges,
        },
        instructions: HashMap::from([
            (
                "agent_a".to_string(),
                instruction(
                    "sprint-61-parallel",
                    "agent_a",
                    "{{workflow.input.data.left}}",
                ),
            ),
            (
                "agent_b".to_string(),
                instruction(
                    "sprint-61-parallel",
                    "agent_b",
                    "{{workflow.input.data.right}}",
                ),
            ),
        ]),
    }
}

fn assert_system_action_pauses_for_approval(
    action_type: SystemActionType,
    command: &str,
    args: Vec<String>,
) -> ApprovalRequest {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.nodes[1] = WorkflowNode::SystemAction(SystemActionNode {
        id: "agent".to_string(),
        label: "Risky system action".to_string(),
        action_type,
        command: command.to_string(),
        args: args.clone(),
        working_directory: None,
        system_timeout_ms: None,
        timeout_ms: 1_000,
        max_output_bytes: 4,
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
        &NoExternalTools,
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
    assert_eq!(approval.context["actionType"], json!("system_action"));
    assert_eq!(approval.context["command"], json!(command));
    assert_eq!(approval.context["args"], json!(args));
    approval
}

fn permission_workflow() -> CompiledWorkflow {
    let nodes = vec![
        WorkflowNode::Input(InputNode {
            id: "input".to_string(),
            label: "Input".to_string(),
            output_key: "workflow.input".to_string(),
            input_schema: json!({"type": "object"}),
        }),
        WorkflowNode::Agent(AgentNode {
            id: "agent1".to_string(),
            label: "Generate Rust".to_string(),
            objective: "Generate a large Rust codebase.".to_string(),
            input_mappings: HashMap::from([(
                "context".to_string(),
                "{{workflow.input}}".to_string(),
            )]),
            output_key: "nodes.agent1.output".to_string(),
            system_timeout_ms: None,
        }),
        WorkflowNode::Router(RouterNode {
            id: "router".to_string(),
            label: "Route".to_string(),
            expression: "true".to_string(),
            routes: vec![
                RouterRoute {
                    port: "matched".to_string(),
                    condition: "true".to_string(),
                },
                RouterRoute {
                    port: "not_matched".to_string(),
                    condition: "false".to_string(),
                },
            ],
            system_timeout_ms: None,
        }),
        WorkflowNode::Permission(PermissionNode {
            id: "permission".to_string(),
            label: "Review".to_string(),
            permission: PermissionKind::FileWrite,
            reason: "Review generated Rust code before file write".to_string(),
            on_denied: PermissionDeniedBehavior::Fail,
        }),
        WorkflowNode::Agent(AgentNode {
            id: "agent2".to_string(),
            label: "Finalize".to_string(),
            objective: "Finalize the approved asset.".to_string(),
            input_mappings: HashMap::from([(
                "context".to_string(),
                "{{nodes.agent1.output}}".to_string(),
            )]),
            output_key: "nodes.agent2.output".to_string(),
            system_timeout_ms: None,
        }),
        WorkflowNode::Output(OutputNode {
            id: "output".to_string(),
            label: "Output".to_string(),
            input_mapping: "{{nodes.agent2.output}}".to_string(),
            output_schema: json!({"type": "object"}),
            completion_kind: WorkflowCompletionKind::Result,
        }),
    ];
    let edges = vec![
        ("e1", "input", "out", "agent1"),
        ("e2", "agent1", "out", "router"),
        ("e3", "router", "matched", "permission"),
        ("e4", "router", "not_matched", "output"),
        ("e5", "permission", "approved", "agent2"),
        ("e6", "agent2", "out", "output"),
    ]
    .into_iter()
    .map(|(id, source, port, target)| WorkflowEdge {
        id: id.to_string(),
        source_node_id: source.to_string(),
        source_port: port.to_string(),
        target_node_id: target.to_string(),
        target_port: None,
    })
    .collect();
    let instruction = |node_id: &str, mapping: &str| CompiledInstruction {
        id: format!("instruction-{node_id}"),
        workflow_id: "permission-workflow".to_string(),
        workflow_version: 1,
        node_id: node_id.to_string(),
        node_kind: WorkflowNodeKind::Agent,
        system_prompt: format!("Execute {node_id} deterministically."),
        input_variable_mappings: HashMap::from([("context".to_string(), mapping.to_string())]),
        evaluation_protocol: json!({
            "successCriteria": ["exists"],
            "failureAction": "fail",
            "maxRetries": 0
        }),
        compiler_model: "gemma-4-e2b-qat".to_string(),
        compiler_version: "1.0.0".to_string(),
        created_at_ms: 1,
    };
    CompiledWorkflow {
        workflow_ir: WorkflowIr {
            schema_version: "1.0.0".to_string(),
            workflow_id: "permission-workflow".to_string(),
            workflow_version: 1,
            name: "Permission workflow".to_string(),
            description: String::new(),
            compiler: CompilerTarget {
                model: "gemma-4-e2b-qat".to_string(),
            },
            metadata: None,
            nodes,
            edges,
        },
        instructions: HashMap::from([
            (
                "agent1".to_string(),
                instruction("agent1", "{{workflow.input}}"),
            ),
            (
                "agent2".to_string(),
                instruction("agent2", "{{nodes.agent1.output}}"),
            ),
        ]),
    }
}

fn local_sandbox_log_summarizer_workflow() -> CompiledWorkflow {
    let nodes = vec![
        WorkflowNode::Input(InputNode {
            id: "manual-start".to_string(),
            label: "Manual Start".to_string(),
            output_key: "workflow.input".to_string(),
            input_schema: json!({"type": "object"}),
        }),
        WorkflowNode::McpTool(McpToolNode {
            id: "local-sandbox-read".to_string(),
            label: "Read Sandbox Instructions".to_string(),
            server_name: "local_filesystem".to_string(),
            tool_name: "read_file".to_string(),
            arguments: json!({"path": "instruction_input.txt"}),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: Some(15_000),
        }),
        WorkflowNode::Agent(AgentNode {
            id: "local-sandbox-summary".to_string(),
            label: "Gemma 4 Summary".to_string(),
            objective: "Generate an executive summary grounded in the sandbox read payload."
                .to_string(),
            input_mappings: HashMap::from([(
                "sandbox_read".to_string(),
                "{{nodes.local-sandbox-read.output.data.structuredContent.content}}".to_string(),
            )]),
            output_key: "nodes.local-sandbox-summary.output".to_string(),
            system_timeout_ms: None,
        }),
        WorkflowNode::Permission(PermissionNode {
            id: "local-sandbox-approval".to_string(),
            label: "Approve Report".to_string(),
            permission: PermissionKind::FileWrite,
            reason: "Write executive_summary.txt after human approval".to_string(),
            on_denied: PermissionDeniedBehavior::Fail,
        }),
        WorkflowNode::McpTool(McpToolNode {
            id: "local-sandbox-write".to_string(),
            label: "Write Executive Summary".to_string(),
            server_name: "local_filesystem".to_string(),
            tool_name: "write_file".to_string(),
            arguments: json!({
                "path": "executive_summary.txt",
                "content": "{{nodes.local-sandbox-summary.output.data}}"
            }),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: Some(15_000),
        }),
        WorkflowNode::Output(OutputNode {
            id: "local-sandbox-output".to_string(),
            label: "Output".to_string(),
            input_mapping: "{{nodes.local-sandbox-write.output}}".to_string(),
            output_schema: json!({"type": "object"}),
            completion_kind: WorkflowCompletionKind::Result,
        }),
    ];
    let edges = vec![
        ("e1", "manual-start", "out", "local-sandbox-read"),
        ("e2", "local-sandbox-read", "out", "local-sandbox-summary"),
        (
            "e3",
            "local-sandbox-summary",
            "out",
            "local-sandbox-approval",
        ),
        (
            "e4",
            "local-sandbox-approval",
            "approved",
            "local-sandbox-write",
        ),
        ("e5", "local-sandbox-write", "out", "local-sandbox-output"),
    ]
    .into_iter()
    .map(|(id, source, port, target)| WorkflowEdge {
        id: id.to_string(),
        source_node_id: source.to_string(),
        source_port: port.to_string(),
        target_node_id: target.to_string(),
        target_port: None,
    })
    .collect();
    let instruction = CompiledInstruction {
            id: "instruction-local-sandbox-summary".to_string(),
            workflow_id: "local-sandbox-log-summarizer".to_string(),
            workflow_version: 1,
            node_id: "local-sandbox-summary".to_string(),
            node_kind: WorkflowNodeKind::Agent,
            system_prompt: "Summarize the sandbox MCP payload for an executive reader. Return a concise report with a Logical Certificate.".to_string(),
            input_variable_mappings: HashMap::from([(
                "sandbox_read".to_string(),
                "{{nodes.local-sandbox-read.output.data.structuredContent.content}}".to_string(),
            )]),
            evaluation_protocol: json!({
                "successCriteria": ["report contains a logical certificate"],
                "failureAction": "fail",
                "maxRetries": 0
            }),
            compiler_model: "gemma-4-e2b-qat".to_string(),
            compiler_version: "1.0.0".to_string(),
            created_at_ms: 1,
        };
    CompiledWorkflow {
        workflow_ir: WorkflowIr {
            schema_version: "1.0.0".to_string(),
            workflow_id: "local-sandbox-log-summarizer".to_string(),
            workflow_version: 1,
            name: "Local Sandbox Log Summarizer".to_string(),
            description: "Reads the secure local sandbox and writes an approved executive summary."
                .to_string(),
            compiler: CompilerTarget {
                model: "gemma-4-e2b-qat".to_string(),
            },
            metadata: None,
            nodes,
            edges,
        },
        instructions: HashMap::from([("local-sandbox-summary".to_string(), instruction)]),
    }
}

fn register_local_filesystem_server(registry: &McpClientRegistry, sandbox_root: &Path) {
    let config = McpServerConfig {
        name: "local_filesystem".to_string(),
        command: "oomu-native".to_string(),
        args: Vec::new(),
        env: HashMap::from([(
            "OOMU_MCP_SANDBOX_DIR".to_string(),
            sandbox_root.to_string_lossy().to_string(),
        )]),
        transport: McpTransportConfig::Native,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");
    assert_eq!(
        runtime.block_on(registry.register_server_configs(vec![config])),
        1
    );
}

fn logical_certificate_is_valid(text: &str) -> bool {
    let Some(divider) = text.find("\n---\n").or_else(|| text.find("---\n")) else {
        return false;
    };
    let certificate = &text[divider..];
    ["Premises:", "Execution Path:", "Formal Conclusion:"]
        .iter()
        .all(|header| {
            certificate
                .find(header)
                .and_then(|index| certificate[index + header.len()..].lines().next())
                .is_some_and(|line| !line.trim().is_empty())
        })
}

mod collection;
mod execution;
mod mcp;
mod mcp_2;
mod permission;
mod system_action;
