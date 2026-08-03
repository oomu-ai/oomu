use super::*;
use serde_json::json;

struct DeterministicTestCompiler;

impl InstructionCompiler for DeterministicTestCompiler {
    fn compile(&self, workflow_ir: &WorkflowIr) -> Result<CompilerOutput, WorkflowCompilerError> {
        let instructions = workflow_ir
            .nodes
            .iter()
            .filter_map(|node| match node {
                WorkflowNode::Agent(agent) => Some(CompilerInstruction {
                    node_id: agent.id.clone(),
                    system_prompt: agent.objective.clone(),
                    input_variable_mappings: agent
                        .input_mappings
                        .iter()
                        .map(|(name, template)| VariableMapping {
                            name: name.clone(),
                            template: template.clone(),
                        })
                        .collect(),
                    evaluation_protocol: EvaluationProtocol {
                        success_criteria: vec!["A grounded response exists.".to_string()],
                        failure_action: FailureAction::Fail,
                        max_retries: 0,
                    },
                }),
                _ => None,
            })
            .collect();
        Ok(CompilerOutput {
            compiler_version: COMPILER_VERSION.to_string(),
            instructions,
        })
    }
}

fn workflow_ir() -> WorkflowIr {
    serde_json::from_value(json!({
        "schemaVersion": "1.0.0",
        "workflowId": "wf-compiler-test",
        "workflowVersion": 1,
        "name": "Compiler test",
        "description": "Compile one agent",
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
                "kind": "agent",
                "id": "agent",
                "label": "Draft",
                "objective": "Draft a response",
                "inputMappings": { "request": "{{workflow.input}}" },
                "outputKey": "nodes.agent.output"
            },
            {
                "kind": "output",
                "id": "output",
                "label": "Output",
                "inputMapping": "{{nodes.agent.output}}",
                "outputSchema": { "type": "object" }
            }
        ],
        "edges": [
            {
                "id": "e1",
                "sourceNodeId": "input",
                "sourcePort": "out",
                "targetNodeId": "agent"
            },
            {
                "id": "e2",
                "sourceNodeId": "agent",
                "sourcePort": "out",
                "targetNodeId": "output"
            }
        ]
    }))
    .unwrap()
}

fn test_mcp_capability(server_name: &str, tool_name: &str) -> CapabilityAction {
    CapabilityAction {
        id: mcp_capability_id(server_name, tool_name),
        kind: "mcp_tool".to_string(),
        title: tool_name.to_string(),
        outcome: tool_name.to_string(),
        detail: tool_name.to_string(),
        source: "mcp".to_string(),
        available: true,
        availability: "available".to_string(),
        unavailable_reason: None,
        server_name: Some(server_name.to_string()),
        tool_name: Some(tool_name.to_string()),
        input_schema: Some(json!({"type": "object"})),
        output_schema: None,
        node_kind: Some("mcp".to_string()),
        node_template: None,
    }
}

fn workflow_ir_with_heavy_metadata() -> WorkflowIr {
    let enum_values = (0..200)
        .map(|index| format!("giant-enum-value-{index:03}"))
        .collect::<Vec<_>>();
    let long_description = "schema metadata ".repeat(200);
    serde_json::from_value(json!({
        "schemaVersion": "1.0.0",
        "workflowId": "wf-compiler-token-test",
        "workflowVersion": 1,
        "name": "Compiler token test",
        "description": "Compile one agent with a tool upstream",
        "compiler": { "model": "gemma-4-e2b-qat" },
        "nodes": [
            {
                "kind": "input",
                "id": "input",
                "label": "Input",
                "outputKey": "workflow.input",
                "inputSchema": {
                    "type": "object",
                    "description": long_description.clone(),
                    "properties": {
                        "request": {
                            "type": "string",
                            "enum": enum_values.clone()
                        }
                    }
                }
            },
            {
                "kind": "mcp_tool",
                "id": "tool",
                "label": "Search tool",
                "serverName": "search",
                "toolName": "query",
                "arguments": {
                    "query": "{{workflow.input}}",
                    "limit": 5
                },
                "inputSchema": {
                    "type": "object",
                    "description": long_description.clone(),
                    "properties": {
                        "query": {
                            "type": "string",
                            "enum": enum_values.clone()
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 50
                        }
                    }
                }
            },
            {
                "kind": "agent",
                "id": "agent",
                "label": "Summarize",
                "objective": "Summarize the tool result",
                "inputMappings": { "context": "{{nodes.tool.output}}" },
                "outputKey": "nodes.agent.output"
            },
            {
                "kind": "output",
                "id": "output",
                "label": "Output",
                "inputMapping": "{{nodes.agent.output}}",
                "outputSchema": {
                    "type": "object",
                    "description": long_description.clone(),
                    "properties": {
                        "summary": {
                            "type": "string",
                            "enum": enum_values
                        }
                    }
                }
            }
        ],
        "edges": [
            {
                "id": "e1",
                "sourceNodeId": "input",
                "sourcePort": "out",
                "targetNodeId": "tool"
            },
            {
                "id": "e2",
                "sourceNodeId": "tool",
                "sourcePort": "out",
                "targetNodeId": "agent"
            },
            {
                "id": "e3",
                "sourceNodeId": "agent",
                "sourcePort": "out",
                "targetNodeId": "output"
            }
        ]
    }))
    .unwrap()
}

fn compose_catalog(available: bool) -> CapabilityCatalog {
    CapabilityCatalog {
        version: WORKFLOW_CAPABILITY_CATALOG_VERSION.to_string(),
        authoring_enabled: true,
        generated_at_ms: 1,
        templates: Vec::new(),
        actions: vec![
            library_capability(
                "library:draft:create-draft",
                "agent",
                "Create Draft",
                "Draft a grounded response.",
                "draft",
            ),
            CapabilityAction {
                id: mcp_capability_id("search", "query"),
                kind: "mcp_tool".to_string(),
                title: "Search".to_string(),
                outcome: "Search connected content.".to_string(),
                detail: "Search connected content.".to_string(),
                source: "mcp".to_string(),
                available,
                availability: if available {
                    "available".to_string()
                } else {
                    "requires_connection".to_string()
                },
                unavailable_reason: (!available)
                    .then(|| "Connect search to use query.".to_string()),
                server_name: Some("search".to_string()),
                tool_name: Some("query".to_string()),
                input_schema: Some(json!({"type": "object"})),
                output_schema: None,
                node_kind: Some("mcp".to_string()),
                node_template: None,
            },
        ],
    }
}

fn compose_request(available: bool) -> ComposeWorkflowRequest {
    ComposeWorkflowRequest {
        prompt: "Search and summarize.".to_string(),
        capability_catalog: compose_catalog(available),
        project_id: None,
        workflow_id: Some("wf-compose-test".to_string()),
        name: Some("Compose Test".to_string()),
    }
}

fn mail_compose_request() -> ComposeWorkflowRequest {
    ComposeWorkflowRequest {
        prompt: "Read unread mail, draft replies, and ask me before opening any draft.".to_string(),
        project_id: None,
        capability_catalog: CapabilityCatalog {
            version: WORKFLOW_CAPABILITY_CATALOG_VERSION.to_string(),
            authoring_enabled: true,
            generated_at_ms: 1,
            templates: Vec::new(),
            actions: vec![
                library_capability(
                    "library:draft:create-draft",
                    "agent",
                    "Create Draft",
                    "Draft a grounded response.",
                    "draft",
                ),
                CapabilityAction {
                    id: mcp_capability_id("macos_applescript", "read_system_emails"),
                    kind: "mcp_tool".to_string(),
                    title: "Read macOS Mail".to_string(),
                    outcome: "Read recent Mail messages.".to_string(),
                    detail: "Read recent Mail messages.".to_string(),
                    source: "mcp".to_string(),
                    available: true,
                    availability: "available".to_string(),
                    unavailable_reason: None,
                    server_name: Some("macos_applescript".to_string()),
                    tool_name: Some("read_system_emails".to_string()),
                    input_schema: Some(json!({"type": "object"})),
                    output_schema: None,
                    node_kind: Some("mcp".to_string()),
                    node_template: None,
                },
                CapabilityAction {
                    id: mcp_capability_id("macos_applescript", "draft_system_email"),
                    kind: "mcp_tool".to_string(),
                    title: "Draft macOS Mail Email".to_string(),
                    outcome: "Open a visible draft in Apple Mail for user review.".to_string(),
                    detail: "Open a visible draft in Apple Mail for user review.".to_string(),
                    source: "mcp".to_string(),
                    available: true,
                    availability: "available".to_string(),
                    unavailable_reason: None,
                    server_name: Some("macos_applescript".to_string()),
                    tool_name: Some("draft_system_email".to_string()),
                    input_schema: Some(json!({"type": "object"})),
                    output_schema: None,
                    node_kind: Some("mcp".to_string()),
                    node_template: None,
                },
            ],
        },
        workflow_id: Some("wf-mail-regression".to_string()),
        name: Some("Mail Regression".to_string()),
    }
}

fn taskflow_compose_request() -> ComposeWorkflowRequest {
    let mut actions = vec![library_capability(
        "library:draft:create-draft",
        "agent",
        "Create Draft",
        "Draft a grounded response.",
        "draft",
    )];
    actions.extend(taskflow_native_capabilities().expect("native taskflow capabilities"));
    ComposeWorkflowRequest {
        prompt: "Read a project folder, summarize it, write a Markdown report, and preview it."
            .to_string(),
        project_id: None,
        capability_catalog: CapabilityCatalog {
            version: WORKFLOW_CAPABILITY_CATALOG_VERSION.to_string(),
            authoring_enabled: true,
            generated_at_ms: 1,
            templates: Vec::new(),
            actions,
        },
        workflow_id: Some("wf-taskflow-native".to_string()),
        name: Some("Taskflow Native".to_string()),
    }
}

fn mail_compose_request_with_report_tools() -> ComposeWorkflowRequest {
    let mut request = mail_compose_request();
    request
        .capability_catalog
        .actions
        .extend(taskflow_native_capabilities().expect("native taskflow capabilities"));
    request
}

fn calendar_compose_request() -> ComposeWorkflowRequest {
    ComposeWorkflowRequest {
        prompt: "Read tomorrow's calendar and draft a concise daily brief.".to_string(),
        project_id: None,
        capability_catalog: CapabilityCatalog {
            version: WORKFLOW_CAPABILITY_CATALOG_VERSION.to_string(),
            authoring_enabled: true,
            generated_at_ms: 1,
            templates: Vec::new(),
            actions: vec![
                library_capability(
                    "library:draft:create-draft",
                    "agent",
                    "Create Draft",
                    "Draft a grounded response.",
                    "draft",
                ),
                CapabilityAction {
                    id: mcp_capability_id("macos_applescript", "read_system_calendar"),
                    kind: "mcp_tool".to_string(),
                    title: "Read your Calendar".to_string(),
                    outcome: "Read upcoming events from Calendar on this Mac.".to_string(),
                    detail: "Read upcoming events from Calendar on this Mac.".to_string(),
                    source: "mcp".to_string(),
                    available: true,
                    availability: "available".to_string(),
                    unavailable_reason: None,
                    server_name: Some("macos_applescript".to_string()),
                    tool_name: Some("read_system_calendar".to_string()),
                    input_schema: Some(json!({"type": "object"})),
                    output_schema: None,
                    node_kind: Some("mcp".to_string()),
                    node_template: None,
                },
                CapabilityAction {
                    id: mcp_capability_id("macos_applescript", "draft_system_email"),
                    kind: "mcp_tool".to_string(),
                    title: "Open a Mail draft for review".to_string(),
                    outcome:
                        "Prepare a visible Apple Mail draft that you can review before sending."
                            .to_string(),
                    detail:
                        "Prepare a visible Apple Mail draft that you can review before sending."
                            .to_string(),
                    source: "mcp".to_string(),
                    available: true,
                    availability: "available".to_string(),
                    unavailable_reason: None,
                    server_name: Some("macos_applescript".to_string()),
                    tool_name: Some("draft_system_email".to_string()),
                    input_schema: Some(json!({"type": "object"})),
                    output_schema: None,
                    node_kind: Some("mcp".to_string()),
                    node_template: None,
                },
            ],
        },
        workflow_id: Some("wf-calendar-regression".to_string()),
        name: Some("Calendar Regression".to_string()),
    }
}

fn compose_output(workflow_ir: Value) -> String {
    json!({
        "status": "composed",
        "reason": "ok",
        "workflowIr": workflow_ir,
        "partialDraft": null,
        "missingCapabilities": []
    })
    .to_string()
}

fn repetition_collapse_error() -> GemmaError {
    GemmaError {
        code: LOCAL_MODEL_REPETITION_COLLAPSE_CODE,
        message: "The local model entered a repetition loop.".to_string(),
    }
}

mod capability;
mod compiler;
mod compose;
mod output_validation;
mod topology;
