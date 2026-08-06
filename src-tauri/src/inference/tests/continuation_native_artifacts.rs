use super::*;

#[test]
fn connected_text_tools_cannot_intercept_native_rich_artifact_creation() {
    let decision = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "native_artifact_creation_filter".to_string(),
        reason: "native artifact".to_string(),
        matched_signals: vec!["native artifact creation request".to_string()],
        status_label: "Planning".to_string(),
    };
    let text_writer = ConversationalMcpToolCapability {
        server_name: "local_filesystem".to_string(),
        tool_name: "write_file".to_string(),
        description: "Write UTF-8 text in the connected sandbox.".to_string(),
        input_schema: serde_json::json!({}),
    };

    for prompt in [
        "Create a PDF document containing ‘Hello World’.",
        "Create a Word doc with ‘Hello World’.",
        "Create a PowerPoint presentation containing ‘Hello World’.",
        "Create an Excel spreadsheet containing ‘Hello World’.",
    ] {
        assert!(
            executable_intent_gate::requires_agentic_escalation(
                &decision,
                prompt,
                std::slice::from_ref(&text_writer),
            ),
            "{prompt} was intercepted by the text-only connected catalog"
        );
    }
}

#[test]
fn connected_catalog_cannot_intercept_deterministic_decision_pack() {
    let decision = crate::agentic_loop::ChatIntentRouteDecision {
        route: crate::agentic_loop::ChatIntentRoute::AgenticPlanner,
        requires_local_access: true,
        decision_source: "deterministic_decision_pack_filter".to_string(),
        reason: "deterministic decision pack".to_string(),
        matched_signals: vec!["evidence-bound decision pack".to_string()],
        status_label: "Planning".to_string(),
    };
    let arbitrary_capability = ConversationalMcpToolCapability {
        server_name: "connected_customer_service".to_string(),
        tool_name: "lookup_customer".to_string(),
        description: String::new(),
        input_schema: serde_json::json!({}),
    };

    assert!(executable_intent_gate::requires_agentic_escalation(
        &decision,
        "Prepare the complete supplier decision pack.",
        &[arbitrary_capability],
    ));
}
