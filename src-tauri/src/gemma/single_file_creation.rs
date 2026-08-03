pub(super) fn is_objective(lowered: &str) -> bool {
    super::file_creation_intent(lowered)
        && super::file_formats::requested_file_formats(lowered).len() == 1
        && ![
            "calendar",
            " mail",
            "email",
            "event",
            "meeting",
            "read ",
            "inspect ",
            "analyze ",
            "analyse ",
            "reconcile ",
            "research ",
            "search the web",
            "unsent",
        ]
        .iter()
        .any(|term| lowered.contains(term))
}

pub(super) fn preserves_deterministic_draft(draft: &super::GeneratedActionPlanDraft) -> bool {
    if !matches!(draft.source, super::IntentSource::Deterministic) {
        return false;
    }
    draft.steps.iter().any(|step| {
        let super::GeneratedToolDraft::RegisteredTaskTool {
            operation,
            arguments,
        } = &step.tool
        else {
            return false;
        };
        let Some(file) = arguments.get("file") else {
            return false;
        };
        let destination = file
            .get("destinationPath")
            .and_then(serde_json::Value::as_str)
            .map(std::path::Path::new);
        operation == "create_file"
            && destination.is_some_and(std::path::Path::is_absolute)
            && file
                .get("title")
                .and_then(serde_json::Value::as_str)
                .is_some()
            && file
                .get("content")
                .and_then(serde_json::Value::as_str)
                .is_some()
            && file
                .get("format")
                .and_then(serde_json::Value::as_str)
                .is_some()
    })
}

pub(super) fn preserves_grounded_draft(
    draft: &super::GeneratedActionPlanDraft,
    grounded: &super::GeneratedPlanStepDraft,
) -> bool {
    let super::GeneratedToolDraft::RegisteredTaskTool {
        arguments: expected,
        ..
    } = &grounded.tool
    else {
        return false;
    };
    draft.steps.iter().any(|step| {
        matches!(
            &step.tool,
            super::GeneratedToolDraft::RegisteredTaskTool { operation, arguments }
                if operation == "create_file" && arguments == expected
        )
    })
}
