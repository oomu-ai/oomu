use super::*;

#[test]
fn common_artifact_names_create_hello_world_in_the_requested_user_folder() {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .expect("HOME");
    for (label, format, folder) in [
        ("PDF document", "pdf", "Downloads"),
        ("Word doc", "docx", "Documents"),
        ("PowerPoint", "pptx", "Desktop"),
        ("Excel file", "xlsx", "Downloads"),
    ] {
        let objective =
            format!("Create a {label} in my {folder} folder with content “Hello World”.");
        let draft = generated_plan_from_text(
            objective,
            "model emitted malformed action-plan text".to_string(),
        );
        let expected = home.join(folder).join(format!("hello_world.{format}"));
        assert!(
            matches!(
                &draft.steps[0].tool,
                GeneratedToolDraft::RegisteredTaskTool { operation, arguments }
                    if operation == "create_file"
                        && arguments["file"]["format"] == format
                        && arguments["file"]["destinationPath"] == expected.to_string_lossy().as_ref()
                        && arguments["file"]["title"] == "hello_world"
                        && arguments["file"]["content"] == "Hello World"
            ),
            "{label}"
        );
    }
}

#[test]
fn common_artifact_without_a_destination_uses_downloads_and_an_obvious_name() {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .expect("HOME");
    let draft = generated_plan_from_text(
        "Create a Word doc with “Hello World”.".to_string(),
        "model emitted malformed action-plan text".to_string(),
    );
    let expected = home.join("Downloads/hello_world.docx");
    assert!(matches!(
        &draft.steps[0].tool,
        GeneratedToolDraft::RegisteredTaskTool { operation, arguments }
            if operation == "create_file"
                && arguments["file"]["format"] == "docx"
                && arguments["file"]["destinationPath"] == expected.to_string_lossy().as_ref()
                && arguments["file"]["content"] == "Hello World"
    ));
}

#[test]
fn topic_driven_presentation_preserves_the_native_presentation_tool() {
    let objective = "Create a presentation for me that explains the AI financial bubble in details. I want you to focus on the valuation and hype side of the risks.";
    let original = GeneratedActionPlanDraft {
        steps: vec![GeneratedPlanStepDraft {
            step: "Create the verified presentation review.".to_string(),
            tool: GeneratedToolDraft::RegisteredTaskTool {
                operation: "create_presentation".to_string(),
                arguments: serde_json::json!({"brief":{
                    "title":"The AI Financial Bubble",
                    "summary":"Valuation and hype risks",
                    "locale":"en-US"
                }}),
            },
            risk_level: GeneratedRiskLevel::High,
        }],
        exit_condition: "Return the result.".to_string(),
        generated_text: "model plan".to_string(),
        source: IntentSource::Cloud,
        degraded_reason: None,
    };

    let normalized = normalize_generated_plan_for_known_objectives(objective, original);
    assert!(matches!(normalized.source, IntentSource::Cloud));
    assert!(matches!(
        &normalized.steps[0].tool,
        GeneratedToolDraft::RegisteredTaskTool { operation, .. }
            if operation == "create_presentation"
    ));
    assert!(normalized
        .exit_condition
        .contains("native verification receipt"));
}

#[test]
fn topic_driven_presentation_recovers_malformed_plans_without_false_clarification() {
    for objective in [
        "create a PPTX presentation for me that explains the AI financial bubble in details",
        "I want you to investigate the AI bubble in detail. Let's create a presentation around it.",
    ] {
        let draft = generated_plan_from_text(
            objective.to_string(),
            "model emitted malformed action-plan text".to_string(),
        );
        assert!(
            matches!(draft.source, IntentSource::Deterministic),
            "{objective}"
        );
        assert_eq!(draft.steps.len(), 1, "{objective}");
        assert!(matches!(
            &draft.steps[0].tool,
            GeneratedToolDraft::RegisteredTaskTool { operation, arguments }
                if operation == "create_presentation"
                    && arguments.pointer("/brief/summary")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|summary| summary.contains("AI"))
        ));
    }
}

#[test]
fn exact_literal_pptx_creation_keeps_the_existing_verified_file_path() {
    let draft = generated_plan_from_text(
        "Create a PowerPoint presentation containing ‘Hello World’.".to_string(),
        "model emitted malformed action-plan text".to_string(),
    );
    assert!(matches!(
        &draft.steps[0].tool,
        GeneratedToolDraft::RegisteredTaskTool { operation, arguments }
            if operation == "create_file"
                && arguments.pointer("/file/format")
                    .and_then(serde_json::Value::as_str) == Some("pptx")
                && arguments.pointer("/file/content")
                    .and_then(serde_json::Value::as_str) == Some("Hello World")
    ));
}
