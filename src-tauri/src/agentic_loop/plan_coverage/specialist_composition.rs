use super::*;

const COMPOUND_EXIT_SUFFIX: &str =
    "Also complete and verify every separately requested action before exiting.";

pub(super) fn needs_model_composition(objective: &str, draft: &GeneratedActionPlanDraft) -> bool {
    is_deterministic_specialist(draft) && !uncovered_requirements(objective, draft).is_empty()
}

pub(super) fn validation_objective(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> Option<String> {
    let missing = uncovered_requirements(objective, draft);
    (!missing.is_empty()).then(|| {
        format!(
            "Complete only these separately requested additions; do not repeat work already owned by the deterministic specialist:\n{}",
            missing
                .iter()
                .map(Requirement::planner_clause)
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

pub(super) fn compose(
    objective: &str,
    mut base: GeneratedActionPlanDraft,
    candidate: GeneratedActionPlanDraft,
) -> Result<GeneratedActionPlanDraft, PlanCoverageDeficit> {
    let core_len = base.steps.len();
    let mut missing = uncovered_requirement_labels(objective, &base);
    for step in candidate.steps {
        if is_specialist_core_step(&step) {
            continue;
        }
        let mut trial = base.clone();
        trial.steps.push(step.clone());
        let next_missing = uncovered_requirement_labels(objective, &trial);
        if next_missing.len() < missing.len() {
            base.steps.push(step);
            missing = next_missing;
        }
    }
    if !missing.is_empty() {
        return Err(PlanCoverageDeficit::missing(missing));
    }
    if base.steps.len() == core_len {
        return Err(PlanCoverageDeficit::missing(vec![
            "separately requested compound action".to_string(),
        ]));
    }
    base.exit_condition = compound_exit_condition(&base.exit_condition);
    base.generated_text = generated_text(&base)?;
    validate_objective_coverage(objective, &base)?;
    Ok(base)
}

pub(super) fn validate_authorized_extras(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
    core_len: usize,
) -> Result<(), PlanCoverageDeficit> {
    if draft.steps.len() <= core_len {
        return Ok(());
    }
    let mut prefix = draft.clone();
    prefix.steps.truncate(core_len);
    let mut missing = uncovered_requirement_labels(objective, &prefix);
    for step in &draft.steps[core_len..] {
        if is_specialist_core_step(step) {
            return Err(PlanCoverageDeficit::missing(vec![
                "duplicate deterministic specialist operation".to_string(),
            ]));
        }
        let mut trial = prefix.clone();
        trial.steps.push(step.clone());
        let next_missing = uncovered_requirement_labels(objective, &trial);
        if next_missing.len() >= missing.len() {
            return Err(PlanCoverageDeficit::missing(vec![format!(
                "unrequested compound step '{}'",
                step.step.trim()
            )]));
        }
        prefix.steps.push(step.clone());
        missing = next_missing;
    }
    Ok(())
}

pub(super) fn exit_condition_matches(actual: &str, base: &str, has_extras: bool) -> bool {
    if has_extras {
        actual == compound_exit_condition(base)
    } else {
        actual == base
    }
}

fn compound_exit_condition(base: &str) -> String {
    format!("{} {}", base.trim(), COMPOUND_EXIT_SUFFIX)
}

fn is_deterministic_specialist(draft: &GeneratedActionPlanDraft) -> bool {
    draft.steps.first().is_some_and(is_specialist_core_step)
}

fn is_specialist_core_step(step: &GeneratedPlanStepDraft) -> bool {
    matches!(
        &step.tool,
        GeneratedToolDraft::RegisteredTaskTool { operation, .. }
            if matches!(
                normalized_operation(operation).as_str(),
                "prepare_background_agent_comparison"
                    | "prepare_milestone_constraint_recovery_plan"
                    | "create_decision_pack"
                    | "create_conflict_free_calendar_event"
                    | "draft_decision_pack_email"
                    | "prepare_release_recovery_agenda"
                    | "create_release_recovery_calendar_event"
                    | "draft_release_recovery_email"
            )
    )
}

fn generated_text(draft: &GeneratedActionPlanDraft) -> Result<String, PlanCoverageDeficit> {
    let steps = draft
        .steps
        .iter()
        .map(|step| {
            Ok(serde_json::json!({
                "step":step.step,
                "tool":generated_tool(&step.tool)?,
                "risk_level":step.risk_level,
            }))
        })
        .collect::<Result<Vec<_>, PlanCoverageDeficit>>()?;
    Ok(serde_json::json!({
        "steps":steps,
        "exit_condition":draft.exit_condition,
    })
    .to_string())
}

fn generated_tool(tool: &GeneratedToolDraft) -> Result<Value, PlanCoverageDeficit> {
    if let GeneratedToolDraft::RegisteredTaskTool {
        operation,
        arguments,
    } = tool
    {
        let Some(mut fields) = arguments.as_object().cloned() else {
            return Err(PlanCoverageDeficit::missing(vec![format!(
                "valid arguments for '{operation}'"
            )]));
        };
        fields.insert(
            "kind".to_string(),
            Value::String(normalized_operation(operation)),
        );
        return Ok(Value::Object(fields));
    }
    serde_json::to_value(tool)
        .map_err(|_| PlanCoverageDeficit::missing(vec!["serializable compound action".to_string()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPARISON: &str = "Research current primary or official sources on scheduled/background agent capabilities in OpenClaw and Claude Cowork. Write a sourced comparison to `/Users/test/output/comparison.md`. Include URLs, access times, explicit limitations, and what this implies for OOMU. Do not claim completion until the file exists and you have read it back.";
    const MILESTONE: &str = "Read `/Users/test/project/milestone_source.json` and construct a recovery plan that minimizes completion time while respecting dependencies, one-owner capacity, business hours, a 20% contingency reserve, and the requirement that security validation precede release validation. Write the assumptions, critical path, and three failure contingencies to `/Users/test/output/recovery.md` and verify the file.";
    const DECISION_PACK: &str = "Prepare a board-ready supplier decision pack. Read `/Users/test/mock_data/supplier.json`. Reconcile every quoted amount and margin, identify all exceptions, and independently research current primary or official web sources for freight conditions. Create a new `ship_test_01` folder and deliver `supplier_decision.xlsx`, `supplier_decision.pptx`, `supplier_decision.pdf`, and `sources.md`. Then create a tentative 30-minute event in my `OOMU Test` calendar on the next weekday between 1:00 PM and 4:00 PM titled `Supplier Decision Review`, avoiding conflicts, and create an unsent Mail draft to draft@example.com. Do not send the draft.";
    const RELEASE: &str = "Read `/Users/test/mock_data/project_milestones.json`, identify every overdue or unfinished milestone as of today, and prepare a recovery meeting. Find a conflict-free 30-minute block in my calendars on the next weekday between 1:00 PM and 4:00 PM, using my Mac's current Calendar timezone. Create `/Users/test/output/release_recovery_agenda.md` with the milestone facts, proposed slot, owners, decisions needed, and exactly five agenda items. Then propose one tentative event titled `OOMU Release Readiness` in my `OOMU Test` calendar with that agenda. After the Calendar step is resolved, create one unsent Mail draft to draft@example.com with subject `OOMU Release Readiness — Recovery Meeting`, the same proposed time, and the same five agenda items. Do not send it.";

    #[test]
    fn comparison_composes_a_separately_requested_email_send() {
        let objective =
            format!("{COMPARISON} Send the completed comparison to reviewer@example.com by email.");
        let base = evidence_artifact_contract::compile(&objective)
            .unwrap()
            .unwrap();
        assert_eq!(
            operations(&base),
            vec!["prepare_background_agent_comparison"]
        );
        let composed = compose(
            &objective,
            base,
            candidate(vec![registered_step(
                "Send the verified comparison.",
                "send_system_email",
                serde_json::json!({
                    "to":"reviewer@example.com",
                    "subject":"Background agent comparison",
                    "body":"The verified comparison is ready."
                }),
            )]),
        )
        .unwrap();

        assert_eq!(
            operations(&composed),
            vec!["prepare_background_agent_comparison", "send_system_email"]
        );
        validate_objective_coverage(&objective, &composed).unwrap();
    }

    #[test]
    fn milestone_recovery_composes_a_separate_calendar_event() {
        let objective = format!(
            "{MILESTONE} Create a Calendar event titled `Recovery Review` in my Test calendar."
        );
        let base = evidence_artifact_contract::compile(&objective)
            .unwrap()
            .unwrap();
        assert!(validation_objective(&objective, &base)
            .unwrap()
            .contains("Recovery Review"));
        assert_eq!(
            operations(&base),
            vec!["prepare_milestone_constraint_recovery_plan"]
        );
        let composed = compose(
            &objective,
            base,
            candidate(vec![registered_step(
                "Create the separately requested review event.",
                "create_system_calendar_event",
                serde_json::json!({
                    "calendarName":"Test",
                    "title":"Recovery Review",
                    "startDate":"2026-07-22T14:00:00-04:00",
                    "endDate":"2026-07-22T14:30:00-04:00",
                    "location":"",
                    "notes":"Review the verified recovery plan.",
                    "availability":"tentative"
                }),
            )]),
        )
        .unwrap();

        assert_eq!(
            operations(&composed),
            vec![
                "prepare_milestone_constraint_recovery_plan",
                "create_system_calendar_event"
            ]
        );
        validate_objective_coverage(&objective, &composed).unwrap();
    }

    #[test]
    fn decision_pack_keeps_its_three_step_core_and_composes_a_sent_email() {
        let objective = format!(
            "{DECISION_PACK} Separately send an email to audit@example.com after verification."
        );
        let base = decision_pack_contract::compile(&objective, Some("/Users/test/ship_test_01"))
            .unwrap()
            .unwrap();
        assert_eq!(base.steps.len(), 3);
        let composed = compose(
            &objective,
            base,
            candidate(vec![
                registered_step(
                    "Duplicate work the specialist already owns.",
                    "create_decision_pack",
                    serde_json::json!({}),
                ),
                registered_step(
                    "Send the separately requested verified notification.",
                    "send_system_email",
                    serde_json::json!({
                        "to":"audit@example.com",
                        "subject":"Supplier decision pack verified",
                        "body":"The verified decision pack is complete."
                    }),
                ),
            ]),
        )
        .unwrap();

        assert_eq!(
            operations(&composed),
            vec![
                "create_decision_pack",
                "create_conflict_free_calendar_event",
                "draft_decision_pack_email",
                "send_system_email"
            ]
        );
        validate_objective_coverage(&objective, &composed).unwrap();
    }

    #[test]
    fn release_recovery_composes_connector_channel_and_additional_artifact() {
        let objective = format!(
            "{RELEASE} Use the MCP server named CRM to retrieve the release owner record; configure the Slack channel; create and verify `/Users/test/output/release_audit.md`."
        );
        let base = release_recovery_contract::compile(&objective)
            .unwrap()
            .unwrap();
        assert!(validation_objective(&objective, &base)
            .unwrap()
            .contains("MCP server named CRM"));
        assert_eq!(base.steps.len(), 3);
        let composed = compose(
            &objective,
            base,
            candidate(vec![
                registered_step(
                    "Retrieve the separately requested connected record.",
                    "connected_work",
                    serde_json::json!({
                        "connector_ref":"CRM",
                        "capability":"retrieve_release_owner_record",
                        "arguments":{}
                    }),
                ),
                registered_step(
                    "Configure the separately requested Slack channel.",
                    "configure_channel",
                    serde_json::json!({
                        "platform":"slack",
                        "credentials_json":"{}",
                        "owner_id":"",
                        "is_active":true
                    }),
                ),
                registered_step(
                    "Create the separately requested audit artifact.",
                    "create_file",
                    serde_json::json!({
                        "file":{
                            "destinationPath":"/Users/test/output/release_audit.md",
                            "format":"md"
                        }
                    }),
                ),
            ]),
        )
        .unwrap();

        assert_eq!(
            operations(&composed),
            vec![
                "prepare_release_recovery_agenda",
                "create_release_recovery_calendar_event",
                "draft_release_recovery_email",
                "connected_work",
                "configure_channel",
                "create_file"
            ]
        );
        validate_objective_coverage(&objective, &composed).unwrap();
    }

    #[test]
    fn missing_compound_action_fails_closed_before_signing() {
        let objective = format!("{COMPARISON} Send it to reviewer@example.com by email.");
        let base = evidence_artifact_contract::compile(&objective)
            .unwrap()
            .unwrap();
        let error = compose(&objective, base, candidate(Vec::new())).unwrap_err();

        assert_eq!(error.code(), "planner_objective_coverage_incomplete");
        assert!(error.message().contains("reviewer@example.com"));
    }

    #[test]
    fn composition_objective_preserves_authority_bound_action_sentences() {
        let calendar = "Create a Calendar event titled `Recovery Review` in my `Executive Calendar` calendar at 3:15 PM for 45 minutes";
        let mail = "Send an email to Executive.Reviewer@example.com with subject `Go / No-Go` and body `Proceed only after verification`";
        let objective = format!("{MILESTONE} {calendar}. {mail}.");
        let base = evidence_artifact_contract::compile(&objective)
            .unwrap()
            .unwrap();
        let scoped = validation_objective(&objective, &base).unwrap();

        assert!(scoped.contains(calendar));
        assert!(scoped.contains(mail));
        assert!(scoped.contains("Recovery Review"));
        assert!(scoped.contains("Executive.Reviewer@example.com"));
    }

    fn candidate(steps: Vec<GeneratedPlanStepDraft>) -> GeneratedActionPlanDraft {
        GeneratedActionPlanDraft {
            steps,
            exit_condition: "Complete the separately requested additions.".to_string(),
            generated_text: "{}".to_string(),
            source: IntentSource::Gemma,
            degraded_reason: None,
        }
    }

    fn registered_step(step: &str, operation: &str, arguments: Value) -> GeneratedPlanStepDraft {
        GeneratedPlanStepDraft {
            step: step.to_string(),
            tool: GeneratedToolDraft::RegisteredTaskTool {
                operation: operation.to_string(),
                arguments,
            },
            risk_level: GeneratedRiskLevel::High,
        }
    }

    fn operations(draft: &GeneratedActionPlanDraft) -> Vec<&str> {
        draft
            .steps
            .iter()
            .map(|step| match &step.tool {
                GeneratedToolDraft::RegisteredTaskTool { operation, .. } => operation.as_str(),
                _ => "non_registered",
            })
            .collect()
    }
}
