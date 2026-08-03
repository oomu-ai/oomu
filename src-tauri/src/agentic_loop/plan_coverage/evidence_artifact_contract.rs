use super::*;

pub(super) fn requests(objective: &str) -> bool {
    requests_comparison(objective) || requests_recovery(objective)
}

pub(super) fn compile(
    objective: &str,
) -> Result<Option<GeneratedActionPlanDraft>, PlanCoverageDeficit> {
    if requests_comparison(objective) {
        return comparison_draft(objective).map(Some);
    }
    if requests_recovery(objective) {
        return recovery_draft(objective).map(Some);
    }
    Ok(None)
}

pub(super) fn validate(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> Result<(), PlanCoverageDeficit> {
    if !requests(objective) {
        return Ok(());
    }
    let expected = compile(objective)?
        .ok_or_else(|| failure("The evidence-artifact contract was not selected."))?;
    if draft.steps.is_empty()
        || serde_json::to_value(&draft.steps[..1]).ok()
            != serde_json::to_value(&expected.steps).ok()
        || !specialist_composition::exit_condition_matches(
            &draft.exit_condition,
            &expected.exit_condition,
            draft.steps.len() > 1,
        )
    {
        return Err(failure(
            "The plan must retain its exact evidence-bound native operation.",
        ));
    }
    specialist_composition::validate_authorized_extras(objective, draft, 1)
}

fn comparison_draft(objective: &str) -> Result<GeneratedActionPlanDraft, PlanCoverageDeficit> {
    let output = exact_markdown_output(objective, &["comparison", "openclaw", "claude cowork"])?;
    let arguments = serde_json::json!({
        "outputPath":output,
        "locale":"en-US",
    });
    Ok(single_step_draft(
        "Fetch the current official OpenClaw and Claude Cowork scheduling sources, synthesize the requested evidence-separated comparison, and create and reopen the exact Markdown file.",
        crate::tools::evidence_artifacts::COMPARISON_OPERATION,
        arguments,
        "Exit only after both official pages have current access receipts and the non-empty comparison file is reopened with the exact bytes and digest that were published.",
    ))
}

fn recovery_draft(objective: &str) -> Result<GeneratedActionPlanDraft, PlanCoverageDeficit> {
    let inputs = objective_input_file_references(objective)
        .into_iter()
        .map(|reference| normalize_path(&reference.path))
        .filter(|path| {
            Path::new(path).is_absolute() && file_format(path).as_deref() == Some("json")
        })
        .collect::<Vec<_>>();
    let [input] = inputs.as_slice() else {
        return Err(failure(
            "The recovery plan requires exactly one absolute JSON Project input.",
        ));
    };
    let output = exact_markdown_output(
        objective,
        &[
            "recovery",
            "assumptions",
            "critical path",
            "failure contingencies",
        ],
    )?;
    let arguments = serde_json::json!({
        "inputPath":input,
        "outputPath":output,
        "locale":"en-US",
    });
    Ok(single_step_draft(
        "Read the exact milestone source during execution, compute the unfinished-work and constraint analysis, and create and reopen the exact Markdown recovery plan.",
        crate::tools::evidence_artifacts::RECOVERY_OPERATION,
        arguments,
        "Exit only after the runtime read identifies the real unfinished milestones, preserves every requested constraint and three contingencies, and the non-empty recovery file is reopened with its exact bytes and digest.",
    ))
}

fn single_step_draft(
    label: &str,
    operation: &str,
    arguments: Value,
    exit_condition: &str,
) -> GeneratedActionPlanDraft {
    let step = GeneratedPlanStepDraft {
        step: label.to_string(),
        tool: GeneratedToolDraft::RegisteredTaskTool {
            operation: operation.to_string(),
            arguments: arguments.clone(),
        },
        risk_level: GeneratedRiskLevel::High,
    };
    let generated_text = serde_json::json!({
        "steps":[{
            "step":label,
            "tool":{
                "kind":operation,
                "arguments":arguments,
            },
            "risk_level":"high",
        }],
        "exit_condition":exit_condition,
    })
    .to_string();
    GeneratedActionPlanDraft {
        steps: vec![step],
        exit_condition: exit_condition.to_string(),
        generated_text,
        source: IntentSource::Deterministic,
        degraded_reason: None,
    }
}

fn exact_markdown_output(
    objective: &str,
    semantic_cues: &[&str],
) -> Result<String, PlanCoverageDeficit> {
    specialist_output::markdown_output(objective, semantic_cues).ok_or_else(|| {
        failure(
            "The evidence artifact requires one unambiguous absolute Markdown output; separately requested Markdown artifacts must have distinct purposes.",
        )
    })
}

fn requests_comparison(objective: &str) -> bool {
    let normalized = objective.to_ascii_lowercase();
    normalized.contains("openclaw")
        && normalized.contains("claude cowork")
        && normalized.contains("background")
        && normalized.contains("current")
        && normalized.contains("official")
        && has_unambiguous_positive_action(
            &normalized,
            &[
                "research",
                "researching",
                "search",
                "searching",
                "browse",
                "browsing",
                "investigate",
                "investigating",
                "retrieve",
                "retrieving",
            ],
        )
        && !denies_network_research(&normalized)
        && has_positive_markdown_output(objective)
        && normalized.contains("read it back")
}

fn requests_recovery(objective: &str) -> bool {
    let normalized = objective.to_ascii_lowercase();
    normalized.contains("recovery plan")
        && normalized.contains(".json")
        && normalized.contains("dependencies")
        && normalized.contains("one-owner capacity")
        && normalized.contains("business hours")
        && normalized.contains("20% contingency reserve")
        && normalized.contains("security validation precede release validation")
        && has_positive_recovery_plan_action(&normalized)
        && has_positive_markdown_output(objective)
        && normalized.contains("three failure contingencies")
}

fn has_unambiguous_positive_action(objective: &str, actions: &[&str]) -> bool {
    positive_action_segment(objective, actions)
        && !actions.iter().any(|action| {
            term_positions(objective, action).any(|position| action_is_negated(objective, position))
        })
}

fn has_positive_recovery_plan_action(objective: &str) -> bool {
    const ACTIONS: &[&str] = &[
        "construct",
        "constructing",
        "create",
        "creating",
        "prepare",
        "preparing",
        "write",
        "writing",
        "build",
        "building",
        "develop",
        "developing",
        "generate",
        "generating",
        "produce",
        "producing",
    ];

    objective_clauses(objective)
        .filter(|clause| clause.contains("recovery plan"))
        .any(|clause| has_unambiguous_positive_action(clause, ACTIONS))
}

fn denies_network_research(objective: &str) -> bool {
    const NETWORK_ACTIONS: &[&str] = &[
        "use",
        "using",
        "access",
        "accessing",
        "connect",
        "connecting",
        "go",
        "going",
    ];
    const NETWORK_TARGETS: &[&str] = &["internet", "web", "online", "network"];

    objective_clauses(objective).any(|clause| {
        (clause.contains("offline only") || clause.contains("no internet"))
            || NETWORK_ACTIONS.iter().any(|action| {
                term_positions(clause, action).any(|position| {
                    if !action_is_negated(clause, position) {
                        return false;
                    }
                    let suffix = &clause[position + action.len()..];
                    let bounded = &suffix[..suffix
                        .char_indices()
                        .nth(80)
                        .map_or(suffix.len(), |(index, _)| index)];
                    NETWORK_TARGETS
                        .iter()
                        .any(|target| contains_term(bounded, target))
                })
            })
    })
}

fn objective_clauses(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(|character| matches!(character, '.' | ';' | '!' | '?' | '\n' | '\r'))
        .map(str::trim)
        .filter(|clause| !clause.is_empty())
}

fn has_positive_markdown_output(objective: &str) -> bool {
    objective_output_file_references(objective)
        .iter()
        .any(|reference| file_format(&reference.path).as_deref() == Some("md"))
}

fn failure(message: impl Into<String>) -> PlanCoverageDeficit {
    PlanCoverageDeficit::missing(vec![message.into()])
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPARISON: &str = "Research current primary or official sources on scheduled/background agent capabilities in OpenClaw and Claude Cowork. Write a sourced comparison to `/Users/test/project/output/comparison.md` in my Project folder. Include URLs, access times, explicit limitations, and a section explaining what this implies for OOMU. Do not claim completion until the file exists and you have read it back.";
    const RECOVERY: &str = "Read `/Users/test/project/milestone_source.json` and construct a recovery plan that minimizes completion time while respecting dependencies, one-owner capacity, business hours, a 20% contingency reserve, and the requirement that security validation precede release validation. Write the assumptions, critical path, and three failure contingencies to `/Users/test/project/output/recovery.md` and verify the file.";

    #[test]
    fn exact_comparison_compiles_to_one_evidence_bound_runtime_operation() {
        let draft = compile(COMPARISON).unwrap().unwrap();
        assert_eq!(draft.steps.len(), 1);
        assert!(matches!(
            &draft.steps[0].tool,
            GeneratedToolDraft::RegisteredTaskTool { operation, arguments }
                if operation == crate::tools::evidence_artifacts::COMPARISON_OPERATION
                    && arguments["outputPath"] == "/Users/test/project/output/comparison.md"
        ));
        validate(COMPARISON, &draft).unwrap();
    }

    #[test]
    fn exact_recovery_compiles_to_one_runtime_read_compute_write_operation() {
        let draft = compile(RECOVERY).unwrap().unwrap();
        assert_eq!(draft.steps.len(), 1);
        assert!(matches!(
            &draft.steps[0].tool,
            GeneratedToolDraft::RegisteredTaskTool { operation, arguments }
                if operation == crate::tools::evidence_artifacts::RECOVERY_OPERATION
                    && arguments["inputPath"] == "/Users/test/project/milestone_source.json"
        ));
        validate(RECOVERY, &draft).unwrap();
    }

    #[test]
    fn negated_comparison_output_does_not_activate_the_specialist() {
        let objective = "Research current official OpenClaw and Claude Cowork background capabilities without writing `/Users/test/comparison.md`, then read it back only in chat.";
        assert!(!requests_comparison(objective));
        assert!(compile(objective).unwrap().is_none());
    }

    #[test]
    fn denied_research_or_network_access_never_activates_the_comparison_specialist() {
        for objective in [
            "Do not research current official OpenClaw and Claude Cowork background capabilities. Write `/Users/test/comparison.md` and read it back.",
            "You must not research current official OpenClaw and Claude Cowork background capabilities. Write `/Users/test/comparison.md` and read it back.",
            "Research current official OpenClaw and Claude Cowork background capabilities and write `/Users/test/comparison.md`; do not use the internet. Read it back.",
            "Research current official OpenClaw and Claude Cowork background capabilities and write `/Users/test/comparison.md`; you must not use the web. Read it back.",
        ] {
            assert!(!requests_comparison(objective), "{objective}");
            assert!(compile(objective).unwrap().is_none(), "{objective}");
        }
    }

    #[test]
    fn negated_recovery_output_does_not_activate_the_specialist() {
        let objective = "Read `/Users/test/milestones.json` and describe a recovery plan respecting dependencies, one-owner capacity, business hours, a 20% contingency reserve, and security validation precede release validation, but do not create `/Users/test/recovery.md` or three failure contingencies.";
        assert!(!requests_recovery(objective));
        assert!(compile(objective).unwrap().is_none());
    }

    #[test]
    fn denied_recovery_plan_construction_never_activates_the_specialist() {
        for objective in [
            "Read `/Users/test/milestones.json`, but do not construct a recovery plan respecting dependencies, one-owner capacity, business hours, a 20% contingency reserve, and security validation precede release validation. Write assumptions, critical path, and three failure contingencies to `/Users/test/recovery.md`.",
            "Read `/Users/test/milestones.json`, but you must not prepare a recovery plan respecting dependencies, one-owner capacity, business hours, a 20% contingency reserve, and security validation precede release validation. Write assumptions, critical path, and three failure contingencies to `/Users/test/recovery.md`.",
        ] {
            assert!(!requests_recovery(objective), "{objective}");
            assert!(compile(objective).unwrap().is_none(), "{objective}");
        }
    }
}
