pub(crate) fn is_objective(prompt: &str) -> bool {
    let lowered = &prompt.trim().to_ascii_lowercase();
    if uses_native_presentation_review(prompt) {
        return false;
    }
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

pub(crate) fn is_native_artifact_objective(prompt: &str) -> bool {
    let lowered = prompt.trim().to_ascii_lowercase();
    let formats = super::file_formats::requested_file_formats(&lowered);
    if formats.contains(&"pptx") {
        return is_presentation_creation_objective(&lowered);
    }
    is_presentation_creation_objective(&lowered)
        || (super::file_creation_intent(&lowered) && !formats.is_empty())
}

pub(super) fn is_presentation_creation_objective(prompt: &str) -> bool {
    let words = prompt
        .trim()
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let creation_verbs = [
        "assemble", "build", "create", "design", "generate", "make", "prepare", "produce",
    ];
    let presentation_nouns = ["deck", "powerpoint", "pptx", "presentation", "slides"];
    let competing_objects = [
        "analysis",
        "content",
        "document",
        "explanation",
        "notes",
        "outline",
        "report",
        "script",
        "software",
        "summary",
    ];

    words.iter().enumerate().any(|(noun_index, word)| {
        if !presentation_nouns.contains(&word.as_str()) {
            return false;
        }
        let window_start = noun_index.saturating_sub(7);
        let window = &words[window_start..noun_index];
        let Some(verb_offset) = window
            .iter()
            .rposition(|candidate| creation_verbs.contains(&candidate.as_str()))
        else {
            return false;
        };
        let between = &window[verb_offset + 1..];
        !between
            .iter()
            .any(|candidate| competing_objects.contains(&candidate.as_str()))
            && !creation_is_negated(&window[..=verb_offset])
    })
}

pub(super) fn uses_native_presentation_review(prompt: &str) -> bool {
    is_presentation_creation_objective(prompt)
        && requested_file_content(prompt).is_none()
        && has_presentation_content_direction(prompt)
}

pub(super) fn requested_file_content(objective: &str) -> Option<String> {
    for (open, close) in [('“', '”'), ('‘', '’'), ('"', '"'), ('\'', '\'')] {
        let mut search_from = 0;
        while let Some(relative_start) = objective[search_from..].find(open) {
            let start = search_from + relative_start + open.len_utf8();
            let Some(relative_end) = objective[start..].find(close) else {
                break;
            };
            let end = start + relative_end;
            let candidate = objective[start..end].trim();
            if !candidate.is_empty()
                && candidate.chars().count() <= 100_000
                && !candidate.starts_with('/')
                && !candidate.starts_with("~/")
            {
                return Some(candidate.to_string());
            }
            search_from = end + close.len_utf8();
        }
    }
    objective
        .to_ascii_lowercase()
        .contains("hello world")
        .then(|| "Hello World".to_string())
}

fn has_presentation_content_direction(prompt: &str) -> bool {
    let lowered = prompt.to_ascii_lowercase();
    [
        " about ",
        " around ",
        " based on ",
        " covering ",
        " explains ",
        " explaining ",
        " featuring ",
        " focus on ",
        " investigate ",
        " on the ",
        " titled ",
        " using ",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

fn creation_is_negated(words_through_verb: &[String]) -> bool {
    words_through_verb
        .iter()
        .rev()
        .skip(1)
        .take(3)
        .any(|word| matches!(word.as_str(), "never" | "no" | "not"))
}

pub(super) fn normalize_presentation_plan(
    objective: &str,
    mut draft: super::GeneratedActionPlanDraft,
) -> super::GeneratedActionPlanDraft {
    if draft.steps.iter().any(is_native_presentation_step) {
        draft.exit_condition = verified_presentation_exit_condition();
        draft.degraded_reason = None;
        return draft;
    }

    let was_degraded = matches!(draft.source, super::IntentSource::Degraded);
    if was_degraded {
        draft.steps.clear();
    } else {
        draft.steps.retain(|step| {
            !matches!(step.tool, super::GeneratedToolDraft::Unsupported { .. })
                && !is_generic_presentation_file_step(step)
        });
    }
    draft.steps.push(native_presentation_step(objective));
    draft.exit_condition = verified_presentation_exit_condition();
    draft.degraded_reason = None;
    if was_degraded || draft.steps.len() == 1 {
        draft.source = super::IntentSource::Deterministic;
    }
    draft
}

fn is_native_presentation_step(step: &super::GeneratedPlanStepDraft) -> bool {
    matches!(
        &step.tool,
        super::GeneratedToolDraft::RegisteredTaskTool { operation, .. }
            if operation == "create_presentation"
    )
}

fn is_generic_presentation_file_step(step: &super::GeneratedPlanStepDraft) -> bool {
    matches!(
        &step.tool,
        super::GeneratedToolDraft::RegisteredTaskTool { operation, arguments }
            if operation == "create_file"
                && arguments.pointer("/file/format").and_then(serde_json::Value::as_str)
                    == Some("pptx")
    )
}

fn native_presentation_step(objective: &str) -> super::GeneratedPlanStepDraft {
    super::GeneratedPlanStepDraft {
        step: "Create the requested presentation in OOMU's verified presentation review."
            .to_string(),
        tool: super::GeneratedToolDraft::RegisteredTaskTool {
            operation: "create_presentation".to_string(),
            arguments: serde_json::json!({"brief":{
                "title": presentation_title(objective),
                "summary": bounded_presentation_summary(objective),
                "locale":"en-US"
            }}),
        },
        risk_level: super::GeneratedRiskLevel::High,
    }
}

fn presentation_title(objective: &str) -> String {
    let compact = objective.split_whitespace().collect::<Vec<_>>().join(" ");
    let lowered = compact.to_ascii_lowercase();
    for marker in ["that explains ", "explaining ", "investigate ", "about "] {
        let Some(start) = lowered.find(marker).map(|index| index + marker.len()) else {
            continue;
        };
        let candidate = compact[start..]
            .split(['.', '!', '?'])
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches(" in detail")
            .trim_end_matches(" in details")
            .trim();
        if !candidate.is_empty()
            && !matches!(candidate.to_ascii_lowercase().as_str(), "it" | "this")
        {
            return candidate.chars().take(120).collect();
        }
    }
    "Presentation".to_string()
}

fn bounded_presentation_summary(objective: &str) -> String {
    objective
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(2_000)
        .collect()
}

fn verified_presentation_exit_condition() -> String {
    "Exit only after the Project-bound presentation review exists and its native verification receipt is recorded."
        .to_string()
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
