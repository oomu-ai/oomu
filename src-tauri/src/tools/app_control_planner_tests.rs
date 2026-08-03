use crate::{
    agentic_loop::{generated_step_to_step, step_to_request, Tool},
    gemma::{GeneratedPlanStepDraft, GeneratedRiskLevel, GeneratedToolDraft},
};
use serde_json::Value;

#[test]
fn planner_maps_every_app_control_phase_to_registered_task_tool() {
    for arguments in [
        serde_json::json!({"phase":"start","applicationId":"com.apple.mail"}),
        serde_json::json!({"phase":"observe","sessionId":"appcontrol_fixture"}),
        serde_json::json!({
            "phase":"execute",
            "sessionId":"appcontrol_fixture",
            "observationRevision":1,
            "action":{"kind":"focus","reference":"appref_fixture"},
            "expectedOutcome":"element_state"
        }),
        serde_json::json!({"phase":"stop","sessionId":"appcontrol_fixture"}),
    ] {
        let step = generated_step_to_step(GeneratedPlanStepDraft {
            step: "Use the active desktop application.".to_string(),
            tool: GeneratedToolDraft::RegisteredTaskTool {
                operation: "app_control".to_string(),
                arguments: arguments.clone(),
            },
            risk_level: GeneratedRiskLevel::Low,
        });
        assert!(matches!(step.tool, Tool::RegisteredTaskTool(_)));
        let request = step_to_request(&step);
        assert_eq!(request.kind, "app_control");
        assert_eq!(
            request
                .content
                .as_deref()
                .and_then(|value| serde_json::from_str::<Value>(value).ok()),
            Some(arguments)
        );
    }
}

#[test]
fn persisted_legacy_task_tool_kind_deserializes_into_generic_variant() {
    let tool = serde_json::from_value::<Tool>(serde_json::json!({
        "kind":"app_control",
        "operation":"app_control",
        "arguments":{"phase":"stop","sessionId":"appcontrol_fixture"}
    }))
    .unwrap();
    assert!(matches!(tool, Tool::RegisteredTaskTool(request)
        if request.operation == "app_control"));
}
