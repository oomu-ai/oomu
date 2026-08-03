use super::*;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

#[test]
fn compose_repair_budget_stops_before_instruction_compiler_budget() {
    assert_eq!(COMPOSE_MAX_REPAIR_ATTEMPTS + 1, 2);
    assert_eq!(MAX_REPAIR_ATTEMPTS + 1, 3);
    assert!(COMPOSE_MAX_REPAIR_ATTEMPTS < MAX_REPAIR_ATTEMPTS);
}

#[test]
fn invalid_compose_json_returns_descriptive_parse_error_without_panic() {
    let error = parse_compose_output("{not valid json", &compose_request(true), 0, unix_time_ms())
        .unwrap_err();

    assert!(error.message.contains("Gemma did not return a JSON object"));
    assert!(error.partial_draft.is_none());
}

#[test]
fn compose_parse_rejects_missing_folder_input_for_clarification() {
    let output = compose_output(json!({
        "schemaVersion": "1.0.0",
        "workflowId": "model-tried-id",
        "workflowVersion": 1,
        "name": "Model tried name",
        "description": "Read a project folder, write a report, and preview it.",
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
                "kind": "mcp_tool",
                "id": "read-folder",
                "label": "Read Folder",
                "serverName": "taskflow_native",
                "toolName": "folder_read",
                "arguments": {}
            },
            {
                "kind": "agent",
                "id": "summary",
                "label": "Summary",
                "objective": "Summarize the folder contents.",
                "inputMappings": { "context": "{{nodes.read-folder.output}}" },
                "outputKey": "nodes.summary.output"
            },
            {
                "kind": "mcp_tool",
                "id": "write-report",
                "label": "Write Report",
                "serverName": "taskflow_native",
                "toolName": "write_markdown_report",
                "arguments": {}
            },
            {
                "kind": "mcp_tool",
                "id": "preview-report",
                "label": "Preview Report",
                "serverName": "taskflow_native",
                "toolName": "preview_report",
                "arguments": {}
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
                "targetNodeId": "read-folder"
            },
            {
                "id": "e2",
                "sourceNodeId": "read-folder",
                "sourcePort": "out",
                "targetNodeId": "summary"
            },
            {
                "id": "e3",
                "sourceNodeId": "summary",
                "sourcePort": "out",
                "targetNodeId": "write-report"
            },
            {
                "id": "e4",
                "sourceNodeId": "write-report",
                "sourcePort": "out",
                "targetNodeId": "preview-report"
            },
            {
                "id": "e5",
                "sourceNodeId": "preview-report",
                "sourcePort": "out",
                "targetNodeId": "output"
            }
        ]
    }));

    let error = parse_compose_output(&output, &taskflow_compose_request(), 0, unix_time_ms())
        .expect_err("folder_read without folderPath must request clarification");

    assert!(error.message.contains("needs a safe folderPath"));
    assert!(error.message.contains("path is missing"));
    let partial_draft = error.partial_draft.expect("partial draft is retained");
    let folder_node = partial_draft["nodes"]
        .as_array()
        .and_then(|nodes| {
            nodes
                .iter()
                .find(|node| node["toolName"] == json!("folder_read"))
        })
        .expect("folder_read remains visible for clarification");
    assert_eq!(folder_node["arguments"], json!({}));
}

#[test]
fn metaprompt_protects_permissions_and_requires_json_only() {
    assert!(WORKFLOW_COMPILER_SYSTEM_PROMPT.contains("Never bypass"));
    assert!(WORKFLOW_COMPILER_SYSTEM_PROMPT.contains("no instruction for any other node"));
    assert!(WORKFLOW_COMPILER_SYSTEM_PROMPT.contains("Return one compact JSON object"));
    assert_eq!(
        WORKFLOW_COMPILER_RUNTIME_MODEL_ID,
        "gemma-4-E2B-it-qat-q4_0-gguf"
    );
}

#[test]
fn compiler_request_uses_large_context_and_stable_session() {
    let request = compiler_infer_request("compile", "workflow-compiler:wf:1");
    assert_eq!(
        request.session_id.as_deref(),
        Some("workflow-compiler:wf:1")
    );
    assert!(request.deterministic);
    assert_eq!(request.context_size, Some(8_192));
    assert_eq!(request.max_tokens, Some(4_096));
    assert!(request.grammar.is_some());
}

#[test]
fn natural_language_composer_uses_a_bounded_generation_budget_and_shared_cancellation() {
    let cancellation = Arc::new(AtomicBool::new(false));
    let request = compose_infer_request("compose", "workflow-compose:wf:1", &cancellation);
    assert_eq!(request.context_size, Some(8_192));
    assert_eq!(request.max_tokens, Some(1_536));
    assert!(Arc::ptr_eq(&request.cancellation, &cancellation));
}

#[tokio::test]
async fn bounded_composer_cancels_a_worker_that_exceeds_its_deadline() {
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = Arc::clone(&cancellation);
    let (sender, mut receiver) = tokio::sync::oneshot::channel();
    thread::spawn(move || {
        while !worker_cancellation.load(Ordering::Acquire) {
            thread::yield_now();
        }
        let _ = sender.send(());
    });

    let result = composition_runtime::await_bounded_workflow_worker(
        &mut receiver,
        &cancellation,
        Duration::from_millis(50),
        Duration::from_millis(10),
    )
    .await;

    assert!(result.is_none());
    assert!(cancellation.load(Ordering::Acquire));
}

#[test]
fn compose_parse_returns_valid_grounded_ir() {
    let output = compose_output(json!({
        "schemaVersion": "1.0.0",
        "workflowId": "wf-compose-test",
        "workflowVersion": 1,
        "name": "Compose Test",
        "description": "Search and summarize",
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
                "kind": "mcp_tool",
                "id": "search",
                "label": "Search",
                "serverName": "search",
                "toolName": "query",
                "arguments": { "query": "{{workflow.input}}" }
            },
            {
                "kind": "agent",
                "id": "summary",
                "label": "Summary",
                "objective": "Summarize the search result.",
                "inputMappings": { "context": "{{nodes.search.output}}" },
                "outputKey": "nodes.summary.output"
            },
            {
                "kind": "output",
                "id": "output",
                "label": "Output",
                "inputMapping": "{{nodes.summary.output}}",
                "outputSchema": { "type": "object" }
            }
        ],
        "edges": [
            {
                "id": "e1",
                "sourceNodeId": "input",
                "sourcePort": "out",
                "targetNodeId": "search"
            },
            {
                "id": "e2",
                "sourceNodeId": "search",
                "sourcePort": "out",
                "targetNodeId": "summary"
            },
            {
                "id": "e3",
                "sourceNodeId": "summary",
                "sourcePort": "out",
                "targetNodeId": "output"
            }
        ]
    }));

    let response =
        parse_compose_output(&output, &compose_request(true), 0, unix_time_ms()).unwrap();

    assert_eq!(response.status, "composed");
    assert!(response.workflow_ir.is_some());
    let composed = response.workflow_ir.unwrap();
    assert_eq!(composed.compiler.model, WORKFLOW_COMPILER_MODEL);
    composed.validate().unwrap();
}

#[test]
fn edit_parse_preserves_workflow_identity_and_validates_ir() {
    let output = compose_output(json!({
        "schemaVersion": "1.0.0",
        "workflowId": "model-tried-to-change-id",
        "workflowVersion": 1,
        "name": "Model tried to rename",
        "description": "Search and summarize only on weekdays",
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
                "kind": "conditional",
                "id": "weekday-check",
                "label": "Weekday Check",
                "condition": "Today is a weekday.",
                "inputMapping": "{{workflow.input}}"
            },
            {
                "kind": "agent",
                "id": "summary",
                "label": "Summary",
                "objective": "Summarize when the weekday condition matches.",
                "inputMappings": { "context": "{{workflow.input}}" },
                "outputKey": "nodes.summary.output"
            },
            {
                "kind": "output",
                "id": "output",
                "label": "Output",
                "inputMapping": "{{nodes.summary.output}}",
                "outputSchema": { "type": "object" }
            },
            {
                "kind": "output",
                "id": "skipped-output",
                "label": "No action today",
                "inputMapping": "No action was needed.",
                "outputSchema": { "type": "object" }
            }
        ],
        "edges": [
            {
                "id": "e1",
                "sourceNodeId": "input",
                "sourcePort": "out",
                "targetNodeId": "weekday-check"
            },
            {
                "id": "e2",
                "sourceNodeId": "weekday-check",
                "sourcePort": "true",
                "targetNodeId": "summary"
            },
            {
                "id": "e3",
                "sourceNodeId": "weekday-check",
                "sourcePort": "false",
                "targetNodeId": "skipped-output"
            },
            {
                "id": "e4",
                "sourceNodeId": "summary",
                "sourcePort": "out",
                "targetNodeId": "output"
            }
        ]
    }));
    let request = ComposeWorkflowRequest {
        prompt: "Only on weekdays.".to_string(),
        capability_catalog: compose_catalog(true),
        project_id: None,
        workflow_id: Some("wf-existing".to_string()),
        name: Some("Existing Workflow".to_string()),
    };

    let response = parse_compose_output(&output, &request, 0, unix_time_ms()).unwrap();
    let edited = response.workflow_ir.unwrap();

    assert_eq!(edited.workflow_id, "wf-existing");
    assert_eq!(edited.name, "Existing Workflow");
    assert_eq!(edited.compiler.model, WORKFLOW_COMPILER_MODEL);
    edited.validate().unwrap();
    assert!(edited.edges.iter().any(|edge| edge.source_port == "false"));
}

#[test]
fn compose_parse_degrades_unavailable_mcp_to_connection_message() {
    let output = compose_output(json!({
        "schemaVersion": "1.0.0",
        "workflowId": "wf-compose-test",
        "workflowVersion": 1,
        "name": "Compose Test",
        "description": "Search and summarize",
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
                "kind": "mcp_tool",
                "id": "search",
                "label": "Search",
                "serverName": "search",
                "toolName": "query",
                "arguments": { "query": "{{workflow.input}}" }
            },
            {
                "kind": "output",
                "id": "output",
                "label": "Output",
                "inputMapping": "{{nodes.search.output}}",
                "outputSchema": { "type": "object" }
            }
        ],
        "edges": [
            {
                "id": "e1",
                "sourceNodeId": "input",
                "sourcePort": "out",
                "targetNodeId": "search"
            },
            {
                "id": "e2",
                "sourceNodeId": "search",
                "sourcePort": "out",
                "targetNodeId": "output"
            }
        ]
    }));

    let response =
        parse_compose_output(&output, &compose_request(false), 0, unix_time_ms()).unwrap();

    assert_eq!(response.status, "needs_connection");
    assert!(response.workflow_ir.is_none());
    assert_eq!(response.missing_capabilities.len(), 1);
    assert_eq!(response.missing_capabilities[0], "Search");
    assert_eq!(response.missing_capability_details.len(), 1);
    assert!(response.missing_capability_details[0]
        .reason
        .contains("Connect search"));
}

#[test]
fn compose_parse_rejects_placeholder_needs_connection() {
    let output = json!({
        "status": "needs_connection",
        "reason": "Connect Y to do X.",
        "workflowIr": null,
        "partialDraft": null,
        "missingCapabilities": ["Y"]
    })
    .to_string();

    let error =
        parse_compose_output(&output, &compose_request(true), 0, unix_time_ms()).unwrap_err();

    assert!(error.message.contains("placeholder"));
    assert!(error.missing_capabilities.is_empty());
}

#[test]
fn compose_parse_treats_model_failed_status_as_recoverable_attempt() {
    let output = json!({
        "status": "failed",
        "reason": "I cannot build that.",
        "workflowIr": null,
        "partialDraft": null,
        "missingCapabilities": []
    })
    .to_string();

    let error =
        parse_compose_output(&output, &calendar_compose_request(), 0, unix_time_ms()).unwrap_err();

    assert!(error.message.contains("failed compose response"));
    assert!(error.missing_capabilities.is_empty());
}

#[test]
fn compose_parse_resolves_real_missing_capability_details() {
    let output = json!({
        "status": "needs_connection",
        "reason": "Need search.",
        "workflowIr": null,
        "partialDraft": null,
        "missingCapabilities": ["Search"]
    })
    .to_string();

    let response =
        parse_compose_output(&output, &compose_request(false), 0, unix_time_ms()).unwrap();

    assert_eq!(response.status, "needs_connection");
    assert_eq!(response.missing_capabilities, vec!["Search"]);
    assert_eq!(
        response.missing_capability_details[0].id,
        "mcp:search:query"
    );
    assert!(response.reason.contains("Connect search"));
}

#[test]
fn composed_ir_serializes_without_null_optional_fields() {
    // Regression: the engine emitted `"targetPort": null` (and could emit a null
    // `systemTimeoutMs`) for optional IR fields. The frontend Zod schema treats those
    // as `.optional()`/`.default()`, which reject `null` — so every workflow composed
    // by the real desktop engine passed Rust validation but failed the save-time Zod
    // parse with "the steps were incomplete." Optional fields must be OMITTED, not null.
    let value = serde_json::to_value(workflow_ir()).unwrap();

    let edges = value["edges"].as_array().unwrap();
    assert!(!edges.is_empty());
    for edge in edges {
        assert!(
            edge.get("targetPort").map_or(true, |port| !port.is_null()),
            "edge serialized a null targetPort, which Zod rejects: {edge}"
        );
    }

    for node in value["nodes"].as_array().unwrap() {
        let object = node.as_object().unwrap();
        for key in ["systemTimeoutMs", "inputMapping", "workingDirectory"] {
            if let Some(field) = object.get(key) {
                assert!(
                    !field.is_null(),
                    "node field {key} serialized as null, which Zod rejects: {node}"
                );
            }
        }
    }
}

#[test]
fn compose_repetition_collapse_returns_truthful_failure_without_substitute_ir() {
    let request = mail_compose_request();
    let response =
        recover_compose_inference_error(repetition_collapse_error(), &request, 1, 1).unwrap();

    assert_eq!(response.status, "failed");
    assert_eq!(response.composed_by, "gemma");
    assert_eq!(response.attempts, 1);
    assert!(response.workflow_ir.is_none());
    assert!(response.partial_draft.is_none());
    assert_eq!(response.reason, compose_failed_reason());
}

#[test]
fn edit_repetition_collapse_returns_failed_response_instead_of_throwing() {
    let response = recover_edit_inference_error(repetition_collapse_error(), 1, 2).unwrap();

    assert_eq!(response.status, "failed");
    assert_eq!(response.composed_by, "gemma");
    assert_eq!(response.attempts, 2);
    assert!(response.workflow_ir.is_none());
    assert!(response.missing_capabilities.is_empty());
    assert_eq!(response.reason, compose_failed_reason());
}

#[test]
fn compiler_grammar_allows_empty_string_arrays() {
    assert!(compiler_output_grammar()
        .contains("strings ::= \"[\" ws (string (ws \",\" ws string)*)? ws \"]\""));
}
