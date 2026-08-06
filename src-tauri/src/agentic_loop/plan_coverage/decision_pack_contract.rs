use super::*;

const REQUIRED_OPERATIONS: [&str; 3] = [
    "create_decision_pack",
    "create_conflict_free_calendar_event",
    "draft_decision_pack_email",
];
const OUTPUT_FIELDS: [(&str, &str); 4] = [
    ("workbook", "xlsx"),
    ("presentation", "pptx"),
    ("pdf", "pdf"),
    ("sources", "md"),
];

pub(super) fn compile(
    objective: &str,
    trusted_output_directory: Option<&str>,
) -> Result<Option<GeneratedActionPlanDraft>, PlanCoverageDeficit> {
    if !requests_evidence_bound_decision_pack(objective) {
        return Ok(None);
    }

    let input_paths = objective_requirements(objective)
        .into_iter()
        .filter_map(|requirement| match requirement {
            Requirement::InputFile { path } => Some(normalize_path(&path)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if input_paths.is_empty()
        || input_paths
            .iter()
            .any(|path| !Path::new(path).is_absolute())
    {
        return Err(input_path(
            "Every decision-pack source must resolve to an exact absolute file path before approval.",
        ));
    }
    let output_directory = trusted_output_directory
        .map(normalize_path)
        .or_else(|| explicit_output_directory(objective))
        .filter(|path| Path::new(path).is_absolute())
        .ok_or_else(|| {
            contract(
                "The decision-pack output folder must resolve to one exact absolute path before approval.",
            )
        })?;
    let outputs = serde_json::json!({
        "workbook": requested_output_name(objective, "xlsx")?,
        "presentation": requested_output_name(objective, "pptx")?,
        "pdf": requested_output_name(objective, "pdf")?,
        "sources": requested_output_name(objective, "md")?,
    });
    let research_policy =
        crate::decision_research_policy::compile_research_policy(objective).map_err(contract)?;
    let calendar_name = objective_calendar_name(objective)
        .ok_or_else(PlanCoverageDeficit::decision_pack_calendar_required)?;
    let event_title = objective_event_title(objective)
        .ok_or_else(|| contract("The requested Calendar event title was not explicit."))?;
    let recipient = objective_email(objective)
        .ok_or_else(|| contract("The requested Mail draft recipient was not explicit."))?;
    let output_paths = OUTPUT_FIELDS
        .iter()
        .map(|(field, _)| {
            format!(
                "{}/{}",
                output_directory.trim_end_matches('/'),
                outputs
                    .get(*field)
                    .and_then(Value::as_str)
                    .expect("compiled output name is a string")
            )
        })
        .collect::<Vec<_>>();
    let decision_arguments = serde_json::json!({
        "title": "Supplier Decision Pack",
        "locale": "en-US",
        "inputPaths": input_paths,
        "researchPolicy": research_policy,
        "analysisInstructions": "Reconcile every quoted amount and margin, identify every exception, retain source data and formulas, and bind each recommendation claim to verified source evidence.",
        "outputDirectory": output_directory,
        "outputs": outputs,
    });
    let calendar_arguments = serde_json::json!({
        "calendarName": calendar_name,
        "title": event_title,
        "day": "next_weekday",
        "windowStartLocal": "13:00",
        "windowEndLocal": "16:00",
        "durationMinutes": 30,
        "location": "",
        "notes": "Review the verified supplier decision pack and its evidence-bound recommendation.",
        "availability": "tentative",
    });
    let mail_arguments = serde_json::json!({
        "to": recipient,
        "subject": event_title,
        "expectedOutputPaths": output_paths,
    });
    let exit_condition = "Exit only after all four decision-pack files are reopened and verified, exactly one conflict-free tentative Calendar event exists, and exactly one matching unsent Mail draft exists.".to_string();
    let generated_decision_tool =
        generated_tool_contract(REQUIRED_OPERATIONS[0], &decision_arguments);
    let generated_calendar_tool =
        generated_tool_contract(REQUIRED_OPERATIONS[1], &calendar_arguments);
    let generated_mail_tool = generated_tool_contract(REQUIRED_OPERATIONS[2], &mail_arguments);
    let generated_text = serde_json::json!({
        "steps": [
            {"step":"Create and verify the evidence-bound supplier decision pack.","tool":generated_decision_tool,"risk_level":"high"},
            {"step":"Create and verify one conflict-free tentative Calendar event.","tool":generated_calendar_tool,"risk_level":"high"},
            {"step":"Create and verify one unsent Mail draft bound to the decision-pack outputs.","tool":generated_mail_tool,"risk_level":"high"}
        ],
        "exit_condition": exit_condition,
    })
    .to_string();
    let draft = GeneratedActionPlanDraft {
        steps: vec![
            deterministic_step(
                "Create and verify the evidence-bound supplier decision pack.",
                REQUIRED_OPERATIONS[0],
                decision_arguments,
            ),
            deterministic_step(
                "Create and verify one conflict-free tentative Calendar event.",
                REQUIRED_OPERATIONS[1],
                calendar_arguments,
            ),
            deterministic_step(
                "Create and verify one unsent Mail draft bound to the decision-pack outputs.",
                REQUIRED_OPERATIONS[2],
                mail_arguments,
            ),
        ],
        exit_condition,
        generated_text,
        source: IntentSource::Deterministic,
        degraded_reason: None,
    };
    validate(objective, &draft)?;
    Ok(Some(draft))
}

fn generated_tool_contract(operation: &str, arguments: &Value) -> Value {
    let mut tool = arguments
        .as_object()
        .expect("deterministic registered tool arguments are objects")
        .clone();
    tool.insert("kind".to_string(), Value::String(operation.to_string()));
    Value::Object(tool)
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

pub(super) fn validate(
    objective: &str,
    draft: &GeneratedActionPlanDraft,
) -> Result<(), PlanCoverageDeficit> {
    if !requests_evidence_bound_decision_pack(objective) {
        return Ok(());
    }
    if draft.steps.len() < REQUIRED_OPERATIONS.len()
        || !matches!(
            &draft.steps[0].tool,
            GeneratedToolDraft::RegisteredTaskTool { operation, .. }
                if normalized_operation(operation) == REQUIRED_OPERATIONS[0]
        )
    {
        return Err(contract(format!(
            "The plan must begin with exactly three ordered operations: {}.",
            REQUIRED_OPERATIONS.join(" -> ")
        )));
    }
    let decision = registered_arguments(draft, 0, REQUIRED_OPERATIONS[0])?;
    let calendar = registered_arguments(draft, 1, REQUIRED_OPERATIONS[1])?;
    let mail = registered_arguments(draft, 2, REQUIRED_OPERATIONS[2])?;
    let output_paths = validate_decision_pack_arguments(objective, decision)?;
    validate_calendar_arguments(objective, calendar)?;
    validate_mail_arguments(objective, mail, &output_paths)?;
    specialist_composition::validate_authorized_extras(objective, draft, REQUIRED_OPERATIONS.len())
}

pub(super) fn requests_evidence_bound_decision_pack(objective: &str) -> bool {
    let lowered = objective.to_ascii_lowercase();
    let explicitly_named = lowered.contains("decision pack");
    let supplier_decision_signature = (lowered.contains("supplier") || lowered.contains("vendor"))
        && lowered.contains("recommend")
        && ["amount", "margin", "exception"]
            .iter()
            .any(|term| lowered.contains(term));
    if !explicitly_named && !supplier_decision_signature {
        return false;
    }
    let requirements = objective_requirements(objective);
    let mut formats = HashSet::new();
    let mut input_files = 0usize;
    let mut research = false;
    let mut calendar = false;
    let mut mail = false;
    for requirement in requirements {
        match requirement {
            Requirement::InputFile { .. } => input_files += 1,
            Requirement::OutputFile { format, .. } | Requirement::OutputFormat { format, .. } => {
                formats.insert(format);
            }
            Requirement::ExternalResearch => research = true,
            Requirement::CrossSurface(
                compound_requirements::CrossSurfaceRequirement::CalendarCreate { .. },
            ) => calendar = true,
            Requirement::CrossSurface(
                compound_requirements::CrossSurfaceRequirement::MailDraft { .. },
            ) => mail = true,
            Requirement::CrossSurface(_) => {}
            Requirement::InputDirectory { .. } => {}
        }
    }
    input_files > 0
        && ["xlsx", "pptx", "pdf", "md"]
            .iter()
            .all(|format| formats.contains(*format))
        && research
        && calendar
        && mail
}

fn registered_arguments<'a>(
    draft: &'a GeneratedActionPlanDraft,
    index: usize,
    expected_operation: &str,
) -> Result<&'a Value, PlanCoverageDeficit> {
    match &draft.steps[index].tool {
        GeneratedToolDraft::RegisteredTaskTool {
            operation,
            arguments,
        } if normalized_operation(operation) == expected_operation => Ok(arguments),
        GeneratedToolDraft::RegisteredTaskTool { operation, .. } => Err(contract(format!(
            "Step {} must use '{}', not '{}'.",
            index + 1,
            expected_operation,
            normalized_operation(operation)
        ))),
        _ => Err(contract(format!(
            "Step {} must use the registered '{}' operation.",
            index + 1,
            expected_operation
        ))),
    }
}

fn validate_decision_pack_arguments(
    objective: &str,
    arguments: &Value,
) -> Result<Vec<String>, PlanCoverageDeficit> {
    require_public_keys(
        arguments,
        &[
            "title",
            "locale",
            "inputPaths",
            "analysisInstructions",
            "outputDirectory",
            "outputs",
        ],
        &[
            "researchQueries",
            "researchPolicy",
            "inputBindings",
            "outputBinding",
        ],
        "create_decision_pack",
    )?;
    require_nonempty_string(arguments, "title", "create_decision_pack")?;
    require_nonempty_string(arguments, "locale", "create_decision_pack")?;

    let expected_inputs = objective_requirements(objective)
        .into_iter()
        .filter_map(|requirement| match requirement {
            Requirement::InputFile { path } => Some(path),
            _ => None,
        })
        .collect::<Vec<_>>();
    let actual_inputs = string_array(arguments, "inputPaths", "create_decision_pack")?;
    if actual_inputs.len() != expected_inputs.len() {
        return Err(input_path(format!(
            "create_decision_pack.inputPaths must contain exactly the {} requested input file(s); received {}.",
            expected_inputs.len(),
            actual_inputs.len()
        )));
    }
    for expected in &expected_inputs {
        if !actual_inputs
            .iter()
            .any(|actual| requested_path_matches(expected, actual))
        {
            return Err(input_path(format!(
                "The generated plan changed or omitted the requested input path '{expected}'."
            )));
        }
    }
    if actual_inputs
        .iter()
        .any(|path| !Path::new(path).is_absolute())
    {
        return Err(input_path(
            "create_decision_pack.inputPaths must resolve every contextual filename to an exact absolute path before approval.",
        ));
    }

    match (
        arguments.get("researchPolicy"),
        arguments.get("researchQueries"),
    ) {
        (Some(policy), None) => {
            let policy = serde_json::from_value::<crate::decision_research_policy::ResearchPolicy>(
                policy.clone(),
            )
            .map_err(|_| {
                contract("create_decision_pack.researchPolicy is not a valid signed policy.")
            })?;
            crate::decision_research_policy::policy_matches_objective(objective, &policy)
                .map_err(contract)?;
        }
        (None, Some(_)) => {
            let queries = string_array(arguments, "researchQueries", "create_decision_pack")?;
            if queries.is_empty()
                || queries.len() > 4
                || !bounded_public_queries(objective, queries.iter().copied())
            {
                return Err(contract(
                    "create_decision_pack.researchQueries must contain only bounded independent public research authorized by the objective.",
                ));
            }
        }
        _ => {
            return Err(contract(
                "create_decision_pack requires exactly one signed researchPolicy or legacy researchQueries contract.",
            ));
        }
    }
    let instructions =
        require_nonempty_string(arguments, "analysisInstructions", "create_decision_pack")?
            .to_ascii_lowercase();
    let objective_lowered = objective.to_ascii_lowercase();
    for requested_term in ["quoted", "amount", "margin", "exception"] {
        if objective_lowered.contains(requested_term) && !instructions.contains(requested_term) {
            return Err(contract(format!(
                "create_decision_pack.analysisInstructions omitted the requested '{requested_term}' analysis."
            )));
        }
    }

    let output_directory =
        require_nonempty_string(arguments, "outputDirectory", "create_decision_pack")?;
    let expected_directory = explicit_output_directory(objective).ok_or_else(|| {
        contract("The requested decision-pack output directory was not explicit.")
    })?;
    if !Path::new(output_directory).is_absolute()
        || !requested_path_matches(&expected_directory, output_directory)
    {
        return Err(contract(format!(
            "create_decision_pack.outputDirectory must preserve the exact requested '{expected_directory}' folder."
        )));
    }
    let outputs = arguments
        .get("outputs")
        .ok_or_else(|| contract("create_decision_pack.outputs is required."))?;
    require_public_keys(
        outputs,
        &["workbook", "presentation", "pdf", "sources"],
        &[],
        "create_decision_pack.outputs",
    )?;
    let mut output_paths = Vec::with_capacity(OUTPUT_FIELDS.len());
    for (field, format) in OUTPUT_FIELDS {
        let actual = require_nonempty_string(outputs, field, "create_decision_pack.outputs")?;
        let expected = requested_output_name(objective, format)?;
        if actual != expected {
            return Err(contract(format!(
                "create_decision_pack.outputs.{field} must preserve the requested filename '{expected}'."
            )));
        }
        output_paths.push(normalize_path(&format!("{output_directory}/{actual}")));
    }
    Ok(output_paths)
}

fn requested_output_name(
    objective: &str,
    expected_format: &str,
) -> Result<String, PlanCoverageDeficit> {
    let names = objective_requirements(objective)
        .into_iter()
        .filter_map(|requirement| match requirement {
            Requirement::OutputFile { path, format } if format == expected_format => {
                Path::new(&path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    if names.len() != 1 {
        return Err(contract(format!(
            "The objective must name exactly one .{expected_format} decision-pack output."
        )));
    }
    Ok(names.into_iter().next().expect("one checked output name"))
}

fn validate_calendar_arguments(
    objective: &str,
    arguments: &Value,
) -> Result<(), PlanCoverageDeficit> {
    require_public_keys(
        arguments,
        &[
            "calendarName",
            "title",
            "day",
            "windowStartLocal",
            "windowEndLocal",
            "durationMinutes",
            "location",
            "notes",
            "availability",
        ],
        &[],
        "create_conflict_free_calendar_event",
    )?;
    for (field, expected) in [
        ("day", "next_weekday"),
        ("windowStartLocal", "13:00"),
        ("windowEndLocal", "16:00"),
        ("availability", "tentative"),
    ] {
        if require_string(arguments, field, "create_conflict_free_calendar_event")? != expected {
            return Err(contract(format!(
                "create_conflict_free_calendar_event.{field} must be '{expected}'."
            )));
        }
    }
    if arguments.get("durationMinutes").and_then(Value::as_i64) != Some(30) {
        return Err(contract(
            "create_conflict_free_calendar_event.durationMinutes must be 30.",
        ));
    }
    let expected = objective_calendar_name(objective)
        .ok_or_else(PlanCoverageDeficit::decision_pack_calendar_required)?;
    let actual = require_string(
        arguments,
        "calendarName",
        "create_conflict_free_calendar_event",
    )?;
    if actual != expected {
        return Err(contract(format!(
            "create_conflict_free_calendar_event.calendarName must preserve '{expected}'."
        )));
    }
    if let Some(expected) = objective_event_title(objective) {
        let actual = require_string(arguments, "title", "create_conflict_free_calendar_event")?;
        if actual != expected {
            return Err(contract(format!(
                "create_conflict_free_calendar_event.title must preserve '{expected}'."
            )));
        }
    }
    require_string(arguments, "location", "create_conflict_free_calendar_event")?;
    require_nonempty_string(arguments, "notes", "create_conflict_free_calendar_event")?;
    Ok(())
}

fn validate_mail_arguments(
    objective: &str,
    arguments: &Value,
    output_paths: &[String],
) -> Result<(), PlanCoverageDeficit> {
    require_public_keys(
        arguments,
        &["to", "subject", "expectedOutputPaths"],
        &[],
        "draft_decision_pack_email",
    )?;
    let recipient = require_nonempty_string(arguments, "to", "draft_decision_pack_email")?;
    if let Some(expected) = objective_email(objective) {
        if !recipient.eq_ignore_ascii_case(expected) {
            return Err(contract(format!(
                "draft_decision_pack_email.to must preserve the requested recipient '{expected}'."
            )));
        }
    }
    require_nonempty_string(arguments, "subject", "draft_decision_pack_email")?;
    let actual_paths = string_array(
        arguments,
        "expectedOutputPaths",
        "draft_decision_pack_email",
    )?;
    let expected = output_paths
        .iter()
        .map(|path| normalize_path(path))
        .collect::<HashSet<_>>();
    let actual = actual_paths
        .iter()
        .map(|path| normalize_path(path))
        .collect::<HashSet<_>>();
    if actual.len() != 4 || actual != expected {
        return Err(contract(
            "draft_decision_pack_email.expectedOutputPaths must exactly match the four decision-pack outputs.",
        ));
    }
    Ok(())
}

fn require_public_keys(
    value: &Value,
    required: &[&str],
    allowed_internal: &[&str],
    operation: &str,
) -> Result<(), PlanCoverageDeficit> {
    let object = value
        .as_object()
        .ok_or_else(|| contract(format!("{operation} arguments must be an object.")))?;
    if required.iter().any(|field| !object.contains_key(*field))
        || object.keys().any(|field| {
            !required.contains(&field.as_str()) && !allowed_internal.contains(&field.as_str())
        })
    {
        return Err(contract(format!(
            "{operation} arguments must contain the declared public fields only."
        )));
    }
    Ok(())
}

fn require_string<'a>(
    value: &'a Value,
    field: &str,
    operation: &str,
) -> Result<&'a str, PlanCoverageDeficit> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| contract(format!("{operation}.{field} must be a string.")))
}

fn require_nonempty_string<'a>(
    value: &'a Value,
    field: &str,
    operation: &str,
) -> Result<&'a str, PlanCoverageDeficit> {
    require_string(value, field, operation).and_then(|text| {
        (!text.trim().is_empty())
            .then_some(text)
            .ok_or_else(|| contract(format!("{operation}.{field} cannot be empty.")))
    })
}

fn string_array<'a>(
    value: &'a Value,
    field: &str,
    operation: &str,
) -> Result<Vec<&'a str>, PlanCoverageDeficit> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| contract(format!("{operation}.{field} must be an array.")))?
        .iter()
        .map(|item| {
            item.as_str()
                .ok_or_else(|| contract(format!("{operation}.{field} must contain strings.")))
        })
        .collect()
}

fn objective_calendar_name(objective: &str) -> Option<String> {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    let captured = REGEX
        .get_or_init(|| {
            Regex::new(r"(?i)\bin my\s+(`[^`\r\n]+`|[a-z0-9][a-z0-9 _-]{0,120}?)\s+calendar\b")
                .expect("calendar name regex")
        })
        .captures(objective)?
        .get(1)?
        .as_str();
    Some(captured.trim_matches('`').trim().to_string())
}

fn objective_event_title(objective: &str) -> Option<String> {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    static TRAILING_CALENDAR: OnceLock<Regex> = OnceLock::new();
    let captured = REGEX
        .get_or_init(|| {
            Regex::new(r"(?i)\btitled\s+(`[^`\r\n]+`|[^,.\r\n]{1,160})").expect("event title regex")
        })
        .captures(objective)?
        .get(1)?
        .as_str();
    let captured = captured.trim_matches('`').trim();
    let calendar_suffix = TRAILING_CALENDAR
        .get_or_init(|| {
            Regex::new(r"(?i)\s+in my\s+(?:`[^`\r\n]+`|[a-z0-9][a-z0-9 _-]{0,120}?)\s+calendar\b")
                .expect("event title trailing calendar regex")
        })
        .find(captured)
        .map(|found| found.start())
        .unwrap_or(captured.len());
    let title = captured[..calendar_suffix].trim();
    (!title.is_empty()).then(|| title.to_string())
}

fn objective_email(objective: &str) -> Option<&str> {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(r"(?i)\b[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9.-]+\.[a-z]{2,}\b")
                .expect("email regex")
        })
        .find(objective)
        .map(|found| found.as_str())
}

fn contract(problem: impl Into<String>) -> PlanCoverageDeficit {
    PlanCoverageDeficit::decision_pack_contract(problem)
}

fn input_path(problem: impl Into<String>) -> PlanCoverageDeficit {
    PlanCoverageDeficit::decision_pack_input_path(problem)
}
