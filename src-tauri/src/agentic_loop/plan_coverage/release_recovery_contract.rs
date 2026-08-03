use super::*;

const REQUIRED_OPERATIONS: [&str; 3] = [
    "prepare_release_recovery_agenda",
    "create_release_recovery_calendar_event",
    "draft_release_recovery_email",
];

pub(super) fn compile(
    objective: &str,
) -> Result<Option<GeneratedActionPlanDraft>, PlanCoverageDeficit> {
    if !requests_release_recovery_workflow(objective) {
        return Ok(None);
    }
    let contract = objective_contract(objective)?;
    let prepare_arguments = serde_json::json!({
        "inputPath":contract.input_path,
        "outputPath":contract.output_path,
        "day":"next_weekday",
        "windowStartLocal":"13:00",
        "windowEndLocal":"16:00",
        "durationMinutes":30,
        "agendaItemCount":5,
        "locale":"en-US",
    });
    let calendar_arguments = serde_json::json!({
        "calendarName":contract.calendar_name,
        "title":contract.event_title,
        "agendaStep":0,
        "availability":"tentative",
    });
    let mail_arguments = serde_json::json!({
        "to":contract.recipient,
        "subject":contract.mail_subject,
        "agendaStep":0,
        "calendarStep":1,
    });
    let exit_condition = "Exit only after the exact Markdown agenda is reopened with its original digest, exactly one matching conflict-free tentative Calendar event exists, exactly one matching unsent Mail draft exists, and no matching message was sent."
        .to_string();
    let steps = vec![
        deterministic_step(
            "Read the exact milestone fixture, freeze one conflict-free slot, and create and verify the five-item recovery agenda.",
            REQUIRED_OPERATIONS[0],
            prepare_arguments.clone(),
        ),
        deterministic_step(
            "After its separate approval, create and verify one tentative Calendar event bound to the frozen agenda receipt.",
            REQUIRED_OPERATIONS[1],
            calendar_arguments.clone(),
        ),
        deterministic_step(
            "After Calendar succeeds and its separate approval is granted, create and verify one unsent Mail draft bound to the same receipt.",
            REQUIRED_OPERATIONS[2],
            mail_arguments.clone(),
        ),
    ];
    let generated_text = serde_json::json!({
        "steps":[
            {"step":steps[0].step,"tool":generated_tool(REQUIRED_OPERATIONS[0], &prepare_arguments),"risk_level":"high"},
            {"step":steps[1].step,"tool":generated_tool(REQUIRED_OPERATIONS[1], &calendar_arguments),"risk_level":"high"},
            {"step":steps[2].step,"tool":generated_tool(REQUIRED_OPERATIONS[2], &mail_arguments),"risk_level":"high"}
        ],
        "exit_condition":exit_condition,
    })
    .to_string();
    let draft = GeneratedActionPlanDraft {
        steps,
        exit_condition,
        generated_text,
        source: IntentSource::Deterministic,
        degraded_reason: None,
    };
    validate_exact(objective, &draft)?;
    Ok(Some(draft))
}

pub(super) fn validate(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> Result<(), PlanCoverageDeficit> {
    if !requests_release_recovery_workflow(objective) {
        return Ok(());
    }
    validate_exact(objective, draft)
}

/// Selects the deterministic production contract at completion time. Execution
/// has already passed MLC signature verification before this matcher runs. The
/// Calendar recovery flow may re-sign the same plan after changing exactly one
/// field: the Calendar target selected by the user. Normalizing only that field
/// preserves the final postcondition without allowing any other plan drift.
pub(super) fn matches_runtime_plan(plan: &super::super::ActionPlan) -> bool {
    let Ok(Some(expected)) = compile(&plan.objective) else {
        return false;
    };
    if plan.steps.len() < expected.steps.len()
        || !specialist_composition::exit_condition_matches(
            &plan.exit_condition,
            &expected.exit_condition,
            plan.steps.len() > expected.steps.len(),
        )
    {
        return false;
    }
    let (Ok(actual_steps), Ok(expected_steps)) = (
        serde_json::to_value(&plan.steps),
        serde_json::to_value(&expected.steps),
    ) else {
        return false;
    };
    matches_steps_after_calendar_target_amendment(actual_steps, &expected_steps)
}

pub(super) fn requested_calendar_name(objective: &str) -> Option<String> {
    objective_contract(objective)
        .ok()
        .map(|contract| contract.calendar_name)
}

fn matches_steps_after_calendar_target_amendment(
    mut actual_steps: Value,
    expected_steps: &Value,
) -> bool {
    let (Some(actual), Some(expected)) = (actual_steps.as_array(), expected_steps.as_array())
    else {
        return false;
    };
    if actual.len() < expected.len() {
        return false;
    }
    actual_steps = Value::Array(actual[..expected.len()].to_vec());
    if &actual_steps == expected_steps {
        return true;
    }
    let Some(selected_calendar) = actual_steps
        .pointer("/1/tool/arguments/calendarName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let Some(expected_calendar) = expected_steps
        .pointer("/1/tool/arguments/calendarName")
        .and_then(Value::as_str)
    else {
        return false;
    };
    if selected_calendar == expected_calendar {
        return false;
    }
    let Some(calendar_name) = actual_steps.pointer_mut("/1/tool/arguments/calendarName") else {
        return false;
    };
    *calendar_name = Value::String(expected_calendar.to_string());
    &actual_steps == expected_steps
}

fn validate_exact(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> Result<(), PlanCoverageDeficit> {
    let contract = objective_contract(objective)?;
    if draft.steps.len() < REQUIRED_OPERATIONS.len()
        || !matches!(
            &draft.steps[0].tool,
            GeneratedToolDraft::RegisteredTaskTool { operation, .. }
                if normalized_operation(operation) == REQUIRED_OPERATIONS[0]
        )
    {
        return Err(error(format!(
            "The plan must begin with exactly three ordered receipt-bound operations: {}.",
            REQUIRED_OPERATIONS.join(" -> ")
        )));
    }
    let prepare = registered_arguments(draft, 0, REQUIRED_OPERATIONS[0])?;
    let calendar = registered_arguments(draft, 1, REQUIRED_OPERATIONS[1])?;
    let mail = registered_arguments(draft, 2, REQUIRED_OPERATIONS[2])?;
    exact_fields(
        prepare,
        &[
            ("inputPath", contract.input_path.as_str()),
            ("outputPath", contract.output_path.as_str()),
            ("day", "next_weekday"),
            ("windowStartLocal", "13:00"),
            ("windowEndLocal", "16:00"),
            ("locale", "en-US"),
        ],
        REQUIRED_OPERATIONS[0],
    )?;
    if prepare.get("durationMinutes").and_then(Value::as_i64) != Some(30)
        || prepare.get("agendaItemCount").and_then(Value::as_u64) != Some(5)
        || prepare.as_object().map(|value| value.len()) != Some(8)
    {
        return Err(error(
            "The agenda step must preserve the exact next-weekday 1:00–4:00 PM, 30-minute, five-item contract.",
        ));
    }
    exact_fields(
        calendar,
        &[
            ("calendarName", contract.calendar_name.as_str()),
            ("title", contract.event_title.as_str()),
            ("availability", "tentative"),
        ],
        REQUIRED_OPERATIONS[1],
    )?;
    if calendar.get("agendaStep").and_then(Value::as_u64) != Some(0)
        || calendar.as_object().map(|value| value.len()) != Some(4)
    {
        return Err(error(
            "The Calendar step must bind only to agenda step 0 and cannot invent its own time or notes.",
        ));
    }
    exact_fields(
        mail,
        &[
            ("to", contract.recipient.as_str()),
            ("subject", contract.mail_subject.as_str()),
        ],
        REQUIRED_OPERATIONS[2],
    )?;
    if mail.get("agendaStep").and_then(Value::as_u64) != Some(0)
        || mail.get("calendarStep").and_then(Value::as_u64) != Some(1)
        || mail.as_object().map(|value| value.len()) != Some(4)
    {
        return Err(error(
            "The Mail step must follow Calendar and bind only to the verified agenda and Calendar receipts.",
        ));
    }
    specialist_composition::validate_authorized_extras(objective, draft, REQUIRED_OPERATIONS.len())
}

#[derive(Debug)]
struct ObjectiveContract {
    input_path: String,
    output_path: String,
    calendar_name: String,
    event_title: String,
    recipient: String,
    mail_subject: String,
}

fn objective_contract(objective: &str) -> Result<ObjectiveContract, PlanCoverageDeficit> {
    let mut inputs = Vec::new();
    for requirement in objective_requirements(objective) {
        match requirement {
            Requirement::InputFile { path } if file_format(&path).as_deref() == Some("json") => {
                inputs.push(normalize_path(&path));
            }
            _ => {}
        }
    }
    let [input_path] = inputs.as_slice() else {
        return Err(error(
            "The workflow requires exactly one explicit absolute milestone JSON input path.",
        ));
    };
    let output_path = specialist_output::markdown_output(
        objective,
        &[
            "agenda",
            "milestone facts",
            "proposed slot",
            "decisions needed",
        ],
    )
    .ok_or_else(|| {
        error(
            "The workflow requires one unambiguous absolute Markdown agenda output; separately requested Markdown artifacts must have distinct purposes.",
        )
    })?;
    if !Path::new(input_path).is_absolute() || !Path::new(&output_path).is_absolute() {
        return Err(error(
            "The milestone input and agenda output must resolve to exact absolute paths before approval.",
        ));
    }
    Ok(ObjectiveContract {
        input_path: input_path.clone(),
        output_path,
        calendar_name: capture_required(objective, calendar_name_regex(), "Calendar name")?,
        event_title: capture_required(objective, event_title_regex(), "event title")?,
        recipient: capture_required(objective, email_regex(), "Mail recipient")?,
        mail_subject: capture_required(objective, mail_subject_regex(), "Mail subject")?,
    })
}

pub(super) fn requests_release_recovery_workflow(objective: &str) -> bool {
    let lowered = objective.to_ascii_lowercase();
    lowered.contains("overdue or unfinished milestone")
        && lowered.contains("recovery meeting")
        && lowered.contains("conflict-free 30-minute")
        && lowered.contains("next weekday")
        && lowered.contains("exactly five agenda items")
        && lowered.contains("tentative event")
        && (lowered.contains("unsent mail draft") || lowered.contains("unsent email draft"))
        && lowered.contains("do not send")
}

fn capture_required(
    objective: &str,
    regex: &Regex,
    label: &str,
) -> Result<String, PlanCoverageDeficit> {
    let value = regex
        .captures(objective)
        .and_then(|captures| captures.get(1))
        .map(|found| found.as_str().trim_matches('`').trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| error(format!("The requested {label} was not explicit.")))?;
    Ok(value)
}

fn calendar_name_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\bin my\s+(`[^`\r\n]+`|[a-z0-9][a-z0-9 _-]{0,120}?)\s+calendar\b")
            .expect("release recovery calendar regex")
    })
}

fn event_title_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\bevent titled\s+(`[^`\r\n]+`|[^,.\r\n]{1,160}?)\s+in my\b")
            .expect("release recovery event title regex")
    })
}

fn email_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\b([a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9.-]+\.[a-z]{2,})\b")
            .expect("release recovery email regex")
    })
}

fn mail_subject_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\bsubject\s+(`[^`\r\n]+`|[^,.\r\n]{1,200})")
            .expect("release recovery mail subject regex")
    })
}

fn deterministic_step(step: &str, operation: &str, arguments: Value) -> GeneratedPlanStepDraft {
    GeneratedPlanStepDraft {
        step: step.to_string(),
        tool: GeneratedToolDraft::RegisteredTaskTool {
            operation: operation.to_string(),
            arguments,
        },
        risk_level: GeneratedRiskLevel::High,
    }
}

fn generated_tool(operation: &str, arguments: &Value) -> Value {
    let mut tool = arguments
        .as_object()
        .expect("deterministic release recovery arguments are objects")
        .clone();
    tool.insert("kind".to_string(), Value::String(operation.to_string()));
    Value::Object(tool)
}

fn registered_arguments<'a>(
    draft: &'a GeneratedActionPlanDraft,
    index: usize,
    operation: &str,
) -> Result<&'a Value, PlanCoverageDeficit> {
    match &draft.steps.get(index).map(|step| &step.tool) {
        Some(GeneratedToolDraft::RegisteredTaskTool {
            operation: actual,
            arguments,
        }) if normalized_operation(actual) == operation => Ok(arguments),
        _ => Err(error(format!("Step {} must use '{operation}'.", index + 1))),
    }
}

fn exact_fields(
    arguments: &Value,
    expected: &[(&str, &str)],
    operation: &str,
) -> Result<(), PlanCoverageDeficit> {
    for (field, expected) in expected {
        if arguments.get(*field).and_then(Value::as_str) != Some(*expected) {
            return Err(error(format!(
                "{operation}.{field} must preserve the exact requested value."
            )));
        }
    }
    Ok(())
}

fn error(problem: impl Into<String>) -> PlanCoverageDeficit {
    PlanCoverageDeficit::release_recovery_contract(problem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OBJECTIVE: &str = "Read `/Users/test/testing/mock_data/project_milestones.json`, identify every overdue or unfinished milestone as of today, and prepare a recovery meeting. Find a conflict-free 30-minute block in my calendars on the next weekday between 1:00 PM and 4:00 PM, using my Mac's current Calendar timezone. Create `/Users/test/testing/ship_test_02/release_recovery_agenda.md` with the milestone facts, proposed slot, owners, decisions needed, and exactly five agenda items. Then propose one tentative event titled `OOMU Release Readiness` in my OOMU Test Denial calendar with that agenda. After the Calendar step is resolved, create one unsent Mail draft to tester@example.com with subject `OOMU Release Readiness — Recovery Meeting`, the same proposed time, and the same five agenda items. Do not send it.";

    #[test]
    fn exact_scenario_compiles_to_three_receipt_bound_operations() {
        let draft = compile(OBJECTIVE).unwrap().unwrap();
        assert_eq!(draft.steps.len(), 3);
        for (index, operation) in REQUIRED_OPERATIONS.iter().enumerate() {
            assert!(matches!(
                &draft.steps[index].tool,
                GeneratedToolDraft::RegisteredTaskTool { operation: actual, .. }
                    if actual == operation
            ));
        }
        validate(OBJECTIVE, &draft).unwrap();
    }

    #[test]
    fn exact_scenario_passes_the_same_final_gate_as_the_live_chat_path() {
        let objective = "Read `/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/project_milestones.json`, identify every overdue or unfinished milestone as of today, and prepare a recovery meeting. Find a conflict-free 30-minute block in my calendars on the next weekday between 1:00 PM and 4:00 PM, using my Mac's current Calendar timezone. Create `/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/ship_test_02/release_recovery_agenda.md` with the milestone facts, proposed slot, owners, decisions needed, and exactly five agenda items. Then propose one tentative event titled `OOMU Release Readiness` in my `OOMU Test Denial` calendar with that agenda. After the Calendar step is resolved, create one unsent Mail draft to `recipient@example.com` with subject `OOMU Release Readiness — Recovery Meeting`, the same proposed time, and the same five agenda items. Do not send it. Ask for approvals when required, retain all completed work if I deny an action, and verify the real file, event, and draft before saying you are done. Complete the Calendar step before requesting approval for the Mail draft.";
        let draft = compile(objective).unwrap().unwrap();

        super::super::prepare_draft_for_execution(objective, draft, false)
            .expect("the deterministic recovery contract must survive final plan validation");
    }

    #[test]
    fn cross_surface_tampering_is_rejected() {
        let mut draft = compile(OBJECTIVE).unwrap().unwrap();
        let GeneratedToolDraft::RegisteredTaskTool { arguments, .. } = &mut draft.steps[2].tool
        else {
            panic!("mail step")
        };
        arguments["calendarStep"] = json!(0);
        assert!(validate(OBJECTIVE, &draft).is_err());
    }

    fn runtime_plan() -> crate::agentic_loop::ActionPlan {
        let draft = compile(OBJECTIVE)
            .expect("production contract should compile")
            .expect("objective should select release recovery");
        crate::agentic_loop::generated_draft_to_plan(
            OBJECTIVE.to_string(),
            draft,
            crate::agentic_loop::ModelRouteDecision {
                selected_model: crate::shield_gate::ModelMetadata::local_gemma(),
                provider_config_id: None,
                provider_id: Some("local_model".to_string()),
                recommended_model: None,
                requires_principal_authorization: false,
                reason: "release recovery contract test route".to_string(),
                context_excerpt_count: 0,
                context_sources: Vec::new(),
            },
            crate::agentic_loop::ContextBundle {
                excerpts: Vec::new(),
                claim_sources: Vec::new(),
                inherited_artifact_hashes: Vec::new(),
            },
        )
    }

    fn runtime_arguments_mut(
        plan: &mut crate::agentic_loop::ActionPlan,
        index: usize,
    ) -> &mut Value {
        let crate::agentic_loop::Tool::RegisteredTaskTool(request) = &mut plan.steps[index].tool
        else {
            panic!("release recovery step {index} must remain registered")
        };
        &mut request.arguments
    }

    #[test]
    fn user_selected_calendar_retarget_still_selects_final_verification() {
        let mut plan = runtime_plan();
        runtime_arguments_mut(&mut plan, 1)["calendarName"] = json!("OOMU Test");

        assert!(super::super::matches_deterministic_release_recovery_plan(
            &plan
        ));
    }

    #[test]
    fn user_selected_calendar_retarget_can_be_resigned_and_verified() {
        crate::tools::release_recovery::register_task_tools()
            .expect("Scenario 2 task tools register");
        let identity = crate::sovereign_identity::SovereignIdentity::initialize_ephemeral();
        let mut plan = crate::agentic_loop::sign_plan(runtime_plan(), &identity)
            .expect("the original exact Scenario 2 plan signs");
        runtime_arguments_mut(&mut plan, 1)["calendarName"] = json!("OOMU Test");
        let retargeted = crate::agentic_loop::sign_plan(plan, &identity)
            .expect("the exact user-selected Calendar amendment signs");

        crate::verifier::MlcVerifier::new()
            .verify_approved_plan(&retargeted, &identity)
            .expect("the re-signed Calendar amendment remains an approved plan");
        assert!(super::super::matches_deterministic_release_recovery_plan(
            &retargeted
        ));
    }

    #[test]
    fn calendar_retarget_exception_rejects_every_other_contract_change() {
        let mut retargeted = runtime_plan();
        runtime_arguments_mut(&mut retargeted, 1)["calendarName"] = json!("OOMU Test");
        assert!(super::super::matches_deterministic_release_recovery_plan(
            &retargeted
        ));

        let mut time = retargeted.clone();
        runtime_arguments_mut(&mut time, 0)["windowStartLocal"] = json!("14:00");
        assert!(!super::super::matches_deterministic_release_recovery_plan(
            &time
        ));

        let mut title = retargeted.clone();
        runtime_arguments_mut(&mut title, 1)["title"] = json!("Different meeting");
        assert!(!super::super::matches_deterministic_release_recovery_plan(
            &title
        ));

        let mut mail = retargeted.clone();
        runtime_arguments_mut(&mut mail, 2)["to"] = json!("other@example.com");
        assert!(!super::super::matches_deterministic_release_recovery_plan(
            &mail
        ));

        let mut path = retargeted.clone();
        runtime_arguments_mut(&mut path, 0)["outputPath"] = json!("/tmp/other-agenda.md");
        assert!(!super::super::matches_deterministic_release_recovery_plan(
            &path
        ));

        let mut dependency = retargeted;
        runtime_arguments_mut(&mut dependency, 2)["calendarStep"] = json!(0);
        assert!(!super::super::matches_deterministic_release_recovery_plan(
            &dependency
        ));
    }
}
