use super::{
    registered_task_capabilities, ComposeWorkflowRequest, EditWorkflowRequest,
    WorkflowCompilerError,
};
use crate::{
    foundation::digest::sha256_hex,
    workflow_ir::{WorkflowIr, WORKFLOW_COMPILER_MODEL},
};
use serde_json::json;

const TASK_SERVER: &str = "oomu_task_tools";
const OFFICIAL_FUEL_URL: &str = "https://www.eia.gov/petroleum/gasdiesel/";
const OFFICIAL_TRANSPORT_URL: &str = "https://ops.fhwa.dot.gov/freight/";
const OFFICIAL_TRANSPORT_FALLBACK_URLS: &[&str] =
    &["https://www.fhwa.dot.gov/policyinformation/statistics.cfm"];
const SPECIALIST_METADATA_KEY: &str = "oomuSpecialistComposer";
const SPECIALIST_METADATA_VALUE: &str = "registered_task_v1";
const ROUTINE_DELIVERY_METADATA_KEY: &str = "oomuRoutineDelivery";
const ROUTINE_DELIVERY_METADATA_VALUE: &str = "configured_private_channel";
const SPECIALIST_AGENT_TIMEOUT_MS: u64 = 120_000;

#[derive(Clone, Copy)]
struct OfficialSource {
    url: &'static str,
    fallback_urls: &'static [&'static str],
}

pub(super) struct FollowUpBindings {
    pub(super) calendar_name: String,
    pub(super) event_title: String,
    pub(super) recipient: String,
    pub(super) subject: String,
    pub(super) duration_minutes: u16,
    pub(super) window_start_local: String,
}

#[derive(Debug)]
struct InlineValue {
    value: String,
    start: usize,
    end: usize,
}

pub(super) fn is_registered_specialist_workflow(workflow_ir: &WorkflowIr) -> bool {
    workflow_ir
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(SPECIALIST_METADATA_KEY))
        .and_then(serde_json::Value::as_str)
        == Some(SPECIALIST_METADATA_VALUE)
}

pub(super) fn compose_supported_workflow(
    request: &ComposeWorkflowRequest,
) -> Result<Option<WorkflowIr>, WorkflowCompilerError> {
    build_supported_workflow(
        &request.prompt,
        request.workflow_id.as_deref(),
        request.name.as_deref(),
    )
}

pub(super) fn edit_supported_workflow(
    request: &EditWorkflowRequest,
) -> Result<Option<WorkflowIr>, WorkflowCompilerError> {
    build_supported_workflow(
        &request.instruction,
        Some(&request.workflow_ir.workflow_id),
        Some(&request.workflow_ir.name),
    )
}

fn build_supported_workflow(
    prompt: &str,
    workflow_id: Option<&str>,
    name: Option<&str>,
) -> Result<Option<WorkflowIr>, WorkflowCompilerError> {
    let normalized = prompt.to_ascii_lowercase();
    let inputs = registered_task_capabilities::requested_local_input_paths(prompt);
    let outputs = registered_task_capabilities::requested_artifact_output_paths(prompt);
    let locale = requested_artifact_locale(prompt);

    if is_scheduled_project_inspection(&normalized, prompt, &inputs, &outputs) {
        return scheduled_project_inspection_ir(prompt, workflow_id, name, &inputs).map(Some);
    }

    if is_paired_evidence_request(&normalized, &inputs, &outputs) {
        let Some((supplier_path, milestone_path)) = input_role_pair(prompt, &inputs) else {
            return Ok(None);
        };
        let (Some(markdown_path), Some(pdf_path)) = (
            one_path_with_extension(&outputs, "md"),
            one_path_with_extension(&outputs, "pdf"),
        ) else {
            return Ok(None);
        };
        let Some(sources) = official_sources(prompt, 2) else {
            return Ok(None);
        };
        let Ok(sources): Result<[OfficialSource; 2], _> = sources.try_into() else {
            return Ok(None);
        };
        return paired_evidence_ir(
            prompt,
            workflow_id,
            name,
            supplier_path,
            milestone_path,
            markdown_path,
            pdf_path,
            &sources,
            &locale,
        )
        .map(Some);
    }

    if is_conditional_follow_up_request(&normalized, &inputs, &outputs) {
        let Some(supplier_path) = input_path_for_role(
            prompt,
            &inputs,
            &["supplier", "quote", "vendor", "rate", "pricing"],
            None,
        ) else {
            return Ok(None);
        };
        let Some(report_path) = one_path_with_extension(&outputs, "md") else {
            return Ok(None);
        };
        let Some(source) = official_sources(prompt, 1).and_then(|items| items.into_iter().next())
        else {
            return Ok(None);
        };
        let Some(follow_up) = follow_up_bindings(prompt) else {
            return Ok(None);
        };
        return conditional_follow_up_ir(
            prompt,
            workflow_id,
            name,
            supplier_path,
            report_path,
            source,
            &follow_up,
            &locale,
        )
        .map(Some);
    }

    Ok(None)
}

fn is_scheduled_project_inspection(
    normalized_prompt: &str,
    prompt: &str,
    inputs: &[String],
    outputs: &[String],
) -> bool {
    !inputs.is_empty()
        && outputs.is_empty()
        && inputs_share_quoted_folder(prompt, inputs)
        && (normalized_prompt.contains("scheduled workflow")
            || normalized_prompt.contains("recurring"))
        && (normalized_prompt.contains("daily")
            || normalized_prompt.contains("every morning")
            || normalized_prompt.contains("every day"))
        && ["inspect", "read", "review", "check"]
            .iter()
            .any(|term| normalized_prompt.contains(term))
        && ["digest", "summary", "report"]
            .iter()
            .any(|term| normalized_prompt.contains(term))
        && !registered_task_capabilities::action_is_negated(normalized_prompt, "read")
}

fn inputs_share_quoted_folder(prompt: &str, inputs: &[String]) -> bool {
    let Some(folder) = inputs
        .first()
        .and_then(|path| path.rsplit_once('/').map(|(folder, _)| folder))
    else {
        return false;
    };
    inputs.iter().all(|path| {
        path.strip_prefix(folder)
            .is_some_and(|suffix| suffix.starts_with('/') && suffix.len() > 1)
    }) && [format!("\"{folder}\""), format!("'{folder}'")]
        .iter()
        .any(|quoted| prompt.contains(quoted))
}

fn is_paired_evidence_request(prompt: &str, inputs: &[String], outputs: &[String]) -> bool {
    inputs.len() >= 2
        && one_path_with_extension(outputs, "md").is_some()
        && one_path_with_extension(outputs, "pdf").is_some()
        && has_supplier_analysis_language(prompt)
        && has_milestone_analysis_language(prompt)
        && has_official_source_language(prompt)
        && prompt.contains("matching pdf")
        && !registered_task_capabilities::action_is_negated(prompt, "read")
        && !has_web_denial(prompt)
}

fn is_conditional_follow_up_request(prompt: &str, inputs: &[String], outputs: &[String]) -> bool {
    !inputs.is_empty()
        && one_path_with_extension(outputs, "md").is_some()
        && has_supplier_analysis_language(prompt)
        && prompt.contains("active quote")
        && prompt.contains("historical settled rate")
        && has_official_source_language(prompt)
        && prompt.contains("calendar")
        && prompt.contains("event")
        && registered_task_capabilities::has_send_intent(prompt)
        && !registered_task_capabilities::action_is_negated(prompt, "read")
        && !has_calendar_denial(prompt)
        && !has_send_denial(prompt)
        && !has_web_denial(prompt)
}

fn has_supplier_analysis_language(prompt: &str) -> bool {
    prompt.contains("supplier")
        && (prompt.contains("rate variance")
            || prompt.contains("active quote")
            || prompt.contains("settled rate")
            || prompt.contains("reconcile supplier"))
}

fn has_milestone_analysis_language(prompt: &str) -> bool {
    prompt.contains("milestone")
        && (prompt.contains("unfinished") || prompt.contains("risk") || prompt.contains("status"))
}

fn has_official_source_language(prompt: &str) -> bool {
    (prompt.contains("official") || prompt.contains("primary"))
        && (prompt.contains("source") || prompt.contains("web"))
        && ["retrieve", "research", "fetch", "read"]
            .iter()
            .any(|verb| prompt.contains(verb))
}

fn has_web_denial(prompt: &str) -> bool {
    [
        "do not use the internet",
        "don't use the internet",
        "never use the internet",
        "do not use web",
        "without web access",
        "do not research",
        "don't research",
        "never research",
    ]
    .iter()
    .any(|phrase| prompt.contains(phrase))
        || ["retrieve", "research", "fetch"]
            .iter()
            .any(|verb| registered_task_capabilities::action_is_negated(prompt, verb))
}

fn has_calendar_denial(prompt: &str) -> bool {
    [
        "do not create an event",
        "do not create any event",
        "don't create an event",
        "never create an event",
        "without creating an event",
        "must not create an event",
        "must not create any event",
        "should not create an event",
        "should not create any event",
        "an event must not be created",
        "an event should not be created",
        "no event should be created",
        "no calendar event should be created",
        "skip the calendar",
    ]
    .iter()
    .any(|phrase| prompt.contains(phrase))
}

fn has_send_denial(prompt: &str) -> bool {
    registered_task_capabilities::action_is_negated(prompt, "send")
        || prompt.contains("skip the email")
}

fn one_path_with_extension<'a>(paths: &'a [String], extension: &str) -> Option<&'a str> {
    let mut matches = paths.iter().filter(|path| {
        path.rsplit_once('.')
            .is_some_and(|(_, actual)| actual.eq_ignore_ascii_case(extension))
    });
    let only = matches.next()?;
    matches.next().is_none().then_some(only.as_str())
}

fn input_role_pair<'a>(prompt: &str, inputs: &'a [String]) -> Option<(&'a str, &'a str)> {
    let supplier_terms = ["supplier", "quote", "vendor", "rate", "pricing"];
    let milestone_terms = ["milestone", "roadmap", "project status", "delivery target"];
    let mut ranked = inputs
        .iter()
        .enumerate()
        .flat_map(|(supplier_index, supplier)| {
            inputs
                .iter()
                .enumerate()
                .filter(move |(milestone_index, _)| *milestone_index != supplier_index)
                .map(move |(milestone_index, milestone)| {
                    let supplier_score = registered_task_capabilities::input_path_role_score(
                        prompt,
                        supplier,
                        &supplier_terms,
                    );
                    let milestone_score = registered_task_capabilities::input_path_role_score(
                        prompt,
                        milestone,
                        &milestone_terms,
                    );
                    (
                        supplier_score.saturating_add(milestone_score),
                        supplier_score,
                        milestone_score,
                        supplier_index,
                        milestone_index,
                    )
                })
        })
        .filter(|(_, supplier_score, milestone_score, _, _)| {
            *supplier_score > 0 && *milestone_score > 0
        })
        .collect::<Vec<_>>();
    ranked.sort_unstable_by(|left, right| right.0.cmp(&left.0));
    let best = ranked.first()?;
    if ranked.get(1).is_some_and(|next| next.0 == best.0) {
        return None;
    }
    Some((inputs[best.3].as_str(), inputs[best.4].as_str()))
}

fn input_path_for_role<'a>(
    prompt: &str,
    inputs: &'a [String],
    terms: &[&str],
    excluded: Option<usize>,
) -> Option<&'a str> {
    let mut ranked = inputs
        .iter()
        .enumerate()
        .filter(|(index, _)| excluded != Some(*index))
        .map(|(index, path)| {
            (
                registered_task_capabilities::input_path_role_score(prompt, path, terms),
                index,
            )
        })
        .filter(|(score, _)| *score > 0)
        .collect::<Vec<_>>();
    ranked.sort_unstable_by(|left, right| right.0.cmp(&left.0));
    let best = ranked.first()?;
    if ranked.get(1).is_some_and(|next| next.0 == best.0) {
        return None;
    }
    Some(inputs[best.1].as_str())
}

fn official_sources(prompt: &str, required: usize) -> Option<Vec<OfficialSource>> {
    let normalized = prompt.to_ascii_lowercase();
    let policies = [
        (
            &["energy", "fuel", "diesel", "gasoline"] as &[&str],
            OfficialSource {
                url: OFFICIAL_FUEL_URL,
                fallback_urls: &[],
            },
        ),
        (
            &["transport", "freight", "logistics", "shipping"],
            OfficialSource {
                url: OFFICIAL_TRANSPORT_URL,
                fallback_urls: OFFICIAL_TRANSPORT_FALLBACK_URLS,
            },
        ),
    ];
    let mut selected = policies
        .into_iter()
        .filter(|(terms, _)| terms.iter().any(|term| normalized.contains(term)))
        .map(|(_, source)| source)
        .collect::<Vec<_>>();
    selected.dedup_by_key(|source| source.url);
    (selected.len() >= required).then(|| selected.into_iter().take(required).collect())
}

fn specialist_metadata(prompt: &str) -> serde_json::Value {
    let mut metadata = serde_json::Map::from_iter([(
        SPECIALIST_METADATA_KEY.to_string(),
        json!(SPECIALIST_METADATA_VALUE),
    )]);
    let normalized = prompt.to_ascii_lowercase();
    if normalized.contains("configured private channel")
        || normalized.contains("routine's configured channel")
        || normalized.contains("routine’s configured channel")
    {
        metadata.insert(
            ROUTINE_DELIVERY_METADATA_KEY.to_string(),
            json!(ROUTINE_DELIVERY_METADATA_VALUE),
        );
    }
    serde_json::Value::Object(metadata)
}

fn inline_values(prompt: &str) -> Vec<InlineValue> {
    let mut open = None;
    let mut values = Vec::new();
    for (index, character) in prompt.char_indices() {
        if character != '`' {
            continue;
        }
        if let Some(start) = open.take() {
            let value = normalize_inline_value(&prompt[start..index]);
            if !value.is_empty() {
                values.push(InlineValue {
                    value,
                    start,
                    end: index,
                });
            }
        } else {
            open = Some(index + 1);
        }
    }
    values
}

fn normalize_inline_value(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut characters = value.trim().chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' && matches!(characters.peek(), Some(' ' | '~' | '\\')) {
            normalized.push(characters.next().expect("peeked escaped character"));
        } else {
            normalized.push(character);
        }
    }
    normalized
}

fn inline_value_after(prompt: &str, keyword: &str) -> Option<String> {
    let normalized = prompt.to_ascii_lowercase();
    let keyword_at = normalized.find(keyword)?;
    inline_values(prompt)
        .into_iter()
        .filter(|value| value.start >= keyword_at.saturating_add(keyword.len()))
        .find(|value| value.start.saturating_sub(keyword_at + keyword.len()) <= 96)
        .map(|value| value.value)
}

fn plain_value_after(prompt: &str, keyword: &str, terminators: &[&str]) -> Option<String> {
    let normalized = prompt.to_ascii_lowercase();
    let keyword_at = normalized.find(keyword)?;
    let start = keyword_at.saturating_add(keyword.len());
    let remainder = prompt.get(start..)?.trim_start_matches(|character: char| {
        character.is_whitespace() || matches!(character, ':' | '`' | '"' | '\'')
    });
    let lowered = remainder.to_ascii_lowercase();
    let end = terminators
        .iter()
        .filter_map(|terminator| lowered.find(terminator))
        .min()
        .unwrap_or(remainder.len());
    let value = remainder[..end]
        .trim()
        .trim_matches(|character: char| matches!(character, '`' | '"' | '\'' | ',' | '.'));
    (!value.is_empty()).then(|| value.to_string())
}

fn calendar_name(prompt: &str) -> Option<String> {
    let normalized = prompt.to_ascii_lowercase();
    let inline = inline_values(prompt).into_iter().find_map(|value| {
        let suffix_start = value.end.saturating_add(1);
        let after_end = (suffix_start + 32).min(prompt.len());
        let after = normalized.get(suffix_start..after_end)?;
        let calendar_at = after.find("calendar")?;
        (!after[..calendar_at].contains('`')).then_some(value.value)
    });
    inline.or_else(|| {
        let calendar_at = normalized.find(" calendar")?;
        let prefix = prompt.get(..calendar_at)?;
        [" in the ", " in my "]
            .iter()
            .filter_map(|marker| {
                prefix
                    .to_ascii_lowercase()
                    .rfind(marker)
                    .map(|at| (at, marker))
            })
            .max_by_key(|(at, _)| *at)
            .map(|(at, marker)| prefix[at + marker.len()..].trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn email_recipient(prompt: &str) -> Option<String> {
    prompt
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|character: char| {
                matches!(character, '`' | '"' | '\'' | '(' | ')' | ',' | ';' | '.')
            })
        })
        .find(|token| {
            let mut parts = token.split('@');
            parts.next().is_some_and(|local| !local.is_empty())
                && parts.next().is_some_and(|domain| domain.contains('.'))
                && parts.next().is_none()
        })
        .map(str::to_string)
}

fn duration_minutes(prompt: &str) -> Option<u16> {
    let normalized = prompt.to_ascii_lowercase();
    normalized.split_whitespace().find_map(|token| {
        let numeric = token
            .strip_suffix("-minute")
            .or_else(|| token.strip_suffix("-minutes"))?;
        numeric.parse::<u16>().ok().filter(|value| *value > 0)
    })
}

fn window_start_local(prompt: &str) -> Option<String> {
    let tokens = prompt.split_whitespace().collect::<Vec<_>>();
    for pair in tokens.windows(2) {
        let time =
            pair[0].trim_matches(|character: char| !character.is_ascii_digit() && character != ':');
        let meridiem = pair[1].trim_matches(|character: char| !character.is_ascii_alphabetic());
        if !matches!(meridiem.to_ascii_lowercase().as_str(), "am" | "pm") {
            continue;
        }
        let (hours, minutes) = time.split_once(':').unwrap_or((time, "0"));
        let (Ok(mut hours), Ok(minutes)) = (hours.parse::<u16>(), minutes.parse::<u16>()) else {
            continue;
        };
        if !(1..=12).contains(&hours) || minutes > 59 {
            continue;
        }
        if meridiem.eq_ignore_ascii_case("pm") && hours != 12 {
            hours += 12;
        } else if meridiem.eq_ignore_ascii_case("am") && hours == 12 {
            hours = 0;
        }
        return Some(format!("{hours:02}:{minutes:02}"));
    }
    None
}

pub(super) fn follow_up_bindings(prompt: &str) -> Option<FollowUpBindings> {
    let normalized = prompt.to_ascii_lowercase();
    if !normalized.contains("next conflict-free weekday") && !normalized.contains("next weekday") {
        return None;
    }
    Some(FollowUpBindings {
        calendar_name: calendar_name(prompt)?,
        event_title: inline_value_after(prompt, "titled").or_else(|| {
            plain_value_after(
                prompt,
                "titled",
                &[" in the ", " in my ", " on the ", ", and "],
            )
        })?,
        recipient: email_recipient(prompt)?,
        subject: inline_value_after(prompt, "subject").or_else(|| {
            plain_value_after(
                prompt,
                "subject",
                &[" and the report", " with the report", ". ", " these "],
            )
        })?,
        duration_minutes: duration_minutes(prompt)?,
        window_start_local: window_start_local(prompt)?,
    })
}

pub(super) fn requests_report_attachment(prompt: &str) -> bool {
    let normalized = prompt.to_ascii_lowercase();
    normalized.contains("report attached")
        || normalized.contains("attach the report")
        || normalized.contains("report as an attachment")
}

fn artifact_title(path: &str) -> String {
    let stem = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(path);
    let without_template = stem
        .split('<')
        .next()
        .unwrap_or(stem)
        .trim_matches(['_', '-']);
    let words = without_template
        .split(['_', '-'])
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    if words.is_empty() {
        "Verified report".to_string()
    } else {
        let mut title = words.join(" ");
        if let Some(first) = title.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        title
    }
}

fn requested_artifact_locale(prompt: &str) -> String {
    let candidate = inline_value_after(prompt, "locale").or_else(|| {
        let words = prompt.split_whitespace().collect::<Vec<_>>();
        words.windows(2).find_map(|pair| {
            pair[0]
                .trim_matches(|character: char| !character.is_ascii_alphabetic())
                .eq_ignore_ascii_case("locale")
                .then(|| {
                    pair[1]
                        .trim_matches(|character: char| {
                            !character.is_ascii_alphanumeric() && character != '-'
                        })
                        .to_string()
                })
        })
    });
    candidate
        .filter(|value| is_locale_tag(value))
        .unwrap_or_else(|| "und".to_string())
}

fn is_locale_tag(value: &str) -> bool {
    (2..=35).contains(&value.len())
        && value.split('-').all(|part| {
            !part.is_empty()
                && part.len() <= 8
                && part
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        })
}

fn workflow_identity(
    prompt: &str,
    requested_id: Option<&str>,
    requested_name: Option<&str>,
    kind: &str,
    fallback_name: &str,
) -> (String, String) {
    let workflow_id = requested_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let digest = sha256_hex(format!("{kind}:{prompt}").as_bytes());
            format!("wf-{kind}-{}", &digest[..12])
        });
    let name = requested_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_name)
        .to_string();
    (workflow_id, name)
}

fn scheduled_project_inspection_ir(
    prompt: &str,
    workflow_id: Option<&str>,
    name: Option<&str>,
    inputs: &[String],
) -> Result<WorkflowIr, WorkflowCompilerError> {
    let fallback_name = inputs
        .first()
        .map(|path| artifact_title(path))
        .unwrap_or_else(|| "Project".to_string());
    let (workflow_id, name) = workflow_identity(
        prompt,
        workflow_id,
        name,
        "scheduled-project-inspection",
        &fallback_name,
    );
    let agent_objective = format!(
        "Use only the mapped verified Project file contents to carry out this exact user objective: {prompt} Return the requested operational digest, state when no matching exception exists, and never invent records or claim access beyond these inputs."
    );
    let mut nodes = vec![json!({
        "kind":"input","id":"input","label":name,
        "outputKey":"workflow.input","inputSchema":{"type":"object"}
    })];
    let mut edges = Vec::new();
    let mut previous_node = "input".to_string();
    let mut input_mappings = serde_json::Map::new();

    for (index, path) in inputs.iter().enumerate() {
        let ordinal = index + 1;
        let node_id = format!("read-project-file-{ordinal}");
        nodes.push(json!({
            "kind":"mcp_tool","id":node_id,"label":artifact_title(path),
            "serverName":TASK_SERVER,"toolName":"read_project_file","arguments":{"path":path}
        }));
        edges.push(json!({
            "id":format!("edge-{ordinal}"),"sourceNodeId":previous_node,
            "sourcePort":"out","targetNodeId":node_id
        }));
        input_mappings.insert(
            format!("projectFile{ordinal}"),
            json!(format!("{{{{nodes.{node_id}.output.data.content}}}}")),
        );
        previous_node = node_id;
    }

    nodes.push(json!({
        "kind":"agent","id":"prepare-digest","label":name,
        "objective":agent_objective,
        "inputMappings":input_mappings,"outputKey":"nodes.prepare-digest.output",
        "systemTimeoutMs":SPECIALIST_AGENT_TIMEOUT_MS
    }));
    edges.push(json!({
        "id":"edge-prepare-digest","sourceNodeId":previous_node,
        "sourcePort":"out","targetNodeId":"prepare-digest"
    }));
    nodes.push(json!({
        "kind":"output","id":"output","label":name,
        "inputMapping":"{{nodes.prepare-digest.output}}","outputSchema":{"type":"string"}
    }));
    edges.push(json!({
        "id":"edge-output","sourceNodeId":"prepare-digest",
        "sourcePort":"out","targetNodeId":"output"
    }));

    serde_json::from_value(json!({
        "schemaVersion":"1.0.0","workflowId":workflow_id,"workflowVersion":1,
        "name":name,"description":prompt,"compiler":{"model":WORKFLOW_COMPILER_MODEL},
        "metadata":specialist_metadata(prompt),"nodes":nodes,"edges":edges
    }))
    .map_err(WorkflowCompilerError::serialization)
}

fn paired_evidence_ir(
    prompt: &str,
    workflow_id: Option<&str>,
    name: Option<&str>,
    supplier_path: &str,
    milestone_path: &str,
    markdown_path: &str,
    pdf_path: &str,
    sources: &[OfficialSource; 2],
    locale: &str,
) -> Result<WorkflowIr, WorkflowCompilerError> {
    let title = artifact_title(markdown_path);
    let (workflow_id, name) =
        workflow_identity(prompt, workflow_id, name, "paired-evidence", &title);
    let [first_source, second_source] = sources;
    let metadata = specialist_metadata(prompt);
    serde_json::from_value(json!({
        "schemaVersion":"1.0.0",
        "workflowId":workflow_id,
        "workflowVersion":1,
        "name":name,
        "description":"Read two verified role-bound inputs, gather bounded official evidence, and create matching verified Markdown and PDF artifacts.",
        "compiler":{"model":WORKFLOW_COMPILER_MODEL},
        "metadata":metadata,
        "nodes":[
            {"kind":"input","id":"input","label":name,"outputKey":"workflow.input","inputSchema":{"type":"object"}},
            {"kind":"mcp_tool","id":"read-suppliers","label":artifact_title(supplier_path),"serverName":TASK_SERVER,"toolName":"read_project_file","arguments":{"path":supplier_path}},
            {"kind":"mcp_tool","id":"analyze-suppliers","label":artifact_title(supplier_path),"serverName":TASK_SERVER,"toolName":"analyze_supplier_exceptions","arguments":{"content":"{{nodes.read-suppliers.output.data.content}}"}},
            {"kind":"mcp_tool","id":"read-milestones","label":artifact_title(milestone_path),"serverName":TASK_SERVER,"toolName":"read_project_file","arguments":{"path":milestone_path}},
            {"kind":"mcp_tool","id":"analyze-milestones","label":artifact_title(milestone_path),"serverName":TASK_SERVER,"toolName":"analyze_project_milestones","arguments":{"content":"{{nodes.read-milestones.output.data.content}}"}},
            {"kind":"mcp_tool","id":"official-source-a","label":first_source.url,"serverName":TASK_SERVER,"toolName":"fetch_official_page","arguments":{"url":first_source.url,"fallbackUrls":first_source.fallback_urls,"maxContentChars":3000}},
            {"kind":"mcp_tool","id":"official-source-b","label":second_source.url,"serverName":TASK_SERVER,"toolName":"fetch_official_page","arguments":{"url":second_source.url,"fallbackUrls":second_source.fallback_urls,"maxContentChars":3000}},
            {"kind":"mcp_tool","id":"compose-brief","label":title,"serverName":TASK_SERVER,"toolName":"compose_evidence_report","arguments":{"supplierAnalysis":"{{nodes.analyze-suppliers.output.data}}","milestoneAnalysis":"{{nodes.analyze-milestones.output.data}}","officialPageReceipts":["{{nodes.official-source-a.output.data}}","{{nodes.official-source-b.output.data}}"]}},
            {"kind":"mcp_tool","id":"validate-brief","label":title,"serverName":TASK_SERVER,"toolName":"validate_evidence_report","arguments":{"content":"{{nodes.compose-brief.output.data.content}}","supplierAnalysis":"{{nodes.analyze-suppliers.output.data}}","milestoneAnalysis":"{{nodes.analyze-milestones.output.data}}","officialPageReceipts":["{{nodes.official-source-a.output.data}}","{{nodes.official-source-b.output.data}}"],"requiredSections":["Executive summary","Supplier data","Exceptions","Milestone risks","Current evidence","Sources","Next actions"]}},
            {"kind":"mcp_tool","id":"write-md","label":title,"serverName":TASK_SERVER,"toolName":"create_file","arguments":{"file":{"title":title,"content":"{{nodes.validate-brief.output.data.content}}","locale":locale,"format":"md","destinationPath":markdown_path}}},
            {"kind":"mcp_tool","id":"write-pdf","label":title,"serverName":TASK_SERVER,"toolName":"create_file","arguments":{"file":{"title":title,"content":"{{nodes.validate-brief.output.data.content}}","locale":locale,"format":"pdf","destinationPath":pdf_path}}},
            {"kind":"output","id":"output","label":title,"inputMapping":"✓ {{nodes.write-md.output.data.structuredContent.path}}\n✓ {{nodes.write-pdf.output.data.structuredContent.path}}","outputSchema":{"type":"string"}}
        ],
        "edges":[
            {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read-suppliers"},
            {"id":"e2","sourceNodeId":"read-suppliers","sourcePort":"out","targetNodeId":"analyze-suppliers"},
            {"id":"e3","sourceNodeId":"analyze-suppliers","sourcePort":"out","targetNodeId":"read-milestones"},
            {"id":"e4","sourceNodeId":"read-milestones","sourcePort":"out","targetNodeId":"analyze-milestones"},
            {"id":"e5","sourceNodeId":"analyze-milestones","sourcePort":"out","targetNodeId":"official-source-a"},
            {"id":"e6","sourceNodeId":"official-source-a","sourcePort":"out","targetNodeId":"official-source-b"},
            {"id":"e7","sourceNodeId":"official-source-b","sourcePort":"out","targetNodeId":"compose-brief"},
            {"id":"e8","sourceNodeId":"compose-brief","sourcePort":"out","targetNodeId":"validate-brief"},
            {"id":"e9","sourceNodeId":"validate-brief","sourcePort":"out","targetNodeId":"write-md"},
            {"id":"e10","sourceNodeId":"write-md","sourcePort":"out","targetNodeId":"write-pdf"},
            {"id":"e11","sourceNodeId":"write-pdf","sourcePort":"out","targetNodeId":"output"}
        ]
    }))
    .map_err(WorkflowCompilerError::serialization)
}

fn conditional_follow_up_ir(
    prompt: &str,
    workflow_id: Option<&str>,
    name: Option<&str>,
    supplier_path: &str,
    report_path: &str,
    source: OfficialSource,
    follow_up: &FollowUpBindings,
    locale: &str,
) -> Result<WorkflowIr, WorkflowCompilerError> {
    let title = artifact_title(report_path);
    let (workflow_id, name) =
        workflow_identity(prompt, workflow_id, name, "conditional-follow-up", &title);
    let calendar_reason = format!("{} · {}", follow_up.event_title, follow_up.calendar_name);
    let send_reason = format!("{} · {}", follow_up.subject, follow_up.recipient);
    let email_body = format!(
        "{}: {{{{nodes.write-report.output.data.structuredContent.path}}}}",
        title
    );
    let metadata = specialist_metadata(prompt);
    serde_json::from_value(json!({
        "schemaVersion":"1.0.0",
        "workflowId":workflow_id,
        "workflowVersion":1,
        "name":name,
        "description":"Create a verified variance report and, only when its typed condition requires it, request approval for one Calendar event and one email.",
        "compiler":{"model":WORKFLOW_COMPILER_MODEL},
        "metadata":metadata,
        "nodes":[
            {"kind":"input","id":"input","label":name,"outputKey":"workflow.input","inputSchema":{"type":"object"}},
            {"kind":"mcp_tool","id":"read-suppliers","label":artifact_title(supplier_path),"serverName":TASK_SERVER,"toolName":"read_project_file","arguments":{"path":supplier_path}},
            {"kind":"mcp_tool","id":"analyze-suppliers","label":artifact_title(supplier_path),"serverName":TASK_SERVER,"toolName":"analyze_supplier_exceptions","arguments":{"content":"{{nodes.read-suppliers.output.data.content}}"}},
            {"kind":"mcp_tool","id":"source","label":source.url,"serverName":TASK_SERVER,"toolName":"fetch_official_page","arguments":{"url":source.url,"fallbackUrls":source.fallback_urls,"maxContentChars":3000}},
            {"kind":"agent","id":"assess","label":title,"objective":"Using only the typed supplier analysis and official-source receipt, write a concise Markdown report. Use these exact headings: Supplier variance, Current evidence, Risk assessment, and Next actions. Include the supplied audit year and quarter when present; every supplier's name, historical settled rate, active quote, variance, and status; and the official receipt's exact final URL and UTC access time. Distinguish current evidence from local fixture facts, state uncertainty plainly, and never invent facts.","inputMappings":{"supplierAnalysis":"{{nodes.analyze-suppliers.output.data}}","source":"{{nodes.source.output.data}}"},"outputKey":"nodes.assess.output","systemTimeoutMs":SPECIALIST_AGENT_TIMEOUT_MS},
            {"kind":"mcp_tool","id":"validate-report","label":title,"serverName":TASK_SERVER,"toolName":"validate_evidence_report","arguments":{"content":"{{nodes.assess.output.data}}","supplierAnalysis":"{{nodes.analyze-suppliers.output.data}}","officialPageReceipts":["{{nodes.source.output.data}}"],"requiredSections":["Supplier variance","Current evidence","Risk assessment","Next actions"]}},
            {"kind":"mcp_tool","id":"write-report","label":title,"serverName":TASK_SERVER,"toolName":"create_file","arguments":{"file":{"title":title,"content":"{{nodes.validate-report.output.data.content}}","locale":locale,"format":"md","destinationPath":report_path}}},
            {"kind":"conditional","id":"has-exception","label":title,"condition":"$.hasException == true","inputMapping":"{{nodes.analyze-suppliers.output.data}}"},
            {"kind":"output","id":"no-exception","label":title,"inputMapping":"✓ {{nodes.write-report.output.data.structuredContent.path}}","outputSchema":{"type":"string"}},
            {"kind":"permission","id":"approve-calendar","label":follow_up.event_title,"permission":"mcp_tool","reason":calendar_reason,"onDenied":"branch"},
            {"kind":"output","id":"calendar-denied","label":follow_up.event_title,"inputMapping":format!("✓ {{{{nodes.write-report.output.data.structuredContent.path}}}}\n○ {}", follow_up.event_title),"outputSchema":{"type":"string"}},
            {"kind":"mcp_tool","id":"calendar","label":follow_up.event_title,"serverName":TASK_SERVER,"toolName":"create_conflict_free_calendar_event","arguments":{"calendarName":follow_up.calendar_name,"title":follow_up.event_title,"day":"next_weekday","windowStartLocal":follow_up.window_start_local,"windowEndLocal":"18:00","durationMinutes":follow_up.duration_minutes,"location":"","notes":"{{nodes.write-report.output.data.structuredContent.path}}","availability":"tentative"}},
            {"kind":"permission","id":"approve-send","label":follow_up.subject,"permission":"mcp_tool","reason":send_reason,"onDenied":"branch"},
            {"kind":"output","id":"send-denied","label":follow_up.subject,"inputMapping":format!("✓ {{{{nodes.write-report.output.data.structuredContent.path}}}}\n✓ {}\n○ {}", follow_up.event_title, follow_up.subject),"outputSchema":{"type":"string"}},
            {"kind":"mcp_tool","id":"send","label":follow_up.subject,"serverName":TASK_SERVER,"toolName":"send_system_email","arguments":{"to":follow_up.recipient,"subject":follow_up.subject,"body":email_body,"attachmentPath":"{{nodes.write-report.output.data.structuredContent.path}}"}},
            {"kind":"output","id":"output","label":title,"inputMapping":format!("✓ {{{{nodes.write-report.output.data.structuredContent.path}}}}\n✓ {}\n✓ {}", follow_up.event_title, follow_up.subject),"outputSchema":{"type":"string"}}
        ],
        "edges":[
            {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read-suppliers"},
            {"id":"e2","sourceNodeId":"read-suppliers","sourcePort":"out","targetNodeId":"analyze-suppliers"},
            {"id":"e3","sourceNodeId":"analyze-suppliers","sourcePort":"out","targetNodeId":"source"},
            {"id":"e4","sourceNodeId":"source","sourcePort":"out","targetNodeId":"assess"},
            {"id":"e5","sourceNodeId":"assess","sourcePort":"out","targetNodeId":"validate-report"},
            {"id":"e6","sourceNodeId":"validate-report","sourcePort":"out","targetNodeId":"write-report"},
            {"id":"e7","sourceNodeId":"write-report","sourcePort":"out","targetNodeId":"has-exception"},
            {"id":"e8","sourceNodeId":"has-exception","sourcePort":"false","targetNodeId":"no-exception"},
            {"id":"e9","sourceNodeId":"has-exception","sourcePort":"true","targetNodeId":"approve-calendar"},
            {"id":"e10","sourceNodeId":"approve-calendar","sourcePort":"denied","targetNodeId":"calendar-denied"},
            {"id":"e11","sourceNodeId":"approve-calendar","sourcePort":"approved","targetNodeId":"calendar"},
            {"id":"e12","sourceNodeId":"calendar","sourcePort":"out","targetNodeId":"approve-send"},
            {"id":"e13","sourceNodeId":"approve-send","sourcePort":"denied","targetNodeId":"send-denied"},
            {"id":"e14","sourceNodeId":"approve-send","sourcePort":"approved","targetNodeId":"send"},
            {"id":"e15","sourceNodeId":"send","sourcePort":"out","targetNodeId":"output"}
        ]
    }))
    .map_err(WorkflowCompilerError::serialization)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_ir::WorkflowNode;

    const OPERATIONS: &str = "At each run, read `/Project/input/quarterly_quotes.json` as the supplier rate input and `/Project/input/roadmap_state.json` as the project milestone input. Retrieve current information from at least two official public web sources, including one energy source and one transport source. Create `/Project/output/executive_pulse_<YYYY-MM-DD_HH-mm>.md` and a matching PDF. Reconcile supplier rate variances and unfinished milestone risks, validate both files, and deliver the exact filenames. Use locale `fr-CA`.";
    const EXCEPTION: &str = "Read `/Project/input/vendor_rates.json` as the supplier quote input. Retrieve one official freight source. Create `reports/rate_watch_<YYYY-MM-DD_HH-mm>.md`. If any supplier's active quote exceeds its historical settled rate, create one 45-minute event titled `Rate Review` in the `Operations` calendar on the next conflict-free weekday at 3:30 PM or later, and send one email to `ops@example.com` with subject `Rate alert`. These actions require explicit user approval.";
    const CANONICAL_EXCEPTION: &str = "Read `/Users/example/Library/Mobile\\ Documents/com\\~apple\\~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json`. Retrieve one current primary or official public source relevant to US freight or fuel conditions. Create `ship_test_06/supplier_exception_<YYYY-MM-DD_HH-mm>.md` containing the local variances, live source URL/access time, risk assessment, and next actions. If any supplier's active quote exceeds its historical settled rate, create one 30-minute event titled `Supplier Exception Follow-up` in the `OOMU Test` calendar on the next conflict-free weekday at 2:00 PM or later, and send one email to `recipient@example.com` with subject `OOMU Test — Supplier Exception` and the report attached or linked. These Calendar and send actions require explicit user approval. If approval is pending, preserve the run and resume from that exact step after approval. Never create duplicate events, messages, reports, or deliveries when retrying or recovering. Finally, deliver the run result and exact report filename to the configured private channel.";
    const LAB_AUDIT: &str = "Create a recurring daily scheduled workflow named \"Lab Inventory & Maintenance Audit\" that runs every morning at 8:00 AM. It should inspect Maintenance_Tickets.csv and Lab_Inventory.csv in \"/Users/jeffreyallan/Documents/OOMU/Projects/mock_data\", flag open critical tickets or depleted inventory, and generate a daily operational digest.";

    #[test]
    fn scheduled_project_inspection_is_composed_without_model_generation() {
        registered_task_capabilities::register_test_tools();
        let mut request = test_request(LAB_AUDIT, "wf-lab-audit");
        request.name = Some("Lab Inventory & Maintenance Audit".to_string());
        request.capability_catalog.actions =
            registered_task_capabilities::catalog_actions().expect("registered actions");

        let ir = compose_supported_workflow(&request)
            .expect("safe specialist composition")
            .expect("scheduled Project inspection");
        ir.validate().expect("valid workflow");
        super::super::validate_workflow_ir_topology(&ir).expect("safe workflow topology");
        registered_task_capabilities::validate_objective_bindings(LAB_AUDIT, &ir)
            .expect("both exact Project inputs are bound");

        let reads = ir
            .nodes
            .iter()
            .filter_map(|node| match node {
                WorkflowNode::McpTool(tool) if tool.tool_name == "read_project_file" => Some(tool),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(reads.len(), 2);
        assert_eq!(
            reads
                .iter()
                .filter_map(|tool| tool
                    .arguments
                    .get("path")
                    .and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>(),
            vec![
                "/Users/jeffreyallan/Documents/OOMU/Projects/mock_data/Maintenance_Tickets.csv",
                "/Users/jeffreyallan/Documents/OOMU/Projects/mock_data/Lab_Inventory.csv",
            ]
        );
        let digest = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::Agent(agent) if agent.id == "prepare-digest" => Some(agent),
                _ => None,
            })
            .expect("digest agent");
        assert!(digest.objective.contains("open critical tickets"));
        assert!(digest.objective.contains("depleted inventory"));
        assert_eq!(digest.input_mappings.len(), 2);
        assert!(digest
            .input_mappings
            .values()
            .all(|mapping| mapping.ends_with(".output.data.content}}")));

        let response = super::super::specialist_compose_response(ir, &request, 0)
            .expect("grounded specialist response");
        assert_eq!(response.status, "composed");
        assert_eq!(response.composed_by, "registered_task_specialist");
        assert_eq!(response.attempts, 1);
    }

    #[test]
    fn unscheduled_project_inspection_keeps_the_generic_composer_fallback() {
        let prompt = LAB_AUDIT.replace(
            "Create a recurring daily scheduled workflow",
            "Create a workflow",
        );
        assert!(
            compose_supported_workflow(&test_request(&prompt, "wf-generic"))
                .expect("safe recognition")
                .is_none()
        );
    }

    #[test]
    fn operations_brief_is_built_without_model_generation() {
        registered_task_capabilities::register_test_tools();
        let mut request = test_request(OPERATIONS, "wf-operations");
        request.capability_catalog.actions =
            registered_task_capabilities::catalog_actions().expect("registered actions");
        let ir = compose_supported_workflow(&request)
            .expect("specialist composition")
            .expect("operations workflow");
        ir.validate().expect("valid workflow");
        super::super::validate_workflow_ir_topology(&ir).expect("safe workflow topology");
        registered_task_capabilities::validate_objective_bindings(OPERATIONS, &ir)
            .expect("objective-bound workflow");
        assert!(is_registered_specialist_workflow(&ir));
        let compiled = super::super::compile_registered_specialist_instructions(&ir)
            .expect("deterministic instruction compilation");
        assert!(compiled.instructions.is_empty());
        assert_eq!(ir.workflow_id, "wf-operations");
        assert!(ir.nodes.iter().any(|node| matches!(node, WorkflowNode::McpTool(tool) if tool.tool_name == "create_file" && tool.arguments.pointer("/file/format") == Some(&json!("pdf")))));
        assert!(ir
            .nodes
            .iter()
            .filter_map(|node| match node {
                WorkflowNode::McpTool(tool) if tool.tool_name == "create_file" => Some(tool),
                _ => None,
            })
            .all(|tool| tool.arguments.pointer("/file/locale") == Some(&json!("fr-CA"))));
        let transport = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::McpTool(tool) if tool.id == "official-source-b" => Some(tool),
                _ => None,
            })
            .expect("transport source");
        assert_eq!(transport.arguments["url"], OFFICIAL_TRANSPORT_URL);
        assert_eq!(
            transport.arguments["fallbackUrls"],
            json!(OFFICIAL_TRANSPORT_FALLBACK_URLS)
        );
        let composer = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::McpTool(tool) if tool.id == "compose-brief" => Some(tool),
                _ => None,
            })
            .expect("deterministic evidence composer");
        assert_eq!(composer.tool_name, "compose_evidence_report");
        assert_eq!(
            composer.arguments["supplierAnalysis"],
            "{{nodes.analyze-suppliers.output.data}}"
        );
        assert_eq!(
            composer.arguments["milestoneAnalysis"],
            "{{nodes.analyze-milestones.output.data}}"
        );
        assert_eq!(
            composer.arguments["officialPageReceipts"],
            json!([
                "{{nodes.official-source-a.output.data}}",
                "{{nodes.official-source-b.output.data}}"
            ])
        );
        let validator = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::McpTool(tool) if tool.id == "validate-brief" => Some(tool),
                _ => None,
            })
            .expect("post-composition validator");
        assert_eq!(
            validator.arguments["content"],
            "{{nodes.compose-brief.output.data.content}}"
        );
        let response = super::super::specialist_compose_response(ir, &request, 0)
            .expect("grounded specialist response");
        assert_eq!(response.status, "composed");
        assert_eq!(response.composed_by, "registered_task_specialist");
    }

    #[test]
    fn supplier_exception_has_two_explicit_denial_continuations() {
        registered_task_capabilities::register_test_tools();
        let mut request = test_request(EXCEPTION, "wf-exception");
        request.capability_catalog.actions =
            registered_task_capabilities::catalog_actions().expect("registered actions");
        let ir = compose_supported_workflow(&request)
            .expect("specialist composition")
            .expect("supplier workflow");
        ir.validate().expect("valid workflow");
        super::super::validate_workflow_ir_topology(&ir).expect("safe workflow topology");
        registered_task_capabilities::validate_objective_bindings(EXCEPTION, &ir)
            .expect("objective-bound workflow");
        let permissions = ir
            .nodes
            .iter()
            .filter(|node| matches!(node, WorkflowNode::Permission(_)))
            .count();
        assert_eq!(permissions, 2);
        assert_eq!(
            ir.edges
                .iter()
                .filter(|edge| edge.source_port == "denied")
                .count(),
            2
        );
        let calendar = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::McpTool(tool)
                    if tool.tool_name == "create_conflict_free_calendar_event" =>
                {
                    Some(tool)
                }
                _ => None,
            })
            .expect("calendar effect");
        assert_eq!(calendar.arguments["calendarName"], "Operations");
        assert_eq!(calendar.arguments["title"], "Rate Review");
        assert_eq!(calendar.arguments["durationMinutes"], 45);
        assert_eq!(calendar.arguments["windowStartLocal"], "15:30");
        let send = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::McpTool(tool) if tool.tool_name == "send_system_email" => Some(tool),
                _ => None,
            })
            .expect("send effect");
        assert_eq!(send.arguments["to"], "ops@example.com");
        assert_eq!(send.arguments["subject"], "Rate alert");
        assert_eq!(
            send.arguments["attachmentPath"],
            "{{nodes.write-report.output.data.structuredContent.path}}"
        );
        assert_eq!(
            ir.nodes.iter().find_map(|node| match node {
                WorkflowNode::McpTool(tool) if tool.id == "write-report" =>
                    tool.arguments.pointer("/file/locale"),
                _ => None,
            }),
            Some(&json!("und"))
        );

        let edit = EditWorkflowRequest {
            instruction: EXCEPTION.to_string(),
            workflow_ir: ir,
            capability_catalog: request.capability_catalog,
        };
        let edited = edit_supported_workflow(&edit)
            .expect("specialist edit")
            .expect("edited workflow");
        assert_eq!(edited.workflow_id, "wf-exception");
        assert_eq!(edited.name, "Rate watch");
    }

    #[test]
    fn canonical_supplier_exception_keeps_exact_preview_and_delivery_contracts() {
        registered_task_capabilities::register_test_tools();
        let mut request = test_request(CANONICAL_EXCEPTION, "wf-canonical-exception");
        request.capability_catalog.actions =
            registered_task_capabilities::catalog_actions().expect("registered actions");
        let ir = compose_supported_workflow(&request)
            .expect("safe specialist composition")
            .expect("canonical supplier workflow");
        ir.validate().expect("valid workflow");
        super::super::validate_workflow_ir_topology(&ir).expect("safe workflow topology");
        registered_task_capabilities::validate_objective_bindings(CANONICAL_EXCEPTION, &ir)
            .expect("objective-bound workflow");

        assert_eq!(
            ir.metadata
                .as_ref()
                .and_then(|metadata| metadata.get(ROUTINE_DELIVERY_METADATA_KEY)),
            Some(&json!(ROUTINE_DELIVERY_METADATA_VALUE))
        );
        let condition = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::Conditional(condition) if condition.id == "has-exception" => {
                    Some(condition)
                }
                _ => None,
            })
            .expect("typed supplier condition");
        assert_eq!(condition.condition, "$.hasException == true");
        assert_eq!(
            condition.input_mapping.as_deref(),
            Some("{{nodes.analyze-suppliers.output.data}}")
        );

        let calendar = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::McpTool(tool) if tool.id == "calendar" => Some(tool),
                _ => None,
            })
            .expect("Calendar effect");
        assert_eq!(calendar.arguments["calendarName"], "OOMU Test");
        assert_eq!(calendar.arguments["title"], "Supplier Exception Follow-up");
        assert_eq!(calendar.arguments["day"], "next_weekday");
        assert_eq!(calendar.arguments["windowStartLocal"], "14:00");
        assert_eq!(calendar.arguments["durationMinutes"], 30);

        let send = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::McpTool(tool) if tool.id == "send" => Some(tool),
                _ => None,
            })
            .expect("Mail effect");
        assert_eq!(send.arguments["to"], "recipient@example.com");
        assert_eq!(send.arguments["subject"], "OOMU Test — Supplier Exception");
        assert_eq!(
            send.arguments["attachmentPath"],
            "{{nodes.write-report.output.data.structuredContent.path}}"
        );

        let permission_ids = ir
            .nodes
            .iter()
            .filter_map(|node| match node {
                WorkflowNode::Permission(permission) => Some(permission.id.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(permission_ids, vec!["approve-calendar", "approve-send"]);
        let approved_targets = ir
            .edges
            .iter()
            .filter(|edge| edge.source_port == "approved")
            .map(|edge| edge.target_node_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(approved_targets, vec!["calendar", "send"]);

        assert!(ir.nodes.iter().any(|node| matches!(
            node,
            WorkflowNode::Agent(agent)
                if agent.id == "assess"
                    && agent.system_timeout_ms == Some(SPECIALIST_AGENT_TIMEOUT_MS)
        )));
        let output = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::Output(output) if output.id == "output" => Some(output),
                _ => None,
            })
            .expect("terminal delivery output");
        assert!(output
            .input_mapping
            .contains("{{nodes.write-report.output.data.structuredContent.path}}"));
        assert!(ir.edges.iter().any(|edge| {
            edge.source_node_id == "send"
                && edge.source_port == "out"
                && edge.target_node_id == "output"
        }));
    }

    #[test]
    fn unquoted_effect_values_remain_exactly_bound() {
        let prompt = EXCEPTION
            .replace("`Rate Review`", "Rate Review")
            .replace("`Operations`", "Operations")
            .replace("`ops@example.com`", "ops@example.com")
            .replace("`Rate alert`", "Rate alert");
        let ir = compose_supported_workflow(&test_request(&prompt, "wf-plain-effects"))
            .expect("safe composition")
            .expect("conditional workflow");
        registered_task_capabilities::validate_objective_bindings(&prompt, &ir)
            .expect("exact effect bindings");
        let calendar = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::McpTool(tool) if tool.id == "calendar" => Some(tool),
                _ => None,
            })
            .expect("calendar effect");
        assert_eq!(calendar.arguments["calendarName"], "Operations");
        assert_eq!(calendar.arguments["title"], "Rate Review");
        let send = ir
            .nodes
            .iter()
            .find_map(|node| match node {
                WorkflowNode::McpTool(tool) if tool.id == "send" => Some(tool),
                _ => None,
            })
            .expect("mail effect");
        assert_eq!(send.arguments["to"], "ops@example.com");
        assert_eq!(send.arguments["subject"], "Rate alert");
    }

    #[test]
    fn negated_file_and_effect_requests_never_enter_the_specialist_path() {
        let no_output = OPERATIONS.replace(
            "Create `/Project/output/executive_pulse_",
            "Do not create `/Project/output/executive_pulse_",
        );
        assert!(
            compose_supported_workflow(&test_request(&no_output, "wf-no-write"))
                .expect("safe recognition")
                .is_none()
        );
        assert!(
            registered_task_capabilities::requested_artifact_output_paths(&no_output).is_empty()
        );

        let must_not_create = OPERATIONS.replace(
            "Create `/Project/output/executive_pulse_",
            "Must not create `/Project/output/executive_pulse_",
        );
        assert!(
            compose_supported_workflow(&test_request(&must_not_create, "wf-must-not-create"))
                .expect("safe recognition")
                .is_none()
        );

        let mixed_output = OPERATIONS.replace(
            "Create `/Project/output/executive_pulse_<YYYY-MM-DD_HH-mm>.md`",
            "Create `/Project/output/executive_pulse_<YYYY-MM-DD_HH-mm>.md`, but this report must not be created",
        );
        assert!(
            compose_supported_workflow(&test_request(&mixed_output, "wf-mixed-write"))
                .expect("safe recognition")
                .is_none()
        );

        let should_not_write = OPERATIONS.replace(
            "Create `/Project/output/executive_pulse_<YYYY-MM-DD_HH-mm>.md`",
            "Create `/Project/output/executive_pulse_<YYYY-MM-DD_HH-mm>.md`, but should not write it",
        );
        assert!(compose_supported_workflow(&test_request(
            &should_not_write,
            "wf-should-not-write"
        ))
        .expect("safe recognition")
        .is_none());

        let no_read = OPERATIONS.replace(
            "At each run, read `/Project/input/quarterly_quotes.json`",
            "At each run, do not read `/Project/input/quarterly_quotes.json`",
        );
        assert!(
            compose_supported_workflow(&test_request(&no_read, "wf-no-read"))
                .expect("safe recognition")
                .is_none()
        );

        let no_send = format!("{EXCEPTION} Do not send one email under any circumstances.");
        assert!(
            compose_supported_workflow(&test_request(&no_send, "wf-no-send"))
                .expect("safe recognition")
                .is_none()
        );

        let must_not_send = format!("{EXCEPTION} This email must not be sent.");
        assert!(
            compose_supported_workflow(&test_request(&must_not_send, "wf-must-not-send"))
                .expect("safe recognition")
                .is_none()
        );

        let no_event = format!("{EXCEPTION} No event should be created.");
        assert!(
            compose_supported_workflow(&test_request(&no_event, "wf-no-event"))
                .expect("safe recognition")
                .is_none()
        );
    }

    fn test_request(prompt: &str, workflow_id: &str) -> ComposeWorkflowRequest {
        ComposeWorkflowRequest {
            prompt: prompt.to_string(),
            capability_catalog: super::super::CapabilityCatalog {
                version: "test".to_string(),
                authoring_enabled: true,
                generated_at_ms: 0,
                actions: Vec::new(),
                templates: Vec::new(),
            },
            project_id: None,
            workflow_id: Some(workflow_id.to_string()),
            name: None,
        }
    }
}
