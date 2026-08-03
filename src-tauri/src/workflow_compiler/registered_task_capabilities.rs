use super::*;
use crate::workflow_ir::PermissionDeniedBehavior;

mod evidence_report_binding;
mod intent;
#[cfg(test)]
mod test_support;

use evidence_report_binding::{exact_agent_data_producer, exact_report_validation_binding};
pub(super) use intent::has_send_intent;
#[cfg(test)]
pub(super) use test_support::register_test_tools;

pub(super) const REGISTERED_TASK_SERVER: &str = "oomu_task_tools";
const EFFECT_DENIAL_CODE: &str = "workflow_effect_missing_denial_continuation";
const OBJECTIVE_BINDING_CODE: &str = "workflow_objective_capability_mismatch";

pub(super) fn catalog_actions() -> Result<Vec<CapabilityAction>, WorkflowCompilerError> {
    let definitions = [
        (
            "native:task:create-file",
            "Create and verify a real file",
            "Create an exact Markdown, PDF, Office, or text artifact and verify its bytes and readable content.",
            "create_file",
        ),
        (
            "native:task:read-project-file",
            "Read a verified Project file",
            "Read one exact UTF-8 file from an approved Project folder and verify its canonical identity, bytes, and SHA-256 without staging it.",
            "read_project_file",
        ),
        (
            "native:task:fetch-official-page",
            "Read an official public source",
            "Fetch one explicit public HTTPS source with final URL, UTC access time, bounded readable content, and SHA-256 evidence.",
            "fetch_official_page",
        ),
        (
            "native:task:analyze-supplier-exceptions",
            "Verify supplier quote exceptions",
            "Parse the exact supplier fixture bytes and calculate typed active-versus-settled variances without model inference.",
            "analyze_supplier_exceptions",
        ),
        (
            "native:task:analyze-project-milestones",
            "Verify unfinished Project milestones",
            "Parse the exact milestone fixture bytes and return a bounded typed unfinished-work ledger without model inference.",
            "analyze_project_milestones",
        ),
        (
            "native:task:compose-evidence-report",
            "Compose a complete evidence brief",
            "Render every typed supplier and milestone fact plus exact official-source receipts into bounded executive Markdown without model inference.",
            "compose_evidence_report",
        ),
        (
            "native:task:validate-evidence-report",
            "Validate an evidence report",
            "Check exact Agent-authored report bytes against typed analyses, official-source receipts, and required Markdown sections before any artifact write or delivery.",
            "validate_evidence_report",
        ),
        (
            "native:task:create-conflict-free-calendar-event",
            "Create a verified conflict-free Calendar event",
            "Choose the earliest free 30-minute slot in the approved next-weekday local window, create one tentative event, and verify it in Calendar.",
            "create_conflict_free_calendar_event",
        ),
        (
            "native:task:send-system-email",
            "Send and verify one email",
            "After explicit approval, send one exact email through Mail and verify exactly one matching message in Sent Mail. Never use this for a draft.",
            "send_system_email",
        ),
    ];
    definitions
        .into_iter()
        .map(|(id, title, outcome, operation)| {
            let input_schema =
                crate::tools::task_tool_runtime::schema(operation).map_err(|_| {
                    WorkflowCompilerError::metadata(format!(
                        "Registered Task tool schema is missing for {operation}."
                    ))
                })?;
            Ok(CapabilityAction {
                id: id.to_string(),
                kind: "mcp_tool".to_string(),
                title: title.to_string(),
                outcome: outcome.to_string(),
                detail: outcome.to_string(),
                source: "native_task".to_string(),
                available: true,
                availability: "available".to_string(),
                unavailable_reason: None,
                server_name: Some(REGISTERED_TASK_SERVER.to_string()),
                tool_name: Some(operation.to_string()),
                input_schema: Some(input_schema),
                output_schema: Some(output_schema(operation)),
                node_kind: Some("mcp".to_string()),
                node_template: None,
            })
        })
        .collect()
}

fn output_schema(operation: &str) -> Value {
    match operation {
        "create_file" => json!({
            "type":"object",
            "properties":{
                "path":{"type":"string"},"format":{"type":"string"},
                "sha256":{"type":"string"},"verifiedContentSha256":{"type":"string"},
                "byteLength":{"type":"integer"},"verificationMethod":{"type":"string"},
                "structuredContent":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}
            },
            "required":["path","format","sha256","verifiedContentSha256","byteLength","verificationMethod","structuredContent"],
            "additionalProperties":true
        }),
        "read_project_file" => json!({
            "type":"object",
            "properties":{
                "canonicalPath":{"type":"string"},
                "content":{"type":"string"},
                "byteCount":{"type":"integer"},
                "contentSha256":{"type":"string"},
                "verified":{"type":"boolean"}
            },
            "required":["canonicalPath","content","byteCount","contentSha256","verified"],
            "additionalProperties":false
        }),
        "fetch_official_page" => json!({
            "type":"object",
            "properties":{
                "requestedUrl":{"type":"string"},"selectedUrl":{"type":"string"},
                "attemptedUrls":{"type":"array","items":{"type":"string"}},
                "fallbackUsed":{"type":"boolean"},"finalUrl":{"type":"string"},
                "accessedAtUtc":{"type":"string"},"statusCode":{"type":"integer"},
                "contentType":{"type":"string"},"content":{"type":"string"},
                "contentSha256":{"type":"string"},"contentBytes":{"type":"integer"},
                "contentTruncated":{"type":"boolean"}
            },
            "required":["requestedUrl","selectedUrl","attemptedUrls","fallbackUsed","finalUrl","accessedAtUtc","statusCode","contentType","content","contentSha256","contentBytes","contentTruncated"],
            "additionalProperties":false
        }),
        "analyze_supplier_exceptions" => json!({
            "type":"object",
            "properties":{
                "sourceSha256":{"type":"string"},
                "auditYear":{"type":"integer"},
                "quarter":{"type":"string","enum":["Q1","Q2","Q3","Q4"]},
                "supplierCount":{"type":"integer"},
                "exceptionCount":{"type":"integer"},
                "hasException":{"type":"boolean"},
                "suppliers":{"type":"array","items":{
                    "type":"object",
                    "properties":{
                        "name":{"type":"string"},
                        "historicalSettledRate":{"type":"number"},
                        "activeQuote":{"type":"number"},
                        "variance":{"type":"number"},
                        "exceedsHistorical":{"type":"boolean"},
                        "status":{"type":"string"}
                    },
                    "required":["name","historicalSettledRate","activeQuote","variance","exceedsHistorical","status"],
                    "additionalProperties":false
                }}
            },
            "required":["sourceSha256","supplierCount","exceptionCount","hasException","suppliers"],
            "additionalProperties":false
        }),
        "analyze_project_milestones" => json!({
            "type":"object",
            "properties":{
                "sourceSha256":{"type":"string"},
                "milestoneCount":{"type":"integer"},
                "unfinishedCount":{"type":"integer"},
                "hasUnfinishedMilestones":{"type":"boolean"},
                "milestones":{"type":"array","items":{
                    "type":"object",
                    "properties":{
                        "milestoneId":{"type":"string"},
                        "name":{"type":"string"},
                        "targetDate":{"type":"string"},
                        "status":{"type":"string"},
                        "owner":{"type":"string"},
                        "dependencies":{"type":"array","items":{"type":"string"}},
                        "unfinished":{"type":"boolean"}
                    },
                    "required":["milestoneId","name","targetDate","status","owner","dependencies","unfinished"],
                    "additionalProperties":false
                }}
            },
            "required":["sourceSha256","milestoneCount","unfinishedCount","hasUnfinishedMilestones","milestones"],
            "additionalProperties":false
        }),
        "compose_evidence_report" => json!({
            "type":"object",
            "properties":{
                "content":{"type":"string"},
                "contentSha256":{"type":"string"},
                "byteCount":{"type":"integer"},
                "supplierAnalysisSha256":{"type":"string"},
                "milestoneAnalysisSha256":{"type":"string"},
                "officialEvidenceSha256":{"type":"string"},
                "sourceCount":{"type":"integer"},
                "requiredSections":{"type":"array","items":{"type":"string"}},
                "compositionMethod":{"type":"string"}
            },
            "required":["content","contentSha256","byteCount","supplierAnalysisSha256","milestoneAnalysisSha256","officialEvidenceSha256","sourceCount","requiredSections","compositionMethod"],
            "additionalProperties":false
        }),
        "validate_evidence_report" => json!({
            "type":"object",
            "properties":{
                "content":{"type":"string"},
                "contentSha256":{"type":"string"},
                "byteCount":{"type":"integer"},
                "supplierAnalysisSha256":{"type":"string"},
                "milestoneAnalysisSha256":{"type":"string"},
                "officialEvidenceSha256":{"type":"string"},
                "sourceCount":{"type":"integer"},
                "requiredSections":{"type":"array","items":{"type":"string"}},
                "verified":{"type":"boolean"}
            },
            "required":["content","contentSha256","byteCount","supplierAnalysisSha256","officialEvidenceSha256","sourceCount","requiredSections","verified"],
            "additionalProperties":false
        }),
        "send_system_email" => json!({
            "type":"object",
            "properties":{
                "sentMessageIdSha256":{"type":"string"},"subjectSha256":{"type":"string"},
                "bodySha256":{"type":"string"},"sent":{"type":"boolean"},
                "verified":{"type":"boolean"},"exactMatchCount":{"type":"integer"},
                "uniquenessVerified":{"type":"boolean"},"reusedExisting":{"type":"boolean"}
            },
            "required":["sentMessageIdSha256","subjectSha256","bodySha256","sent","verified","exactMatchCount","uniquenessVerified"],
            "additionalProperties":true
        }),
        _ => json!({
            "type":"object",
            "properties":{"verified":{"type":"boolean"},"exists":{"type":"boolean"}},
            "required":["verified","exists"],
            "additionalProperties":true
        }),
    }
}

pub(super) fn selection_hint(action: &CapabilityAction) -> Option<&'static str> {
    match (
        action.server_name.as_deref(),
        action.tool_name.as_deref(),
    ) {
        (Some(REGISTERED_TASK_SERVER), Some("create_file")) => Some(
            "Use for every requested real MD/PDF/Office artifact. One node creates and verifies one exact file; use separate nodes for matching MD and PDF outputs.",
        ),
        (Some(REGISTERED_TASK_SERVER), Some("read_project_file")) => Some(
            "Use one node per exact local input named by the user. Pass the exact path; this reads directly from the approved Project root and returns canonicalPath, content, byteCount, and contentSha256.",
        ),
        (Some(REGISTERED_TASK_SERVER), Some("fetch_official_page")) => Some(
            "Use one node per explicit primary or official HTTPS source. Preserve finalUrl, accessedAtUtc, content, and contentSha256 for sourced synthesis.",
        ),
        (Some(REGISTERED_TASK_SERVER), Some("analyze_supplier_exceptions")) => Some(
            "Use after the exact supplier fixture read when later actions depend on active quote versus historical settled rate. Branch on its typed hasException result, never on Agent prose.",
        ),
        (Some(REGISTERED_TASK_SERVER), Some("analyze_project_milestones")) => Some(
            "Use after the exact milestone fixture read. Pass only that read's exact content and use the bounded typed result for unfinished milestones and milestone risks.",
        ),
        (Some(REGISTERED_TASK_SERVER), Some("compose_evidence_report")) => Some(
            "Use for paired supplier-and-milestone evidence briefs. Bind the exact typed analyses and exact official-page receipts; it deterministically includes every record and makes no untyped web claim.",
        ),
        (Some(REGISTERED_TASK_SERVER), Some("validate_evidence_report")) => Some(
            "Use after evidence-bound Agent synthesis and before every report write. Pass the exact Agent text, exact typed analyses, exact official-page receipts, and required Markdown headings; downstream files must consume only its verified content.",
        ),
        (Some(REGISTERED_TASK_SERVER), Some("create_conflict_free_calendar_event")) => Some(
            "Use for a next-weekday conflict-free tentative Calendar event. Put an explicit permission node immediately before it and branch denial to a truthful terminal output.",
        ),
        (Some(REGISTERED_TASK_SERVER), Some("send_system_email")) => Some(
            "Use only when the user explicitly asks to send. Never substitute draft_system_email. Put an explicit permission node immediately before it and branch denial to a truthful terminal output.",
        ),
        (_, Some("draft_system_email")) => Some(
            "High priority for mail, email, reply, draft, drafting replies, or open-draft intents.",
        ),
        (_, Some("read_system_emails")) => {
            Some("High priority for mail, email, inbox, unread message, and reply workflows.")
        }
        (_, Some("write_markdown_report")) => Some(
            "Use only for explicit report or markdown output, such as write a report, save a Markdown summary, or save a project summary.",
        ),
        (_, Some("preview_report")) => Some(
            "Use only for explicit report preview/open-report intents, and only after an upstream report writer exists.",
        ),
        _ => None,
    }
}

pub(super) fn matches_prompt(operation: &str, prompt: &str) -> bool {
    let contains_any = |terms: &[&str]| terms.iter().any(|term| prompt.contains(term));
    match operation {
        "create_file" => contains_any(&["create", "write", "file", "report", "markdown", "pdf"]),
        "read_project_file" => contains_any(&["read", "file", "fixture", "input"]),
        "fetch_official_page" => contains_any(&["official", "primary", "source", "current", "web"]),
        "analyze_supplier_exceptions" => {
            contains_any(&["supplier", "active quote", "settled rate", "variance"])
        }
        "analyze_project_milestones" => contains_any(&[
            "milestone",
            "unfinished",
            "project status",
            "milestone risk",
        ]),
        "compose_evidence_report" => {
            contains_any(&["report", "brief", "evidence", "supplier", "milestone"])
        }
        "validate_evidence_report" => {
            contains_any(&["report", "brief", "evidence", "validate", "verify"])
        }
        "create_conflict_free_calendar_event" => {
            contains_any(&["calendar", "event", "conflict-free", "schedule"])
        }
        "send_system_email" => has_send_intent(prompt),
        _ => false,
    }
}

pub(super) fn validate_objective_bindings(
    prompt: &str,
    ir: &WorkflowIr,
) -> Result<(), WorkflowCompilerError> {
    let normalized_prompt = prompt.to_ascii_lowercase();
    let tools = ir
        .nodes
        .iter()
        .filter_map(|node| match node {
            WorkflowNode::McpTool(tool) => Some(tool),
            _ => None,
        })
        .collect::<Vec<_>>();
    validate_requested_input_paths(prompt, &tools)?;
    validate_requested_output_paths(prompt, &tools)?;
    let matching = |operation: &str| {
        tools
            .iter()
            .copied()
            .filter(|tool| {
                tool.server_name == REGISTERED_TASK_SERVER && tool.tool_name == operation
            })
            .collect::<Vec<_>>()
    };

    if has_verified_file_intent(&normalized_prompt) {
        for format in ["md", "pdf"] {
            if objective_requests_format(&normalized_prompt, format)
                && !matching("create_file").iter().any(|tool| {
                    tool.arguments
                        .pointer("/file/format")
                        .and_then(Value::as_str)
                        == Some(format)
                })
            {
                return Err(objective_mismatch(format!(
                    "The workflow requests a verified {format} artifact but does not use oomu_task_tools/create_file for that format."
                )));
            }
        }
        if tools.iter().any(|tool| {
            tool.tool_name == "write_file"
                || (tool.server_name == TASKFLOW_NATIVE_SERVER
                    && matches!(
                        tool.tool_name.as_str(),
                        "write_markdown_report" | "preview_report"
                    ))
        }) {
            return Err(objective_mismatch(
                "Verified Project artifacts must use create_file, not a generic filesystem or workflow-sandbox writer."
                    .to_string(),
            ));
        }
    }

    if has_official_source_intent(&normalized_prompt) {
        let required = if normalized_prompt.contains("at least two") {
            2
        } else {
            1
        };
        let official_fetches = matching("fetch_official_page");
        let unique_urls = official_fetches
            .iter()
            .filter_map(|tool| tool.arguments.get("url").and_then(Value::as_str))
            .collect::<HashSet<_>>();
        if unique_urls.len() < required {
            return Err(objective_mismatch(format!(
                "The workflow requires {required} distinct primary or official source fetch step(s)."
            )));
        }
        if has_supplier_analysis_intent(&normalized_prompt) {
            validate_official_agent_evidence_bounds(ir, &official_fetches)?;
        }
    }

    if has_supplier_analysis_intent(&normalized_prompt) {
        let analysis_tools = matching("analyze_supplier_exceptions");
        let bound_analysis_tools = analysis_tools
            .into_iter()
            .filter(|tool| supplier_analysis_is_bound_to_requested_read(prompt, &tools, tool))
            .collect::<Vec<_>>();
        if bound_analysis_tools.is_empty() {
            return Err(objective_mismatch(
                "The workflow must calculate typed supplier rate variances from the exact requested Project-file bytes."
                    .to_string(),
            ));
        }
        if has_supplier_exception_intent(&normalized_prompt)
            && !has_typed_supplier_exception_branch(ir, &bound_analysis_tools)
        {
            return Err(objective_mismatch(
                "Supplier exception actions must branch on the typed analyze_supplier_exceptions hasException result, not Agent prose."
                    .to_string(),
            ));
        }
    }

    if has_milestone_analysis_intent(&normalized_prompt) {
        let bound_analysis_tools = matching("analyze_project_milestones")
            .into_iter()
            .filter(|tool| milestone_analysis_is_bound_to_requested_read(prompt, &tools, tool))
            .collect::<Vec<_>>();
        if bound_analysis_tools.is_empty() {
            return Err(objective_mismatch(
                "The workflow must identify unfinished milestones through typed analysis of the exact requested Project-file bytes."
                    .to_string(),
            ));
        }
    }

    if has_verified_file_intent(&normalized_prompt) {
        validate_artifact_evidence_order(prompt, ir, &tools)?;
    }
    if has_operations_brief_synthesis_intent(&normalized_prompt) {
        validate_operations_brief_synthesis(prompt, ir, &tools)?;
    }

    let calendar_tools = matching("create_conflict_free_calendar_event");
    if has_calendar_create_intent(&normalized_prompt) && calendar_tools.is_empty() {
        return Err(objective_mismatch(
            "The workflow requests a conflict-free Calendar event but has no verified Calendar create step."
                .to_string(),
        ));
    }
    let send_tools = matching("send_system_email");
    if has_send_intent(&normalized_prompt) {
        if send_tools.is_empty() {
            return Err(objective_mismatch(
                "The workflow requests a sent email but has no send_system_email step.".to_string(),
            ));
        }
        if tools
            .iter()
            .any(|tool| tool.tool_name == "draft_system_email")
        {
            return Err(objective_mismatch(
                "A sent-email objective cannot substitute an unsent draft action.".to_string(),
            ));
        }
    }

    super::effect_request_validation::validate(ir, &tools, &calendar_tools, &send_tools, prompt)
        .map_err(objective_mismatch)?;

    if normalized_prompt.contains("explicit user approval")
        || normalized_prompt.contains("require explicit")
    {
        for effect in calendar_tools.into_iter().chain(send_tools) {
            validate_effect_denial_continuation(ir, effect)?;
        }
    }
    Ok(())
}

pub(super) fn validate_static_evidence_synthesis(
    ir: &WorkflowIr,
) -> Result<(), WorkflowCompilerError> {
    let tools = ir
        .nodes
        .iter()
        .filter_map(|node| match node {
            WorkflowNode::McpTool(tool) => Some(tool),
            _ => None,
        })
        .collect::<Vec<_>>();
    let project_reads = tools
        .iter()
        .copied()
        .filter(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER && tool.tool_name == "read_project_file"
        })
        .collect::<Vec<_>>();
    let analysis_input_path = |operation: &str| {
        tools
            .iter()
            .copied()
            .find(|tool| tool.server_name == REGISTERED_TASK_SERVER && tool.tool_name == operation)
            .and_then(|analysis| {
                let content = analysis.arguments.get("content").and_then(Value::as_str)?;
                project_reads.iter().copied().find_map(|read| {
                    let expected = format!("{{{{nodes.{}.output.data.content}}}}", read.id);
                    (content == expected)
                        .then(|| read.arguments.get("path").and_then(Value::as_str))
                        .flatten()
                })
            })
    };
    let supplier_path = analysis_input_path("analyze_supplier_exceptions");
    let milestone_path = analysis_input_path("analyze_project_milestones");
    let official_count = tools
        .iter()
        .filter(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER && tool.tool_name == "fetch_official_page"
        })
        .count();
    let formats = tools
        .iter()
        .filter(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER && tool.tool_name == "create_file"
        })
        .filter_map(|tool| {
            tool.arguments
                .pointer("/file/format")
                .and_then(Value::as_str)
        })
        .collect::<HashSet<_>>();
    let (Some(supplier_path), Some(milestone_path)) = (supplier_path, milestone_path) else {
        return Ok(());
    };
    if supplier_path == milestone_path {
        return Err(objective_mismatch(
            "Distinct typed input roles must remain bound to distinct Project-file reads."
                .to_string(),
        ));
    }
    if official_count < 2 || !formats.contains("md") || !formats.contains("pdf") {
        return Ok(());
    }
    let objective = format!(
        "Read `{supplier_path}` as the supplier rate input and `{milestone_path}` as the milestone status input. Retrieve current information from at least two official web sources. Reconcile supplier rate variances, identify unfinished milestones and milestone risks, and create one Markdown file and matching PDF."
    );
    validate_operations_brief_synthesis(&objective, ir, &tools)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObjectiveCodeSpan {
    value: String,
    start: usize,
    end: usize,
}

fn validate_requested_input_paths(
    prompt: &str,
    tools: &[&McpToolNode],
) -> Result<(), WorkflowCompilerError> {
    let requested = unique_paths(
        objective_code_spans(prompt)
            .into_iter()
            .filter(|span| is_local_input_path(&span.value))
            .map(|span| span.value),
    );
    let mut bound_nodes = HashSet::new();
    for path in requested {
        let Some(tool) = tools.iter().copied().find(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER
                && tool.tool_name == "read_project_file"
                && tool.arguments.get("path").and_then(Value::as_str) == Some(path.as_str())
        }) else {
            return Err(objective_mismatch(format!(
                "The workflow must read the exact requested local input {} with oomu_task_tools/read_project_file.",
                compact_error(&path)
            )));
        };
        if !bound_nodes.insert(tool.id.as_str()) {
            return Err(objective_mismatch(
                "Each requested local input must be bound to its own exact read_file step."
                    .to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_requested_output_paths(
    prompt: &str,
    tools: &[&McpToolNode],
) -> Result<(), WorkflowCompilerError> {
    let mut requested = objective_code_spans(prompt)
        .into_iter()
        .filter(|span| {
            is_artifact_output_path(&span.value) && nearby_output_verb(prompt, span.start, span.end)
        })
        .map(|span| span.value)
        .collect::<Vec<_>>();
    let normalized_prompt = prompt.to_ascii_lowercase();
    if matching_pdf_is_positive(&normalized_prompt)
        && !requested.iter().any(|path| path_has_extension(path, "pdf"))
    {
        let markdown_paths = requested
            .iter()
            .filter(|path| path_has_extension(path, "md"))
            .cloned()
            .collect::<Vec<_>>();
        if let [markdown_path] = markdown_paths.as_slice() {
            requested.push(replace_extension(markdown_path, "pdf"));
        }
    }

    let mut bound_nodes = HashSet::new();
    for path in unique_paths(requested) {
        let format = path
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_ascii_lowercase())
            .unwrap_or_default();
        let Some(tool) = tools.iter().copied().find(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER
                && tool.tool_name == "create_file"
                && tool
                    .arguments
                    .pointer("/file/destinationPath")
                    .and_then(Value::as_str)
                    == Some(path.as_str())
                && tool
                    .arguments
                    .pointer("/file/format")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.eq_ignore_ascii_case(&format))
        }) else {
            return Err(objective_mismatch(format!(
                "The workflow must create the exact requested output {} with oomu_task_tools/create_file.",
                compact_error(&path)
            )));
        };
        if !bound_nodes.insert(tool.id.as_str()) {
            return Err(objective_mismatch(
                "Each requested artifact destination must be bound to its own create_file step."
                    .to_string(),
            ));
        }
    }
    Ok(())
}

fn objective_code_spans(prompt: &str) -> Vec<ObjectiveCodeSpan> {
    let mut open = None;
    let mut spans = Vec::new();
    for (index, character) in prompt.char_indices() {
        if character != '`' {
            continue;
        }
        if let Some(start) = open.take() {
            let value = normalize_prompt_path(&prompt[start..index]);
            if !value.is_empty() {
                spans.push(ObjectiveCodeSpan {
                    value,
                    start,
                    end: index,
                });
            }
        } else {
            open = Some(index + character.len_utf8());
        }
    }
    spans
}

pub(super) fn requested_local_input_paths(prompt: &str) -> Vec<String> {
    unique_paths(
        objective_code_spans(prompt)
            .into_iter()
            .filter(|span| is_local_input_path(&span.value))
            .map(|span| span.value),
    )
}

pub(super) fn requested_artifact_output_paths(prompt: &str) -> Vec<String> {
    let mut requested = objective_code_spans(prompt)
        .into_iter()
        .filter(|span| {
            is_artifact_output_path(&span.value) && nearby_output_verb(prompt, span.start, span.end)
        })
        .map(|span| span.value)
        .collect::<Vec<_>>();
    let normalized_prompt = prompt.to_ascii_lowercase();
    if matching_pdf_is_positive(&normalized_prompt)
        && !requested.iter().any(|path| path_has_extension(path, "pdf"))
    {
        let markdown_paths = requested
            .iter()
            .filter(|path| path_has_extension(path, "md"))
            .cloned()
            .collect::<Vec<_>>();
        if let [markdown_path] = markdown_paths.as_slice() {
            requested.push(replace_extension(markdown_path, "pdf"));
        }
    }
    unique_paths(requested)
}

pub(super) fn input_path_role_score(prompt: &str, path: &str, role_terms: &[&str]) -> usize {
    let normalized_prompt = prompt.to_ascii_lowercase();
    let normalized_path = path.to_ascii_lowercase();
    let path_score = role_terms
        .iter()
        .filter(|term| normalized_path.contains(**term))
        .count()
        * 256;
    let proximity_score = objective_code_spans(prompt)
        .into_iter()
        .filter(|span| span.value == path)
        .map(|span| {
            let midpoint = span.start.saturating_add(span.end).saturating_div(2);
            role_terms
                .iter()
                .flat_map(|term| normalized_prompt.match_indices(*term).map(|(at, _)| at))
                .filter_map(|at| {
                    let distance = at.abs_diff(midpoint);
                    (distance <= 192).then(|| 193 - distance)
                })
                .max()
                .unwrap_or_default()
        })
        .max()
        .unwrap_or_default();
    path_score.saturating_add(proximity_score)
}

fn normalize_prompt_path(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut characters = value.trim().chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' && matches!(characters.peek(), Some(' ' | '~' | '\\')) {
            normalized.push(characters.next().expect("peeked escaped path character"));
        } else {
            normalized.push(character);
        }
    }
    normalized
}

fn is_local_input_path(value: &str) -> bool {
    !value.contains("://")
        && ["json", "txt", "csv", "tsv", "xml"]
            .iter()
            .any(|extension| path_has_extension(value, extension))
}

fn is_artifact_output_path(value: &str) -> bool {
    !value.contains("://")
        && ["md", "pdf", "docx", "xlsx", "pptx", "txt"]
            .iter()
            .any(|extension| path_has_extension(value, extension))
}

fn path_has_extension(value: &str, extension: &str) -> bool {
    value
        .rsplit_once('.')
        .is_some_and(|(_, actual)| actual.eq_ignore_ascii_case(extension))
}

fn nearby_output_verb(prompt: &str, span_start: usize, span_end: usize) -> bool {
    let start = floor_char_boundary(prompt, span_start.saturating_sub(160));
    let end = ceil_char_boundary(prompt, span_end.saturating_add(160).min(prompt.len()));
    let context = prompt[start..end].to_ascii_lowercase();
    let verbs = ["create", "write", "save", "generate"];
    verbs.iter().any(|verb| context.contains(verb))
        && !verbs.iter().any(|verb| action_is_negated(&context, verb))
}

fn matching_pdf_is_positive(prompt: &str) -> bool {
    let Some(offset) = prompt.find("matching pdf") else {
        return false;
    };
    let start = floor_char_boundary(prompt, offset.saturating_sub(160));
    let end = ceil_char_boundary(prompt, (offset + 12 + 160).min(prompt.len()));
    let context = &prompt[start..end];
    ["create", "write", "save", "generate"]
        .iter()
        .all(|verb| !action_is_negated(context, verb))
}

pub(super) fn action_is_negated(context: &str, verb: &str) -> bool {
    let context = context.to_ascii_lowercase();
    let (gerund, participle) = match verb {
        "create" => ("creating", "created"),
        "write" => ("writing", "written"),
        "save" => ("saving", "saved"),
        "generate" => ("generating", "generated"),
        "read" => ("reading", "read"),
        "retrieve" => ("retrieving", "retrieved"),
        "research" => ("researching", "researched"),
        "fetch" => ("fetching", "fetched"),
        "send" => ("sending", "sent"),
        other => (other, other),
    };
    let active = [
        format!("do not {verb}"),
        format!("don't {verb}"),
        format!("never {verb}"),
        format!("must not {verb}"),
        format!("mustn't {verb}"),
        format!("should not {verb}"),
        format!("shouldn't {verb}"),
        format!("shall not {verb}"),
        format!("cannot {verb}"),
        format!("can't {verb}"),
        format!("without {gerund}"),
    ]
    .iter()
    .any(|phrase| context.contains(phrase));
    let passive = [
        format!("must not be {participle}"),
        format!("mustn't be {participle}"),
        format!("should not be {participle}"),
        format!("shouldn't be {participle}"),
        format!("shall not be {participle}"),
        format!("cannot be {participle}"),
        format!("can't be {participle}"),
    ]
    .iter()
    .any(|phrase| context.contains(phrase));
    let no_subject = context.contains("no ")
        && [
            format!(" should be {participle}"),
            format!(" must be {participle}"),
            format!(" may be {participle}"),
        ]
        .iter()
        .any(|phrase| context.contains(phrase));
    active || passive || no_subject
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn replace_extension(path: &str, extension: &str) -> String {
    path.rsplit_once('.')
        .map(|(stem, _)| format!("{stem}.{extension}"))
        .unwrap_or_else(|| format!("{path}.{extension}"))
}

fn unique_paths(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn validate_effect_denial_continuation(
    ir: &WorkflowIr,
    effect: &McpToolNode,
) -> Result<(), WorkflowCompilerError> {
    let permission_id = ir
        .edges
        .iter()
        .find(|edge| edge.target_node_id == effect.id && edge.source_port == "approved")
        .map(|edge| edge.source_node_id.as_str())
        .filter(|source| {
            ir.nodes.iter().any(|node| {
                matches!(node, WorkflowNode::Permission(permission)
                    if permission.id == *source
                        && matches!(permission.on_denied, PermissionDeniedBehavior::Branch))
            })
        })
        .ok_or_else(|| denial_error(effect))?;
    let denied_target = ir
        .edges
        .iter()
        .find(|edge| edge.source_node_id == permission_id && edge.source_port == "denied")
        .map(|edge| edge.target_node_id.as_str())
        .ok_or_else(|| denial_error(effect))?;
    let reaches_output = ir.nodes.iter().any(|node| {
        matches!(node, WorkflowNode::Output(_)) && node_reaches(ir, denied_target, node.id())
    });
    if !reaches_output || node_reaches(ir, denied_target, &effect.id) {
        return Err(denial_error(effect));
    }
    Ok(())
}

fn validate_artifact_evidence_order(
    prompt: &str,
    ir: &WorkflowIr,
    tools: &[&McpToolNode],
) -> Result<(), WorkflowCompilerError> {
    let requested_paths = unique_paths(
        objective_code_spans(prompt)
            .into_iter()
            .filter(|span| is_local_input_path(&span.value))
            .map(|span| span.value),
    );
    let project_reads = requested_paths
        .iter()
        .filter_map(|path| {
            tools.iter().copied().find(|tool| {
                tool.server_name == REGISTERED_TASK_SERVER
                    && tool.tool_name == "read_project_file"
                    && tool.arguments.get("path").and_then(Value::as_str) == Some(path.as_str())
            })
        })
        .collect::<Vec<_>>();
    let official_fetches = tools
        .iter()
        .copied()
        .filter(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER && tool.tool_name == "fetch_official_page"
        })
        .collect::<Vec<_>>();
    let creates = tools
        .iter()
        .copied()
        .filter(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER && tool.tool_name == "create_file"
        })
        .collect::<Vec<_>>();
    for create in &creates {
        let content = create
            .arguments
            .pointer("/file/content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let producers = template_references(content)
            .into_iter()
            .filter_map(|reference| referenced_node_id(&reference).map(str::to_string))
            .collect::<HashSet<_>>();
        if producers.is_empty()
            || !producers
                .iter()
                .all(|producer| node_reaches(ir, producer, &create.id))
        {
            return Err(objective_mismatch(format!(
                "The {} step must consume a real upstream content producer.",
                compact_error(&create.label)
            )));
        }
        for evidence in project_reads.iter().chain(&official_fetches) {
            if !producers
                .iter()
                .any(|producer| node_reaches(ir, &evidence.id, producer))
                || !node_reaches(ir, &evidence.id, &create.id)
            {
                return Err(objective_mismatch(format!(
                    "The {} evidence step must complete before content synthesis and {}.",
                    compact_error(&evidence.label),
                    compact_error(&create.label)
                )));
            }
        }
        if !ir.nodes.iter().any(|node| {
            matches!(node, WorkflowNode::Output(_)) && node_reaches(ir, &create.id, node.id())
        }) {
            return Err(objective_mismatch(format!(
                "The {} step must reach a truthful terminal output.",
                compact_error(&create.label)
            )));
        }
    }

    let normalized_prompt = prompt.to_ascii_lowercase();
    if has_supplier_exception_intent(&normalized_prompt) {
        validate_supplier_effect_order(ir, tools, &project_reads, &creates)?;
    }
    Ok(())
}

fn validate_supplier_effect_order(
    ir: &WorkflowIr,
    tools: &[&McpToolNode],
    project_reads: &[&McpToolNode],
    creates: &[&McpToolNode],
) -> Result<(), WorkflowCompilerError> {
    let analyses = tools
        .iter()
        .copied()
        .filter(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER
                && tool.tool_name == "analyze_supplier_exceptions"
        })
        .collect::<Vec<_>>();
    if analyses.iter().any(|analysis| {
        !project_reads
            .iter()
            .any(|read| node_reaches(ir, &read.id, &analysis.id))
    }) {
        return Err(objective_mismatch(
            "The exact Project fixture read must complete before typed supplier analysis."
                .to_string(),
        ));
    }
    let typed_conditionals = analyses
        .iter()
        .flat_map(|analysis| {
            let input = format!("{{{{nodes.{}.output.data}}}}", analysis.id);
            ir.nodes.iter().filter(move |node| {
                matches!(node, WorkflowNode::Conditional(conditional)
                    if conditional.condition.trim() == "$.hasException == true"
                        && conditional.input_mapping.as_deref() == Some(input.as_str()))
            })
        })
        .collect::<Vec<_>>();
    let effects = tools
        .iter()
        .copied()
        .filter(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER
                && matches!(
                    tool.tool_name.as_str(),
                    "create_conflict_free_calendar_event" | "send_system_email"
                )
        })
        .collect::<Vec<_>>();
    if creates
        .iter()
        .any(|create| exact_agent_data_producer(ir, create).is_none())
    {
        return Err(objective_mismatch(
            "The supplier report must consume exact content from a read-only evidence validator bound to the synthesis Agent and typed evidence."
                .to_string(),
        ));
    }
    for effect in &effects {
        let has_verified_report_path = creates.iter().any(|create| {
            let expected = format!(
                "{{{{nodes.{}.output.data.structuredContent.path}}}}",
                create.id
            );
            json_contains(&effect.arguments, &expected)
        });
        if !has_verified_report_path {
            return Err(objective_mismatch(format!(
                "The {} step must link the verified report's canonical create_file path.",
                compact_error(&effect.label)
            )));
        }
    }
    for create in creates {
        if !typed_conditionals
            .iter()
            .any(|conditional| node_reaches(ir, &create.id, conditional.id()))
            || effects
                .iter()
                .any(|effect| !node_reaches(ir, &create.id, &effect.id))
        {
            return Err(objective_mismatch(
                "The verified supplier report must precede the typed exception decision and every approved follow-up effect."
                    .to_string(),
            ));
        }
    }
    Ok(())
}

fn json_contains(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.contains(needle),
        Value::Array(items) => items.iter().any(|item| json_contains(item, needle)),
        Value::Object(object) => object.values().any(|item| json_contains(item, needle)),
        _ => false,
    }
}

fn validate_operations_brief_synthesis(
    prompt: &str,
    ir: &WorkflowIr,
    tools: &[&McpToolNode],
) -> Result<(), WorkflowCompilerError> {
    let supplier_analyses = tools
        .iter()
        .copied()
        .filter(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER
                && tool.tool_name == "analyze_supplier_exceptions"
                && supplier_analysis_is_bound_to_requested_read(prompt, tools, tool)
        })
        .collect::<Vec<_>>();
    let milestone_analyses = tools
        .iter()
        .copied()
        .filter(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER
                && tool.tool_name == "analyze_project_milestones"
                && milestone_analysis_is_bound_to_requested_read(prompt, tools, tool)
        })
        .collect::<Vec<_>>();
    let official_fetches = tools
        .iter()
        .copied()
        .filter(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER && tool.tool_name == "fetch_official_page"
        })
        .collect::<Vec<_>>();
    if supplier_analyses.len() != 1 || milestone_analyses.len() != 1 {
        return Err(objective_mismatch(
            "The operations brief requires one exact typed supplier analysis and one exact typed milestone analysis."
                .to_string(),
        ));
    }
    let distinct_official_urls = official_fetches
        .iter()
        .filter_map(|tool| tool.arguments.get("url").and_then(Value::as_str))
        .collect::<HashSet<_>>();
    if distinct_official_urls.len() < 2 {
        return Err(objective_mismatch(
            "The operations brief requires at least two distinct official-source URLs.".to_string(),
        ));
    }

    let mut estimated_evidence_bytes = 4 * 1024usize;
    let mut expected_inputs = HashSet::new();
    for analysis in supplier_analyses.iter().chain(&milestone_analyses) {
        expected_inputs.insert(format!("{{{{nodes.{}.output.data}}}}", analysis.id));
        estimated_evidence_bytes = estimated_evidence_bytes.saturating_add(
            if analysis.tool_name == "analyze_supplier_exceptions" {
                crate::tools::supplier_exception::MAX_ANALYSIS_JSON_BYTES
            } else {
                crate::tools::milestone_analysis::MAX_ANALYSIS_JSON_BYTES
            },
        );
    }
    for fetch in &official_fetches {
        let maximum = fetch
            .arguments
            .get("maxContentChars")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| {
                (1_000..=crate::tools::official_page::MAX_AGENT_EVIDENCE_CHARS).contains(value)
            })
            .ok_or_else(|| {
                objective_mismatch(format!(
                    "The {} step must declare maxContentChars between 1000 and {} before an Agent can consume it.",
                    compact_error(&fetch.label),
                    crate::tools::official_page::MAX_AGENT_EVIDENCE_CHARS
                ))
            })?;
        estimated_evidence_bytes = estimated_evidence_bytes.saturating_add(maximum);
        expected_inputs.insert(format!("{{{{nodes.{}.output.data}}}}", fetch.id));
    }
    if estimated_evidence_bytes
        > crate::workflow_runtime::evidence_input::WORKFLOW_AGENT_VARIABLE_MAX_BYTES
    {
        return Err(objective_mismatch(format!(
            "The operations-brief evidence budget is {estimated_evidence_bytes} bytes, above the reliable local synthesis limit of {} bytes.",
            crate::workflow_runtime::evidence_input::WORKFLOW_AGENT_VARIABLE_MAX_BYTES
        )));
    }

    let creates = tools
        .iter()
        .copied()
        .filter(|tool| {
            tool.server_name == REGISTERED_TASK_SERVER
                && tool.tool_name == "create_file"
                && matches!(
                    tool.arguments
                        .pointer("/file/format")
                        .and_then(Value::as_str),
                    Some("md" | "pdf")
                )
        })
        .collect::<Vec<_>>();
    let mut shared_validators = HashSet::new();
    let mut shared_producers: Option<HashSet<String>> = None;
    for create in &creates {
        if let Some((validator_id, _)) = exact_report_validation_binding(ir, create) {
            shared_validators.insert(validator_id.to_string());
        }
        let producers = exact_agent_data_producer(ir, create)
            .map(str::to_string)
            .into_iter()
            .collect::<HashSet<_>>();
        shared_producers = Some(match shared_producers {
            None => producers,
            Some(mut shared) => {
                shared.retain(|producer| producers.contains(producer));
                shared
            }
        });
    }
    let valid_synthesis = shared_validators.len() == 1
        && shared_producers
            .unwrap_or_default()
            .into_iter()
            .any(|producer_id| {
                ir.nodes.iter().any(|node| {
                    let (actual_inputs, declared_input_count) = match node {
                        WorkflowNode::Agent(agent) if agent.id == producer_id => (
                            agent
                                .input_mappings
                                .values()
                                .map(|value| value.trim().to_string())
                                .collect::<HashSet<_>>(),
                            agent.input_mappings.len(),
                        ),
                        WorkflowNode::McpTool(tool)
                            if tool.id == producer_id
                                && tool.server_name == REGISTERED_TASK_SERVER
                                && tool.tool_name == "compose_evidence_report" =>
                        {
                            let Some(arguments) = tool.arguments.as_object() else {
                                return false;
                            };
                            let Some(supplier) =
                                arguments.get("supplierAnalysis").and_then(Value::as_str)
                            else {
                                return false;
                            };
                            let Some(milestone) =
                                arguments.get("milestoneAnalysis").and_then(Value::as_str)
                            else {
                                return false;
                            };
                            let Some(receipts) = arguments
                                .get("officialPageReceipts")
                                .and_then(Value::as_array)
                            else {
                                return false;
                            };
                            let mut inputs = HashSet::from([
                                supplier.trim().to_string(),
                                milestone.trim().to_string(),
                            ]);
                            for receipt in receipts {
                                let Some(receipt) = receipt.as_str() else {
                                    return false;
                                };
                                inputs.insert(receipt.trim().to_string());
                            }
                            (inputs, arguments.len().saturating_sub(1) + receipts.len())
                        }
                        _ => return false,
                    };
                    declared_input_count == expected_inputs.len()
                        && actual_inputs == expected_inputs
                        && supplier_analyses
                            .iter()
                            .chain(&milestone_analyses)
                            .chain(&official_fetches)
                            .all(|evidence| node_reaches(ir, &evidence.id, &producer_id))
                        && creates
                            .iter()
                            .all(|create| node_reaches(ir, &producer_id, &create.id))
                })
            });
    if !valid_synthesis {
        return Err(objective_mismatch(
            "Both operations-brief files must share one exact synthesis step whose only inputs are the typed supplier result, typed milestone result, and bounded official-source receipts."
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_official_agent_evidence_bounds(
    ir: &WorkflowIr,
    official_fetches: &[&McpToolNode],
) -> Result<(), WorkflowCompilerError> {
    for fetch in official_fetches {
        let expected = format!("{{{{nodes.{}.output.data}}}}", fetch.id);
        let consumed_mappings = ir.nodes.iter().filter_map(|node| match node {
            WorkflowNode::Agent(agent) => Some(agent.input_mappings.values()),
            _ => None,
        });
        let consumed = consumed_mappings
            .flatten()
            .filter(|mapping| {
                template_references(mapping)
                    .iter()
                    .any(|reference| referenced_node_id(reference) == Some(fetch.id.as_str()))
            })
            .collect::<Vec<_>>();
        if consumed.is_empty() {
            continue;
        }
        let maximum = fetch
            .arguments
            .get("maxContentChars")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok());
        if !maximum.is_some_and(|value| {
            (1_000..=crate::tools::official_page::MAX_AGENT_EVIDENCE_CHARS).contains(&value)
        }) || consumed.iter().any(|mapping| mapping.trim() != expected)
        {
            return Err(objective_mismatch(format!(
                "The {} step must give its Agent the exact bounded receipt with maxContentChars between 1000 and {}.",
                compact_error(&fetch.label),
                crate::tools::official_page::MAX_AGENT_EVIDENCE_CHARS
            )));
        }
    }
    Ok(())
}

fn node_reaches(ir: &WorkflowIr, start: &str, target: &str) -> bool {
    if start == target {
        return true;
    }
    let mut stack = vec![start];
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if !seen.insert(node) {
            continue;
        }
        for edge in ir.edges.iter().filter(|edge| edge.source_node_id == node) {
            if edge.target_node_id == target {
                return true;
            }
            stack.push(edge.target_node_id.as_str());
        }
    }
    false
}

fn has_verified_file_intent(prompt: &str) -> bool {
    (prompt.contains(".md") || prompt.contains("pdf"))
        && (prompt.contains("create") || prompt.contains("write"))
}

fn objective_requests_format(prompt: &str, format: &str) -> bool {
    match format {
        "md" => prompt.contains(".md") || prompt.contains("markdown"),
        "pdf" => prompt.contains("pdf"),
        _ => false,
    }
}

fn has_official_source_intent(prompt: &str) -> bool {
    (prompt.contains("primary") || prompt.contains("official"))
        && (prompt.contains("source") || prompt.contains("web"))
        && (prompt.contains("current") || prompt.contains("retrieve"))
}

fn has_supplier_analysis_intent(prompt: &str) -> bool {
    prompt.contains("supplier")
        && (prompt.contains("rate variance")
            || prompt.contains("rate variances")
            || prompt.contains("reconcile supplier")
            || (prompt.contains("active quote") && prompt.contains("historical settled rate")))
}

fn has_supplier_exception_intent(prompt: &str) -> bool {
    prompt.contains("supplier")
        && prompt.contains("active quote")
        && prompt.contains("historical settled rate")
}

fn has_milestone_analysis_intent(prompt: &str) -> bool {
    prompt.contains("milestone")
        && (prompt.contains("unfinished")
            || prompt.contains("milestone risk")
            || prompt.contains("milestone status"))
}

fn has_operations_brief_synthesis_intent(prompt: &str) -> bool {
    has_supplier_analysis_intent(prompt)
        && has_milestone_analysis_intent(prompt)
        && has_official_source_intent(prompt)
        && has_verified_file_intent(prompt)
}

fn has_typed_supplier_exception_branch(ir: &WorkflowIr, tools: &[&McpToolNode]) -> bool {
    tools.iter().any(|tool| {
        let expected_input = format!("{{{{nodes.{}.output.data}}}}", tool.id);
        ir.nodes.iter().any(|node| {
            matches!(node, WorkflowNode::Conditional(conditional)
                if conditional.condition.trim() == "$.hasException == true"
                    && conditional.input_mapping.as_deref() == Some(expected_input.as_str()))
        })
    })
}

fn supplier_analysis_is_bound_to_requested_read(
    prompt: &str,
    tools: &[&McpToolNode],
    analysis: &McpToolNode,
) -> bool {
    analysis_is_bound_to_requested_read(prompt, tools, analysis, "supplier")
}

fn milestone_analysis_is_bound_to_requested_read(
    prompt: &str,
    tools: &[&McpToolNode],
    analysis: &McpToolNode,
) -> bool {
    analysis_is_bound_to_requested_read(prompt, tools, analysis, "milestone")
}

fn analysis_is_bound_to_requested_read(
    prompt: &str,
    tools: &[&McpToolNode],
    analysis: &McpToolNode,
    path_term: &str,
) -> bool {
    let content = analysis.arguments.get("content").and_then(Value::as_str);
    let role_terms: &[&str] = match path_term {
        "supplier" => &["supplier", "quote", "vendor", "rate", "pricing"],
        "milestone" => &["milestone", "roadmap", "project status", "delivery target"],
        _ => &[path_term],
    };
    let candidates = objective_code_spans(prompt)
        .into_iter()
        .filter(|span| is_local_input_path(&span.value))
        .map(|span| {
            let score = input_path_role_score(prompt, &span.value, role_terms);
            (span, score)
        })
        .collect::<Vec<_>>();
    let best_score = candidates
        .iter()
        .map(|(_, score)| *score)
        .max()
        .unwrap_or(0);
    candidates
        .into_iter()
        .filter(|(_, score)| best_score > 0 && *score == best_score)
        .map(|(span, _)| span)
        .any(|span| {
            tools.iter().any(|tool| {
                let expected = format!("{{{{nodes.{}.output.data.content}}}}", tool.id);
                tool.server_name == REGISTERED_TASK_SERVER
                    && tool.tool_name == "read_project_file"
                    && tool.arguments.get("path").and_then(Value::as_str)
                        == Some(span.value.as_str())
                    && content == Some(expected.as_str())
            })
        })
}

fn has_calendar_create_intent(prompt: &str) -> bool {
    prompt.contains("calendar")
        && prompt.contains("event")
        && (prompt.contains("create") || prompt.contains("schedule"))
}

fn objective_mismatch(message: String) -> WorkflowCompilerError {
    WorkflowCompilerError::topological_anomaly(OBJECTIVE_BINDING_CODE, message)
}

fn denial_error(effect: &McpToolNode) -> WorkflowCompilerError {
    WorkflowCompilerError::topological_anomaly(
        EFFECT_DENIAL_CODE,
        format!(
            "The {} step requires an explicit permission node whose denied branch reaches a truthful terminal output without running the effect.",
            compact_error(&effect.label)
        ),
    )
}
