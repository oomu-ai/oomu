use super::*;
use crate::db::SavedWorkflowRecord;
use crate::mcp::{client::McpServerConfig, shield::McpTransportConfig};
use crate::workflow_ir::{
    CompilerTarget, ConditionalNode, InputNode, LoopNode, McpToolNode, OutputNode, PermissionKind,
    PermissionNode, RouterRoute, SystemActionNode, SystemActionType, WorkflowNodeKind,
    WORKFLOW_COMPILER_MODEL, WORKFLOW_IR_SCHEMA_VERSION,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering as AtomicOrdering},
    Arc, Mutex,
};

#[derive(Clone)]
struct StubModel;
#[derive(Clone)]
struct NoExternalTools;
#[derive(Clone)]
struct StubExternalTools;
#[derive(Clone)]
struct SyncExternalTools;
#[derive(Clone)]
struct SlowModel;
#[derive(Clone)]
struct FailingPreflightTools;
#[derive(Clone)]
struct CountingReadinessTools {
    checks: Arc<AtomicUsize>,
    executions: Arc<AtomicUsize>,
    fail_on_check: Option<usize>,
}
#[derive(Clone)]
struct RemoteReviewTools {
    binding: McpToolApprovalBinding,
    executions: Arc<AtomicUsize>,
}
#[derive(Clone)]
struct ConditionalFixtureModel;
#[derive(Clone)]
struct RepairFixtureModel;
#[derive(Clone)]
struct CountingCollectionModel {
    calls: Arc<AtomicUsize>,
}
#[derive(Clone)]
struct FixedMcpResultTools {
    result: Value,
    executions: Arc<AtomicUsize>,
    delay_ms: u64,
}
#[derive(Clone)]
struct SelectiveReadinessTools {
    source_result: Value,
    offline_server: String,
    readiness_checks: Arc<Mutex<Vec<String>>>,
    executions: Arc<Mutex<Vec<String>>>,
}
#[derive(Clone)]
struct PerNodeCollectionTools {
    results: HashMap<String, Value>,
    executions: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
struct SandboxSummaryModel {
    observed_sources: Arc<Mutex<Vec<String>>>,
}

fn remote_review_binding(
    server_name: &str,
    tool_name: &str,
    destination: &str,
) -> McpToolApprovalBinding {
    McpToolApprovalBinding {
        server_name: server_name.to_string(),
        tool_name: tool_name.to_string(),
        arguments_binding: "stable-arguments".to_string(),
        canonical_origin: Some(format!("https://{destination}")),
        transport: "https".to_string(),
        resolved_destination_class: Some("public".to_string()),
        destination_binding: Some(format!("destination-{destination}")),
        server_identity_binding: Some("stable-server".to_string()),
        certificate_binding: Some("stable-certificate".to_string()),
        tool_definition_binding: "stable-tool".to_string(),
        response_byte_limit: 1_048_576,
        requires_native_shield: true,
    }
}

impl RuntimeModel for StubModel {
    fn execute_agent(
        &self,
        _session_id: &str,
        _system_prompt: &str,
        variables: &Map<String, Value>,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        Ok(ModelOutput {
            text: format!("handled:{}", variables["context"]),
            prompt_tokens: 7,
            completion_tokens: 3,
        })
    }

    fn classify_route(
        &self,
        _session_id: &str,
        _router: &RouterNode,
        _input: &Value,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        Ok(ModelOutput {
            text: "not_matched".to_string(),
            prompt_tokens: 4,
            completion_tokens: 1,
        })
    }
}

impl RuntimeExternalTools for NoExternalTools {
    fn ensure_mcp_server_ready(
        &self,
        _server_name: &str,
        _timeout_ms: u64,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }

    fn execute_mcp_tool(
        &self,
        _execution_id: &str,
        _node_id: &str,
        _label: &str,
        _server_name: &str,
        _tool_name: &str,
        _arguments: Value,
        _timeout_ms: u64,
        _approval_binding: Option<McpToolApprovalBinding>,
        _human_approved: bool,
    ) -> Result<Value, WorkflowRuntimeError> {
        Err(WorkflowRuntimeError::execution(
            "No external tools are registered for this test.".to_string(),
        ))
    }
}

impl RuntimeExternalTools for StubExternalTools {
    fn ensure_mcp_server_ready(
        &self,
        _server_name: &str,
        _timeout_ms: u64,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }

    fn execute_mcp_tool(
        &self,
        _execution_id: &str,
        _node_id: &str,
        _label: &str,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
        _timeout_ms: u64,
        _approval_binding: Option<McpToolApprovalBinding>,
        _human_approved: bool,
    ) -> Result<Value, WorkflowRuntimeError> {
        Ok(json!({
            "content": [{ "type": "text", "text": "ok" }],
            "structuredContent": {
                "serverName": server_name,
                "toolName": tool_name,
                "arguments": arguments,
            },
            "isError": false
        }))
    }
}

impl RuntimeExternalTools for FixedMcpResultTools {
    fn ensure_mcp_server_ready(
        &self,
        _server_name: &str,
        _timeout_ms: u64,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }

    fn execute_mcp_tool(
        &self,
        _execution_id: &str,
        _node_id: &str,
        _label: &str,
        _server_name: &str,
        _tool_name: &str,
        _arguments: Value,
        _timeout_ms: u64,
        _approval_binding: Option<McpToolApprovalBinding>,
        _human_approved: bool,
    ) -> Result<Value, WorkflowRuntimeError> {
        if self.delay_ms > 0 {
            thread::sleep(Duration::from_millis(self.delay_ms));
        }
        self.executions.fetch_add(1, AtomicOrdering::SeqCst);
        Ok(self.result.clone())
    }
}

impl RuntimeExternalTools for SelectiveReadinessTools {
    fn ensure_mcp_server_ready(
        &self,
        server_name: &str,
        _timeout_ms: u64,
    ) -> Result<(), WorkflowRuntimeError> {
        self.readiness_checks
            .lock()
            .unwrap()
            .push(server_name.to_string());
        if server_name == self.offline_server {
            return Err(WorkflowRuntimeError::mcp_server_unreachable(
                server_name,
                "simulated offline branch server".to_string(),
            ));
        }
        Ok(())
    }

    fn execute_mcp_tool(
        &self,
        _execution_id: &str,
        _node_id: &str,
        _label: &str,
        server_name: &str,
        _tool_name: &str,
        _arguments: Value,
        _timeout_ms: u64,
        _approval_binding: Option<McpToolApprovalBinding>,
        _human_approved: bool,
    ) -> Result<Value, WorkflowRuntimeError> {
        self.executions
            .lock()
            .unwrap()
            .push(server_name.to_string());
        Ok(self.source_result.clone())
    }
}

impl RuntimeExternalTools for PerNodeCollectionTools {
    fn ensure_mcp_server_ready(
        &self,
        _server_name: &str,
        _timeout_ms: u64,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }

    fn execute_mcp_tool(
        &self,
        _execution_id: &str,
        node_id: &str,
        _label: &str,
        _server_name: &str,
        _tool_name: &str,
        _arguments: Value,
        _timeout_ms: u64,
        _approval_binding: Option<McpToolApprovalBinding>,
        _human_approved: bool,
    ) -> Result<Value, WorkflowRuntimeError> {
        self.executions.lock().unwrap().push(node_id.to_string());
        Ok(self.results.get(node_id).cloned().unwrap_or_else(|| {
            json!({
                "content": [],
                "structuredContent": {"written": true},
                "isError": false
            })
        }))
    }
}

impl RuntimeExternalTools for RemoteReviewTools {
    fn ensure_mcp_server_ready(
        &self,
        _server_name: &str,
        _timeout_ms: u64,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }

    fn prepare_mcp_tool_approval_binding(
        &self,
        _server_name: &str,
        _tool_name: &str,
        _arguments: Value,
        _timeout_ms: u64,
    ) -> Result<Option<McpToolApprovalBinding>, WorkflowRuntimeError> {
        Ok(Some(self.binding.clone()))
    }

    fn execute_mcp_tool(
        &self,
        _execution_id: &str,
        _node_id: &str,
        _label: &str,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
        _timeout_ms: u64,
        approval_binding: Option<McpToolApprovalBinding>,
        human_approved: bool,
    ) -> Result<Value, WorkflowRuntimeError> {
        if !human_approved || approval_binding.as_ref() != Some(&self.binding) {
            return Err(WorkflowRuntimeError::permission_rejected(
                "The exact remote service was not approved.",
            ));
        }
        self.executions.fetch_add(1, AtomicOrdering::SeqCst);
        Ok(json!({
            "content": [{ "type": "text", "text": "ok" }],
            "structuredContent": {
                "serverName": server_name,
                "toolName": tool_name,
                "arguments": arguments,
            },
            "isError": false
        }))
    }
}

impl RuntimeExternalTools for SyncExternalTools {
    fn ensure_mcp_server_ready(
        &self,
        server_name: &str,
        _timeout_ms: u64,
    ) -> Result<(), WorkflowRuntimeError> {
        Err(WorkflowRuntimeError::execution(format!(
            "unexpected MCP preflight for {server_name}"
        )))
    }

    fn execute_mcp_tool(
        &self,
        _execution_id: &str,
        _node_id: &str,
        _label: &str,
        _server_name: &str,
        _tool_name: &str,
        _arguments: Value,
        _timeout_ms: u64,
        _approval_binding: Option<McpToolApprovalBinding>,
        _human_approved: bool,
    ) -> Result<Value, WorkflowRuntimeError> {
        panic!("local sync should not call external MCP execution")
    }

    fn execute_sync_knowledge_vault(
        &self,
        arguments: Value,
    ) -> Result<Value, WorkflowRuntimeError> {
        Ok(json!({
            "indexedFiles": 2,
            "indexedChunks": 5,
            "skippedFiles": 0,
            "arguments": arguments,
        }))
    }
}

impl RuntimeExternalTools for FailingPreflightTools {
    fn ensure_mcp_server_ready(
        &self,
        server_name: &str,
        _timeout_ms: u64,
    ) -> Result<(), WorkflowRuntimeError> {
        Err(WorkflowRuntimeError::mcp_server_unreachable(
            server_name,
            "simulated connection refusal".to_string(),
        ))
    }

    fn execute_mcp_tool(
        &self,
        _execution_id: &str,
        _node_id: &str,
        _label: &str,
        _server_name: &str,
        _tool_name: &str,
        _arguments: Value,
        _timeout_ms: u64,
        _approval_binding: Option<McpToolApprovalBinding>,
        _human_approved: bool,
    ) -> Result<Value, WorkflowRuntimeError> {
        panic!("MCP tool execution must not run when preflight fails")
    }
}

impl CountingReadinessTools {
    fn fail_on_check(check: usize) -> Self {
        Self {
            checks: Arc::new(AtomicUsize::new(0)),
            executions: Arc::new(AtomicUsize::new(0)),
            fail_on_check: Some(check),
        }
    }
}

impl RuntimeExternalTools for CountingReadinessTools {
    fn ensure_mcp_server_ready(
        &self,
        server_name: &str,
        _timeout_ms: u64,
    ) -> Result<(), WorkflowRuntimeError> {
        let check = self.checks.fetch_add(1, AtomicOrdering::SeqCst) + 1;
        if self.fail_on_check == Some(check) {
            return Err(WorkflowRuntimeError::mcp_server_unreachable(
                server_name,
                "simulated socket refusal".to_string(),
            ));
        }
        Ok(())
    }

    fn execute_mcp_tool(
        &self,
        _execution_id: &str,
        _node_id: &str,
        _label: &str,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
        _timeout_ms: u64,
        _approval_binding: Option<McpToolApprovalBinding>,
        _human_approved: bool,
    ) -> Result<Value, WorkflowRuntimeError> {
        self.executions.fetch_add(1, AtomicOrdering::SeqCst);
        Ok(json!({
            "content": [{ "type": "text", "text": "ok" }],
            "structuredContent": {
                "serverName": server_name,
                "toolName": tool_name,
                "arguments": arguments,
            },
            "isError": false
        }))
    }
}

impl RuntimeModel for SlowModel {
    fn execute_agent(
        &self,
        _session_id: &str,
        _system_prompt: &str,
        _variables: &Map<String, Value>,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        thread::sleep(Duration::from_millis(150));
        Ok(ModelOutput {
            text: "too late".to_string(),
            prompt_tokens: 0,
            completion_tokens: 0,
        })
    }

    fn classify_route(
        &self,
        _session_id: &str,
        _router: &RouterNode,
        _input: &Value,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        unreachable!("timeout test uses an agent node")
    }
}

impl RuntimeModel for ConditionalFixtureModel {
    fn execute_agent(
        &self,
        _session_id: &str,
        _system_prompt: &str,
        variables: &Map<String, Value>,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        Ok(ModelOutput {
            text: format!("branch:{}", variables["context"]),
            prompt_tokens: 2,
            completion_tokens: 2,
        })
    }

    fn classify_route(
        &self,
        _session_id: &str,
        _router: &RouterNode,
        _input: &Value,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        unreachable!("conditional fixture does not use router nodes")
    }

    fn evaluate_condition(
        &self,
        _session_id: &str,
        _conditional: &ConditionalNode,
        input: &Value,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        Ok(ModelOutput {
            text: if input == "green" { "true" } else { "false" }.to_string(),
            prompt_tokens: 3,
            completion_tokens: 1,
        })
    }
}

impl RuntimeModel for CountingCollectionModel {
    fn execute_agent(
        &self,
        _session_id: &str,
        _system_prompt: &str,
        _variables: &Map<String, Value>,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        Ok(ModelOutput {
            text: "handled first collection item".to_string(),
            prompt_tokens: 3,
            completion_tokens: 2,
        })
    }

    fn classify_route(
        &self,
        _session_id: &str,
        _router: &RouterNode,
        _input: &Value,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        unreachable!("collection fixture has no router")
    }
}

impl RuntimeModel for RepairFixtureModel {
    fn execute_agent(
        &self,
        _session_id: &str,
        _system_prompt: &str,
        variables: &Map<String, Value>,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        StubModel.execute_agent(_session_id, _system_prompt, variables)
    }

    fn classify_route(
        &self,
        session_id: &str,
        router: &RouterNode,
        input: &Value,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        StubModel.classify_route(session_id, router, input)
    }

    fn repair_system_action(
        &self,
        _session_id: &str,
        _failure: &SystemActionFailureContext,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        Ok(ModelOutput {
                text: r#"{"command":"echo","args":["healed"],"workingDirectory":"","explanation":"replace failing binary with harmless echo"}"#.to_string(),
                prompt_tokens: 5,
                completion_tokens: 5,
            })
    }
}

#[derive(Clone)]
struct FileReferenceModel {
    calls: Arc<AtomicUsize>,
}

impl RuntimeModel for FileReferenceModel {
    fn execute_agent(
        &self,
        _session_id: &str,
        _system_prompt: &str,
        variables: &Map<String, Value>,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        let call = self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        let text = if call == 0 {
            "x".repeat(LARGE_OUTPUT_BYTES + 1)
        } else {
            let path = variables
                .get("context")
                .and_then(|value| value.get("assetPath"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    WorkflowRuntimeError::execution(
                        "Agent 2 did not receive Agent 1's asset path.".to_string(),
                    )
                })?;
            format!("reviewed:{path}")
        };
        Ok(ModelOutput {
            text,
            prompt_tokens: 5,
            completion_tokens: 2,
        })
    }

    fn classify_route(
        &self,
        _session_id: &str,
        _router: &RouterNode,
        _input: &Value,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        unreachable!("the integration router uses deterministic basic logic")
    }
}

impl RuntimeModel for SandboxSummaryModel {
    fn execute_agent(
        &self,
        _session_id: &str,
        _system_prompt: &str,
        variables: &Map<String, Value>,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        let sandbox_text = variables
            .get("sandbox_read")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                WorkflowRuntimeError::execution(
                    "Sandbox summary model did not receive read_file content.".to_string(),
                )
            })?;
        self.observed_sources
            .lock()
            .unwrap()
            .push(sandbox_text.to_string());
        Ok(ModelOutput {
                text: format!(
                    "Executive summary: {sandbox_text}\n---\nPremises: The local filesystem MCP read instruction_input.txt from the secure sandbox.\nExecution Path: The mock runtime model summarized the MCP payload and the approved workflow wrote executive_summary.txt.\nFormal Conclusion: The offline sandbox summarizer completed with grounded output."
                ),
                prompt_tokens: 11,
                completion_tokens: 17,
            })
    }

    fn classify_route(
        &self,
        _session_id: &str,
        _router: &RouterNode,
        _input: &Value,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        unreachable!("local sandbox summarizer fixture has no router")
    }
}

fn compiled_workflow(with_router: bool) -> CompiledWorkflow {
    let input = WorkflowNode::Input(InputNode {
        id: "input".to_string(),
        label: "Input".to_string(),
        output_key: "workflow.input".to_string(),
        input_schema: json!({"type": "object"}),
    });
    let agent = WorkflowNode::Agent(AgentNode {
        id: "agent".to_string(),
        label: "Agent".to_string(),
        objective: "Handle".to_string(),
        input_mappings: HashMap::from([("context".to_string(), "{{workflow.input}}".to_string())]),
        output_key: "nodes.agent.output".to_string(),
        system_timeout_ms: None,
    });
    let output = WorkflowNode::Output(OutputNode {
        id: "output".to_string(),
        label: "Output".to_string(),
        input_mapping: "{{nodes.agent.output}}".to_string(),
        output_schema: json!({"type": "object"}),
        completion_kind: WorkflowCompletionKind::Result,
    });
    let mut nodes = vec![input, agent];
    let mut edges = vec![WorkflowEdge {
        id: "e1".to_string(),
        source_node_id: "input".to_string(),
        source_port: "out".to_string(),
        target_node_id: "agent".to_string(),
        target_port: None,
    }];
    if with_router {
        nodes.push(WorkflowNode::Router(RouterNode {
            id: "router".to_string(),
            label: "Router".to_string(),
            expression: "false".to_string(),
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
        }));
        edges.push(WorkflowEdge {
            id: "e2".to_string(),
            source_node_id: "agent".to_string(),
            source_port: "out".to_string(),
            target_node_id: "router".to_string(),
            target_port: None,
        });
        edges.push(WorkflowEdge {
            id: "e3".to_string(),
            source_node_id: "router".to_string(),
            source_port: "matched".to_string(),
            target_node_id: "output".to_string(),
            target_port: None,
        });
        edges.push(WorkflowEdge {
            id: "e4".to_string(),
            source_node_id: "router".to_string(),
            source_port: "not_matched".to_string(),
            target_node_id: "output".to_string(),
            target_port: None,
        });
    } else {
        edges.push(WorkflowEdge {
            id: "e2".to_string(),
            source_node_id: "agent".to_string(),
            source_port: "out".to_string(),
            target_node_id: "output".to_string(),
            target_port: None,
        });
    }
    nodes.push(output);
    let instruction = CompiledInstruction {
        id: "instruction".to_string(),
        workflow_id: "workflow".to_string(),
        workflow_version: 1,
        node_id: "agent".to_string(),
        node_kind: WorkflowNodeKind::Agent,
        system_prompt: "Handle input.".to_string(),
        input_variable_mappings: HashMap::from([(
            "context".to_string(),
            "{{workflow.input}}".to_string(),
        )]),
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
            workflow_id: "workflow".to_string(),
            workflow_version: 1,
            name: "Runtime".to_string(),
            description: String::new(),
            compiler: CompilerTarget {
                model: "gemma-4-e2b-qat".to_string(),
            },
            metadata: None,
            nodes,
            edges,
        },
        instructions: HashMap::from([("agent".to_string(), instruction)]),
    }
}

fn indexed_collection_workflow(index: usize) -> CompiledWorkflow {
    let mapping =
        format!("{{{{nodes.read.output.data.structuredContent.emails.{index}.subject}}}}");
    let nodes = vec![
        WorkflowNode::Input(InputNode {
            id: "input".to_string(),
            label: "Input".to_string(),
            output_key: "workflow.input".to_string(),
            input_schema: json!({"type": "object"}),
        }),
        WorkflowNode::McpTool(McpToolNode {
            id: "read".to_string(),
            label: "Read collection".to_string(),
            server_name: "local_filesystem".to_string(),
            tool_name: "read_file".to_string(),
            arguments: json!({"path": "collection.json"}),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: Some(1_000),
        }),
        WorkflowNode::Agent(AgentNode {
            id: "consumer".to_string(),
            label: "Prepare collection action".to_string(),
            objective: "Prepare the downstream action from the collection payload.".to_string(),
            input_mappings: HashMap::from([(
                "context".to_string(),
                "{{nodes.read.output}}".to_string(),
            )]),
            output_key: "nodes.consumer.output".to_string(),
            system_timeout_ms: None,
        }),
        WorkflowNode::McpTool(McpToolNode {
            id: "downstream-tool".to_string(),
            label: "Use first collection item".to_string(),
            server_name: "local_filesystem".to_string(),
            tool_name: "read_file".to_string(),
            arguments: json!({"path": mapping}),
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
        ("e1", "input", "out", "read"),
        ("e2", "read", "out", "consumer"),
        ("e3", "consumer", "out", "downstream-tool"),
        ("e4", "downstream-tool", "out", "output"),
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
            schema_version: WORKFLOW_IR_SCHEMA_VERSION.to_string(),
            workflow_id: "indexed-collection-workflow".to_string(),
            workflow_version: 1,
            name: "Indexed collection workflow".to_string(),
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
            instruction(
                "indexed-collection-workflow",
                "consumer",
                "{{nodes.read.output}}",
            ),
        )]),
    }
}

fn email_responder_workflow() -> CompiledWorkflow {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion": WORKFLOW_IR_SCHEMA_VERSION,
            "workflowId": "email-responder-e2e",
            "workflowVersion": 1,
            "name": "Email Responder",
            "description": "Read unread mail, prepare a reply, and ask before opening a draft.",
            "compiler": { "model": WORKFLOW_COMPILER_MODEL },
            "nodes": [
                {
                    "kind": "input",
                    "id": "input",
                    "label": "Input",
                    "outputKey": "workflow.input",
                    "inputSchema": { "type": "object" }
                },
                {
                    "kind": "mcp_tool",
                    "id": "read-unread-emails",
                    "label": "Read macOS Emails",
                    "serverName": "macos_applescript",
                    "toolName": "read_system_emails",
                    "arguments": { "max_messages": 5, "unread_only": true },
                    "outputSchema": {
                        "type": "object",
                        "properties": {
                            "structuredContent": {
                                "type": "object",
                                "properties": { "emails": { "type": "array" } }
                            }
                        },
                        "x-oomu-result-contract": {
                            "kind": "collection",
                            "path": "/structuredContent/emails",
                            "emptyIsSuccess": true
                        }
                    }
                },
                {
                    "kind": "conditional",
                    "id": "mail-has-messages",
                    "label": "Check mail",
                    "condition": "$ != []",
                    "inputMapping": "{{nodes.read-unread-emails.output.data.structuredContent.emails}}"
                },
                {
                    "kind": "agent",
                    "id": "draft-reply",
                    "label": "Draft reply",
                    "objective": "Draft a professional reply without inventing details.",
                    "inputMappings": { "context": "{{nodes.read-unread-emails.output}}" },
                    "outputKey": "nodes.draft-reply.output"
                },
                {
                    "kind": "permission",
                    "id": "approve-email-reply",
                    "label": "Approve Email Reply",
                    "permission": "mcp_tool",
                    "reason": "Review the reply before opening the Mail draft.",
                    "onDenied": "fail"
                },
                {
                    "kind": "mcp_tool",
                    "id": "draft-outgoing-email",
                    "label": "Draft Outgoing Email",
                    "serverName": "macos_applescript",
                    "toolName": "draft_system_email",
                    "arguments": {
                        "to": "{{nodes.read-unread-emails.output.data.structuredContent.emails.0.sender}}",
                        "subject": "Re: {{nodes.read-unread-emails.output.data.structuredContent.emails.0.subject}}",
                        "body": "{{nodes.draft-reply.output.data}}",
                        "cc": "",
                        "bcc": ""
                    }
                },
                {
                    "kind": "output",
                    "id": "output",
                    "label": "Ready",
                    "inputMapping": "{{nodes.draft-outgoing-email.output}}",
                    "outputSchema": { "type": "object" }
                },
                {
                    "kind": "output",
                    "id": "empty-output",
                    "label": "Nothing found",
                    "inputMapping": "{{nodes.read-unread-emails.output.data.structuredContent.emails}}",
                    "outputSchema": { "type": "array" },
                    "completionKind": "empty_collection"
                }
            ],
            "edges": [
                { "id": "e1", "sourceNodeId": "input", "sourcePort": "out", "targetNodeId": "read-unread-emails" },
                { "id": "e2", "sourceNodeId": "read-unread-emails", "sourcePort": "out", "targetNodeId": "mail-has-messages" },
                { "id": "e3", "sourceNodeId": "mail-has-messages", "sourcePort": "true", "targetNodeId": "draft-reply" },
                { "id": "e4", "sourceNodeId": "mail-has-messages", "sourcePort": "false", "targetNodeId": "empty-output" },
                { "id": "e5", "sourceNodeId": "draft-reply", "sourcePort": "out", "targetNodeId": "approve-email-reply" },
                { "id": "e6", "sourceNodeId": "approve-email-reply", "sourcePort": "approved", "targetNodeId": "draft-outgoing-email" },
                { "id": "e7", "sourceNodeId": "draft-outgoing-email", "sourcePort": "out", "targetNodeId": "output" }
            ]
        }))
        .expect("Email Responder fixture is valid Workflow IR");
    workflow_ir
        .validate()
        .expect("Email Responder fixture passes the platform contract");

    CompiledWorkflow {
        instructions: HashMap::from([(
            "draft-reply".to_string(),
            instruction(
                "email-responder-e2e",
                "draft-reply",
                "{{nodes.read-unread-emails.output}}",
            ),
        )]),
        workflow_ir,
    }
}

fn calendar_assistant_workflow() -> CompiledWorkflow {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion": WORKFLOW_IR_SCHEMA_VERSION,
            "workflowId": "calendar-assistant-e2e",
            "workflowVersion": 1,
            "name": "Calendar Assistant",
            "description": "Read upcoming Calendar events, summarize them, and show a notification.",
            "compiler": { "model": WORKFLOW_COMPILER_MODEL },
            "nodes": [
                {
                    "kind": "input",
                    "id": "input",
                    "label": "Input",
                    "outputKey": "workflow.input",
                    "inputSchema": { "type": "object" }
                },
                {
                    "kind": "mcp_tool",
                    "id": "calendar-assistant-read",
                    "label": "Read macOS Calendar",
                    "serverName": "macos_applescript",
                    "toolName": "read_system_calendar",
                    "arguments": {
                        "calendar_name": "",
                        "hours_ahead": 24,
                        "start_date": "",
                        "end_date": ""
                    },
                    "systemTimeoutMs": 1,
                    "outputSchema": {
                        "type": "object",
                        "properties": {
                            "structuredContent": {
                                "type": "object",
                                "properties": { "events": { "type": "array" } }
                            }
                        },
                        "x-oomu-result-contract": {
                            "kind": "collection",
                            "path": "/structuredContent/events",
                            "emptyIsSuccess": true
                        }
                    }
                },
                {
                    "kind": "conditional",
                    "id": "calendar-assistant-has-events",
                    "label": "Check for events",
                    "condition": "$ != []",
                    "inputMapping": "{{nodes.calendar-assistant-read.output.data.structuredContent.events}}"
                },
                {
                    "kind": "agent",
                    "id": "meeting-audit",
                    "label": "Meeting audit",
                    "objective": "Summarize the returned meetings without inventing details.",
                    "inputMappings": { "context": "{{nodes.calendar-assistant-read.output}}" },
                    "outputKey": "nodes.meeting-audit.output"
                },
                {
                    "kind": "mcp_tool",
                    "id": "calendar-notification",
                    "label": "Show notification",
                    "serverName": "macos_applescript",
                    "toolName": "trigger_system_notification",
                    "arguments": {
                        "title_text": "OOMU Calendar Assistant",
                        "subtitle_text": "Upcoming Meetings",
                        "body_text": "{{nodes.meeting-audit.output.data}}"
                    },
                    "systemTimeoutMs": 1
                },
                {
                    "kind": "output",
                    "id": "output",
                    "label": "Ready",
                    "inputMapping": "{{nodes.calendar-notification.output}}",
                    "outputSchema": { "type": "object" }
                },
                {
                    "kind": "output",
                    "id": "empty-output",
                    "label": "Nothing found",
                    "inputMapping": "{{nodes.calendar-assistant-read.output.data.structuredContent.events}}",
                    "outputSchema": { "type": "array" },
                    "completionKind": "empty_collection"
                }
            ],
            "edges": [
                { "id": "e1", "sourceNodeId": "input", "sourcePort": "out", "targetNodeId": "calendar-assistant-read" },
                { "id": "e2", "sourceNodeId": "calendar-assistant-read", "sourcePort": "out", "targetNodeId": "calendar-assistant-has-events" },
                { "id": "e3", "sourceNodeId": "calendar-assistant-has-events", "sourcePort": "true", "targetNodeId": "meeting-audit" },
                { "id": "e4", "sourceNodeId": "calendar-assistant-has-events", "sourcePort": "false", "targetNodeId": "empty-output" },
                { "id": "e5", "sourceNodeId": "meeting-audit", "sourcePort": "out", "targetNodeId": "calendar-notification" },
                { "id": "e6", "sourceNodeId": "calendar-notification", "sourcePort": "out", "targetNodeId": "output" }
            ]
        }))
        .expect("Calendar Assistant fixture is valid Workflow IR");
    workflow_ir
        .validate()
        .expect("Calendar Assistant fixture passes the platform contract");

    CompiledWorkflow {
        instructions: HashMap::from([(
            "meeting-audit".to_string(),
            instruction(
                "calendar-assistant-e2e",
                "meeting-audit",
                "{{nodes.calendar-assistant-read.output}}",
            ),
        )]),
        workflow_ir,
    }
}

fn instruction(workflow_id: &str, node_id: &str, template: &str) -> CompiledInstruction {
    CompiledInstruction {
        id: format!("instruction-{node_id}"),
        workflow_id: workflow_id.to_string(),
        workflow_version: 1,
        node_id: node_id.to_string(),
        node_kind: WorkflowNodeKind::Agent,
        system_prompt: format!("Execute {node_id}."),
        input_variable_mappings: HashMap::from([("context".to_string(), template.to_string())]),
        evaluation_protocol: json!({
            "successCriteria": ["exists"],
            "failureAction": "fail",
            "maxRetries": 0
        }),
        compiler_model: "gemma-4-e2b-qat".to_string(),
        compiler_version: "1.0.0".to_string(),
        created_at_ms: 1,
    }
}

mod scheduled;
mod support_continued;
