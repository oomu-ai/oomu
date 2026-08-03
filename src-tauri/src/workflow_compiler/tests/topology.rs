use super::*;

#[test]
fn topology_rejects_report_preview_without_upstream_writer() {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
        "schemaVersion": "1.0.0",
        "workflowId": "wf-preview-gap",
        "workflowVersion": 1,
        "name": "Preview Gap",
        "description": "Preview a report without writing one first.",
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
                "id": "summary",
                "label": "Summary",
                "objective": "Summarize the request.",
                "inputMappings": { "context": "{{workflow.input}}" },
                "outputKey": "nodes.summary.output"
            },
            {
                "kind": "mcp_tool",
                "id": "preview-report",
                "label": "Preview Report",
                "serverName": "taskflow_native",
                "toolName": "preview_report",
                "arguments": { "reportPath": "workspace/report.md" }
            },
            {
                "kind": "output",
                "id": "output",
                "label": "Output",
                "inputMapping": "{{nodes.preview-report.output}}",
                "outputSchema": { "type": "object" }
            }
        ],
        "edges": [
            {
                "id": "e1",
                "sourceNodeId": "input",
                "sourcePort": "out",
                "targetNodeId": "summary"
            },
            {
                "id": "e2",
                "sourceNodeId": "summary",
                "sourcePort": "out",
                "targetNodeId": "preview-report"
            },
            {
                "id": "e3",
                "sourceNodeId": "preview-report",
                "sourcePort": "out",
                "targetNodeId": "output"
            }
        ]
    }))
    .unwrap();
    workflow_ir.validate().unwrap();

    let error = validate_workflow_ir_topology(&workflow_ir).unwrap_err();
    assert_eq!(error.code, WORKFLOW_TOPOLOGY_MISSING_REPORT_WRITER_CODE);
    assert!(error.message.contains("save the report to disk first"));
}

#[test]
fn topology_allows_report_preview_after_upstream_writer() {
    let response = parse_compose_output(
        &compose_output(json!({
            "schemaVersion": "1.0.0",
            "workflowId": "wf-preview-ok",
            "workflowVersion": 1,
            "name": "Preview OK",
            "description": "Write a report before previewing it.",
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
                    "id": "summary",
                    "label": "Summary",
                    "objective": "Summarize the request.",
                    "inputMappings": { "context": "{{workflow.input}}" },
                    "outputKey": "nodes.summary.output"
                },
                {
                    "kind": "mcp_tool",
                    "id": "write-report",
                    "label": "Write Report",
                    "serverName": "taskflow_native",
                    "toolName": "write_markdown_report",
                    "arguments": {
                        "reportPath": "workspace/report.md",
                        "content": "{{nodes.summary.output}}"
                    }
                },
                {
                    "kind": "mcp_tool",
                    "id": "preview-report",
                    "label": "Preview Report",
                    "serverName": "taskflow_native",
                    "toolName": "preview_report",
                    "arguments": { "reportPath": "workspace/report.md" }
                },
                {
                    "kind": "output",
                    "id": "output",
                    "label": "Output",
                    "inputMapping": "{{nodes.preview-report.output}}",
                    "outputSchema": { "type": "object" }
                }
            ],
            "edges": [
                {
                    "id": "e1",
                    "sourceNodeId": "input",
                    "sourcePort": "out",
                    "targetNodeId": "summary"
                },
                {
                    "id": "e2",
                    "sourceNodeId": "summary",
                    "sourcePort": "out",
                    "targetNodeId": "write-report"
                },
                {
                    "id": "e3",
                    "sourceNodeId": "write-report",
                    "sourcePort": "out",
                    "targetNodeId": "preview-report"
                },
                {
                    "id": "e4",
                    "sourceNodeId": "preview-report",
                    "sourcePort": "out",
                    "targetNodeId": "output"
                }
            ]
        })),
        &taskflow_compose_request(),
        0,
        unix_time_ms(),
    )
    .unwrap();

    let workflow_ir = response.workflow_ir.unwrap();
    validate_workflow_ir_topology(&workflow_ir).unwrap();
}

#[test]
fn topology_rejects_folder_scan_outside_sandbox() {
    let mut workflow_ir = workflow_ir();
    workflow_ir.nodes.insert(
        1,
        WorkflowNode::McpTool(McpToolNode {
            id: "folder-read".to_string(),
            label: "Folder Read".to_string(),
            server_name: TASKFLOW_NATIVE_SERVER.to_string(),
            tool_name: "folder_read".to_string(),
            arguments: json!({"folderPath": "../outside"}),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: Some(10_000),
        }),
    );
    workflow_ir.edges = vec![
        workflow_edge("input", "out", "folder-read"),
        workflow_edge("folder-read", "out", "agent"),
        workflow_edge("agent", "out", "output"),
    ];
    workflow_ir.validate().unwrap();

    let error = validate_workflow_ir_topology(&workflow_ir).unwrap_err();
    assert_eq!(error.code, WORKFLOW_TOPOLOGY_INVALID_SANDBOX_PATH_CODE);
    assert!(error.message.contains("Parent-directory traversal"));
}

#[test]
fn topology_rejects_indexed_collection_without_empty_guard() {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion": "1.0.0",
            "workflowId": "wf-unguarded-collection",
            "workflowVersion": 1,
            "name": "Unguarded collection",
            "description": "Unsafe indexed collection access.",
            "compiler": { "model": "gemma-4-e2b-qat" },
            "nodes": [
                {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
                {"kind":"mcp_tool","id":"read-mail","label":"Read mail","serverName":"macos_applescript","toolName":"read_system_emails","arguments":{}},
                {"kind":"mcp_tool","id":"draft","label":"Draft reply","serverName":"macos_applescript","toolName":"draft_system_email","arguments":{"to":"{{nodes.read-mail.output.data.structuredContent.emails.0.sender}}"}},
                {"kind":"output","id":"output","label":"Output","inputMapping":"{{nodes.draft.output}}","outputSchema":{"type":"object"}}
            ],
            "edges": [
                {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read-mail"},
                {"id":"e2","sourceNodeId":"read-mail","sourcePort":"out","targetNodeId":"draft"},
                {"id":"e3","sourceNodeId":"draft","sourcePort":"out","targetNodeId":"output"}
            ]
        })).unwrap();
    workflow_ir.validate().unwrap();

    let error = validate_workflow_ir_topology(&workflow_ir).unwrap_err();
    assert_eq!(error.code, WORKFLOW_TOPOLOGY_UNSAFE_COLLECTION_CODE);
}

#[test]
fn topology_rejects_shorthand_indexed_collection_without_empty_guard() {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion":"1.0.0","workflowId":"wf-shorthand-collection","workflowVersion":1,
            "name":"Shorthand collection","description":"Shorthand must not bypass safety.",
            "compiler":{"model":"gemma-4-e2b-qat"},
            "nodes":[
                {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
                {"kind":"mcp_tool","id":"read","label":"Read","serverName":"arbitrary_mcp","toolName":"read_items","arguments":{},"outputSchema":{"type":"object","x-oomu-result-contract":{"kind":"collection","path":"/structuredContent/items","emptyIsSuccess":true}}},
                {"kind":"mcp_tool","id":"write","label":"Write","serverName":"arbitrary_mcp","toolName":"write_item","arguments":{"value":"{{read.output.data.structuredContent.items.0.value}}"}},
                {"kind":"output","id":"output","label":"Output","inputMapping":"{{write.output}}","outputSchema":{"type":"object"}}
            ],
            "edges":[
                {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read"},
                {"id":"e2","sourceNodeId":"read","sourcePort":"out","targetNodeId":"write"},
                {"id":"e3","sourceNodeId":"write","sourcePort":"out","targetNodeId":"output"}
            ]
        })).unwrap();
    workflow_ir.validate().unwrap();

    let error = validate_workflow_ir_topology(&workflow_ir).unwrap_err();
    assert_eq!(error.code, WORKFLOW_TOPOLOGY_UNSAFE_COLLECTION_CODE);
}

#[test]
fn topology_requires_declared_collection_guard_before_model_work() {
    let output_schema = json!({
        "type": "object",
        "x-oomu-result-contract": {
            "kind": "collection",
            "path": "/structuredContent/items",
            "emptyIsSuccess": true
        }
    });
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion":"1.0.0","workflowId":"wf-declared-collection","workflowVersion":1,
            "name":"Declared collection","description":"Guard a declared collection.",
            "compiler":{"model":"gemma-4-e2b-qat"},
            "nodes":[
                {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
                {"kind":"mcp_tool","id":"read","label":"Read","serverName":"arbitrary_mcp","toolName":"read_items","arguments":{},"outputSchema":output_schema},
                {"kind":"agent","id":"summarize","label":"Summarize","objective":"Summarize items.","inputMappings":{"context":"{{nodes.read.output}}"},"outputKey":"nodes.summarize.output"},
                {"kind":"output","id":"output","label":"Output","inputMapping":"{{nodes.summarize.output}}","outputSchema":{"type":"object"}}
            ],
            "edges":[
                {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read"},
                {"id":"e2","sourceNodeId":"read","sourcePort":"out","targetNodeId":"summarize"},
                {"id":"e3","sourceNodeId":"summarize","sourcePort":"out","targetNodeId":"output"}
            ]
        })).unwrap();
    workflow_ir.validate().unwrap();
    let error = validate_workflow_ir_topology(&workflow_ir).unwrap_err();
    assert_eq!(error.code, WORKFLOW_TOPOLOGY_UNSAFE_COLLECTION_CODE);
}

#[test]
fn topology_accepts_declared_collection_with_empty_terminal() {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion":"1.0.0","workflowId":"wf-guarded-collection","workflowVersion":1,
            "name":"Guarded collection","description":"Stop cleanly when empty.",
            "compiler":{"model":"gemma-4-e2b-qat"},
            "nodes":[
                {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
                {"kind":"mcp_tool","id":"read","label":"Read","serverName":"arbitrary_mcp","toolName":"read_items","arguments":{},"outputSchema":{"type":"object","x-oomu-result-contract":{"kind":"collection","path":"/structuredContent/items","emptyIsSuccess":true}}},
                {"kind":"conditional","id":"has-items","label":"Check for items","condition":"$ != []","inputMapping":"{{nodes.read.output.data.structuredContent.items}}"},
                {"kind":"agent","id":"summarize","label":"Summarize","objective":"Summarize items.","inputMappings":{"context":"{{nodes.read.output}}"},"outputKey":"nodes.summarize.output"},
                {"kind":"output","id":"output","label":"Output","inputMapping":"{{nodes.summarize.output}}","outputSchema":{"type":"object"}},
                {"kind":"output","id":"empty-output","label":"Nothing found","inputMapping":"{{nodes.read.output.data.structuredContent.items}}","outputSchema":{"type":"array"},"completionKind":"empty_collection"}
            ],
            "edges":[
                {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read"},
                {"id":"e2","sourceNodeId":"read","sourcePort":"out","targetNodeId":"has-items"},
                {"id":"e3","sourceNodeId":"has-items","sourcePort":"true","targetNodeId":"summarize"},
                {"id":"e4","sourceNodeId":"has-items","sourcePort":"false","targetNodeId":"empty-output"},
                {"id":"e5","sourceNodeId":"summarize","sourcePort":"out","targetNodeId":"output"}
            ]
        })).unwrap();
    workflow_ir.validate().unwrap();
    validate_workflow_ir_topology(&workflow_ir).unwrap();
}

#[test]
fn topology_accepts_multiple_collections_when_any_nonempty_result_can_continue() {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion":"1.0.0","workflowId":"wf-multi-collection","workflowVersion":1,
            "name":"Multiple collections","description":"Continue when either source has work.",
            "compiler":{"model":"gemma-4-e2b-qat"},
            "nodes":[
                {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
                {"kind":"mcp_tool","id":"read-mail","label":"Read mail","serverName":"arbitrary_mcp","toolName":"read_mail","arguments":{},"outputSchema":{"type":"object","x-oomu-result-contract":{"kind":"collection","path":"/structuredContent/emails","emptyIsSuccess":true}}},
                {"kind":"mcp_tool","id":"read-reminders","label":"Read reminders","serverName":"arbitrary_mcp","toolName":"read_reminders","arguments":{},"outputSchema":{"type":"object","x-oomu-result-contract":{"kind":"collection","path":"/structuredContent/reminders","emptyIsSuccess":true}}},
                {"kind":"conditional","id":"has-mail","label":"Check mail","condition":"$ != []","inputMapping":"{{nodes.read-mail.output.data.structuredContent.emails}}"},
                {"kind":"conditional","id":"has-reminders","label":"Check reminders","condition":"$ != []","inputMapping":"{{nodes.read-reminders.output.data.structuredContent.reminders}}"},
                {"kind":"agent","id":"summarize","label":"Summarize","objective":"Summarize available work.","inputMappings":{"mail":"{{nodes.read-mail.output}}","reminders":"{{nodes.read-reminders.output}}"},"outputKey":"nodes.summarize.output"},
                {"kind":"output","id":"output","label":"Output","inputMapping":"{{nodes.summarize.output}}","outputSchema":{"type":"object"}},
                {"kind":"output","id":"empty-output","label":"Nothing found","inputMapping":"{{nodes.read-reminders.output.data.structuredContent.reminders}}","outputSchema":{"type":"array"},"completionKind":"empty_collection"}
            ],
            "edges":[
                {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read-mail"},
                {"id":"e2","sourceNodeId":"read-mail","sourcePort":"out","targetNodeId":"read-reminders"},
                {"id":"e3","sourceNodeId":"read-reminders","sourcePort":"out","targetNodeId":"has-mail"},
                {"id":"e4","sourceNodeId":"has-mail","sourcePort":"true","targetNodeId":"summarize"},
                {"id":"e5","sourceNodeId":"has-mail","sourcePort":"false","targetNodeId":"has-reminders"},
                {"id":"e6","sourceNodeId":"has-reminders","sourcePort":"true","targetNodeId":"summarize"},
                {"id":"e7","sourceNodeId":"has-reminders","sourcePort":"false","targetNodeId":"empty-output"},
                {"id":"e8","sourceNodeId":"summarize","sourcePort":"out","targetNodeId":"output"}
            ]
        })).unwrap();
    workflow_ir.validate().unwrap();

    validate_workflow_ir_topology(&workflow_ir).unwrap();
}

#[test]
fn topology_rejects_side_effects_before_the_empty_terminal() {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion":"1.0.0","workflowId":"wf-unsafe-empty-branch","workflowVersion":1,
            "name":"Unsafe empty branch","description":"Empty branch must stop immediately.",
            "compiler":{"model":"gemma-4-e2b-qat"},
            "nodes":[
                {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
                {"kind":"mcp_tool","id":"read","label":"Read","serverName":"arbitrary_mcp","toolName":"read_items","arguments":{},"outputSchema":{"type":"object","x-oomu-result-contract":{"kind":"collection","path":"/structuredContent/items","emptyIsSuccess":true}}},
                {"kind":"conditional","id":"has-items","label":"Check","condition":"$ != []","inputMapping":"{{nodes.read.output.data.structuredContent.items}}"},
                {"kind":"mcp_tool","id":"write","label":"Write","serverName":"arbitrary_mcp","toolName":"write_item","arguments":{"value":"{{nodes.read.output.data.structuredContent.items.0.value}}"}},
                {"kind":"agent","id":"empty-model","label":"Model on empty","objective":"Do work that should never run.","inputMappings":{"context":"{{workflow.input}}"},"outputKey":"nodes.empty-model.output"},
                {"kind":"output","id":"output","label":"Output","inputMapping":"{{nodes.write.output}}","outputSchema":{"type":"object"}},
                {"kind":"output","id":"empty-output","label":"Nothing found","inputMapping":"{{nodes.read.output.data.structuredContent.items}}","outputSchema":{"type":"array"},"completionKind":"empty_collection"}
            ],
            "edges":[
                {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read"},
                {"id":"e2","sourceNodeId":"read","sourcePort":"out","targetNodeId":"has-items"},
                {"id":"e3","sourceNodeId":"has-items","sourcePort":"true","targetNodeId":"write"},
                {"id":"e4","sourceNodeId":"has-items","sourcePort":"false","targetNodeId":"empty-model"},
                {"id":"e5","sourceNodeId":"write","sourcePort":"out","targetNodeId":"output"},
                {"id":"e6","sourceNodeId":"empty-model","sourcePort":"out","targetNodeId":"empty-output"}
            ]
        })).unwrap();
    workflow_ir.validate().unwrap();

    let error = validate_workflow_ir_topology(&workflow_ir).unwrap_err();
    assert_eq!(error.code, WORKFLOW_TOPOLOGY_UNSAFE_COLLECTION_CODE);
}

#[test]
fn topology_rejects_empty_branch_side_effect_without_a_collection_consumer() {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion":"1.0.0","workflowId":"wf-empty-side-effect-only","workflowVersion":1,
            "name":"Unsafe empty branch","description":"Empty branches must stop before unrelated work.",
            "compiler":{"model":"gemma-4-e2b-qat"},
            "nodes":[
                {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
                {"kind":"mcp_tool","id":"read","label":"Read","serverName":"arbitrary_mcp","toolName":"read_items","arguments":{},"outputSchema":{"type":"object","x-oomu-result-contract":{"kind":"collection","path":"/structuredContent/items","emptyIsSuccess":true}}},
                {"kind":"conditional","id":"has-items","label":"Check","condition":"$ != []","inputMapping":"{{nodes.read.output.data.structuredContent.items}}"},
                {"kind":"agent","id":"empty-model","label":"Model on empty","objective":"Run unrelated work.","inputMappings":{"context":"{{workflow.input}}"},"outputKey":"nodes.empty-model.output"},
                {"kind":"output","id":"output","label":"Output","inputMapping":"{{nodes.read.output}}","outputSchema":{"type":"object"}},
                {"kind":"output","id":"empty-output","label":"Nothing found","inputMapping":"{{nodes.read.output.data.structuredContent.items}}","outputSchema":{"type":"array"},"completionKind":"empty_collection"}
            ],
            "edges":[
                {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read"},
                {"id":"e2","sourceNodeId":"read","sourcePort":"out","targetNodeId":"has-items"},
                {"id":"e3","sourceNodeId":"has-items","sourcePort":"true","targetNodeId":"output"},
                {"id":"e4","sourceNodeId":"has-items","sourcePort":"false","targetNodeId":"empty-model"},
                {"id":"e5","sourceNodeId":"empty-model","sourcePort":"out","targetNodeId":"empty-output"}
            ]
        }))
        .unwrap();
    workflow_ir.validate().unwrap();

    let error = validate_workflow_ir_topology(&workflow_ir).unwrap_err();
    assert_eq!(error.code, WORKFLOW_TOPOLOGY_UNSAFE_COLLECTION_CODE);
    assert!(error.message.contains("before running a model"));
}

#[test]
fn topology_rejects_nested_array_as_whole_workflow_empty() {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion":"1.0.0","workflowId":"wf-nested-empty","workflowVersion":1,
            "name":"Nested empty","description":"Only the declared primary collection may complete empty.",
            "compiler":{"model":"gemma-4-e2b-qat"},
            "nodes":[
                {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
                {"kind":"mcp_tool","id":"read","label":"Read contacts","serverName":"arbitrary_mcp","toolName":"read_contacts","arguments":{},"outputSchema":{"type":"object","x-oomu-result-contract":{"kind":"collection","path":"/structuredContent/contacts","emptyIsSuccess":true}}},
                {"kind":"output","id":"empty-output","label":"Nothing found","inputMapping":"{{nodes.read.output.data.structuredContent.contacts.0.emails}}","outputSchema":{"type":"array"},"completionKind":"empty_collection"}
            ],
            "edges":[
                {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read"},
                {"id":"e2","sourceNodeId":"read","sourcePort":"out","targetNodeId":"empty-output"}
            ]
        })).unwrap();
    workflow_ir.validate().unwrap();

    let error = validate_workflow_ir_topology(&workflow_ir).unwrap_err();
    assert_eq!(error.code, WORKFLOW_TOPOLOGY_UNSAFE_COLLECTION_CODE);
}

#[test]
fn topology_rejects_reference_to_a_node_not_run_on_every_path() {
    let workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion": "1.0.0",
            "workflowId": "wf-branch-reference",
            "workflowVersion": 1,
            "name": "Unsafe branch reference",
            "description": "Output incorrectly relies on the true branch.",
            "compiler": { "model": "gemma-4-e2b-qat" },
            "nodes": [
                {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
                {"kind":"conditional","id":"condition","label":"Condition","condition":"true"},
                {"kind":"agent","id":"summary","label":"Summary","objective":"Summarize.","inputMappings":{"context":"{{workflow.input}}"},"outputKey":"nodes.summary.output"},
                {"kind":"output","id":"output","label":"Output","inputMapping":"{{nodes.summary.output}}","outputSchema":{"type":"object"}}
            ],
            "edges": [
                {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"condition"},
                {"id":"e2","sourceNodeId":"condition","sourcePort":"true","targetNodeId":"summary"},
                {"id":"e3","sourceNodeId":"condition","sourcePort":"false","targetNodeId":"output"},
                {"id":"e4","sourceNodeId":"summary","sourcePort":"out","targetNodeId":"output"}
            ]
        })).unwrap();
    workflow_ir.validate().unwrap();

    let error = validate_workflow_ir_topology(&workflow_ir).unwrap_err();
    assert_eq!(error.code, WORKFLOW_TOPOLOGY_UNSAFE_REFERENCE_CODE);
}
