use super::{node_reaches, REGISTERED_TASK_SERVER};
use crate::workflow_ir::{McpToolNode, WorkflowIr, WorkflowNode};
use serde_json::Value;
use std::collections::HashSet;

pub(super) fn exact_agent_data_producer<'a>(
    ir: &'a WorkflowIr,
    create: &McpToolNode,
) -> Option<&'a str> {
    exact_report_validation_binding(ir, create).map(|(_, agent_id)| agent_id)
}

pub(super) fn exact_report_validation_binding<'a>(
    ir: &'a WorkflowIr,
    create: &McpToolNode,
) -> Option<(&'a str, &'a str)> {
    let content = create
        .arguments
        .pointer("/file/content")
        .and_then(Value::as_str)?
        .trim();
    let validator = ir.nodes.iter().find_map(|node| match node {
        WorkflowNode::McpTool(tool)
            if tool.server_name == REGISTERED_TASK_SERVER
                && tool.tool_name == "validate_evidence_report"
                && content == format!("{{{{nodes.{}.output.data.content}}}}", tool.id) =>
        {
            Some(tool)
        }
        _ => None,
    })?;
    let content_binding = validator.arguments.get("content").and_then(Value::as_str)?;
    let producer_id = exact_output_data_node_id(content_binding)
        .or_else(|| exact_output_data_content_node_id(content_binding))?;
    let supplier = exact_evidence_node(
        ir,
        validator.arguments.get("supplierAnalysis")?,
        "analyze_supplier_exceptions",
    )?;
    let milestone = match validator.arguments.get("milestoneAnalysis") {
        Some(value) => Some(exact_evidence_node(
            ir,
            value,
            "analyze_project_milestones",
        )?),
        None => None,
    };
    let official = validator
        .arguments
        .get("officialPageReceipts")
        .and_then(Value::as_array)?
        .iter()
        .map(|value| exact_evidence_node(ir, value, "fetch_official_page"))
        .collect::<Option<Vec<_>>>()?;
    let required_sections = validator
        .arguments
        .get("requiredSections")
        .and_then(Value::as_array)?;
    if official.is_empty()
        || required_sections.is_empty()
        || required_sections
            .iter()
            .any(|section| section.as_str().is_none_or(|value| value.trim().is_empty()))
    {
        return None;
    }
    let mut evidence = vec![supplier];
    evidence.extend(milestone);
    evidence.extend(official);
    let exact_producer = ir.nodes.iter().any(|node| match node {
        WorkflowNode::Agent(agent) if agent.id == producer_id => {
            exact_output_data_node_id(content_binding) == Some(producer_id)
                && agent.input_mappings.len() == evidence.len()
                && agent
                    .input_mappings
                    .values()
                    .map(|value| value.trim().to_string())
                    .collect::<HashSet<_>>()
                    == evidence
                        .iter()
                        .map(|evidence_id| format!("{{{{nodes.{evidence_id}.output.data}}}}"))
                        .collect::<HashSet<_>>()
        }
        WorkflowNode::McpTool(tool)
            if tool.id == producer_id
                && tool.server_name == REGISTERED_TASK_SERVER
                && tool.tool_name == "compose_evidence_report" =>
        {
            exact_output_data_content_node_id(content_binding) == Some(producer_id)
                && tool.arguments.as_object().is_some_and(|arguments| {
                    arguments.len() == 3
                        && arguments.get("supplierAnalysis")
                            == validator.arguments.get("supplierAnalysis")
                        && arguments.get("milestoneAnalysis")
                            == validator.arguments.get("milestoneAnalysis")
                        && arguments.get("officialPageReceipts")
                            == validator.arguments.get("officialPageReceipts")
                })
        }
        _ => false,
    });
    if !exact_producer
        || !evidence.iter().all(|evidence_id| {
            node_reaches(ir, evidence_id, producer_id)
                && node_reaches(ir, evidence_id, &validator.id)
        })
        || !node_reaches(ir, producer_id, &validator.id)
        || !node_reaches(ir, &validator.id, &create.id)
    {
        return None;
    }
    Some((&validator.id, producer_id))
}

fn exact_evidence_node<'a>(
    ir: &'a WorkflowIr,
    value: &Value,
    expected_tool: &str,
) -> Option<&'a str> {
    let node_id = exact_output_data_node_id(value.as_str()?)?;
    ir.nodes.iter().find_map(|node| match node {
        WorkflowNode::McpTool(tool)
            if tool.id == node_id
                && tool.server_name == REGISTERED_TASK_SERVER
                && tool.tool_name == expected_tool =>
        {
            Some(tool.id.as_str())
        }
        _ => None,
    })
}

fn exact_output_data_node_id(value: &str) -> Option<&str> {
    let node_id = value
        .trim()
        .strip_prefix("{{nodes.")?
        .strip_suffix(".output.data}}")?;
    (!node_id.is_empty()
        && node_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'))
    .then_some(node_id)
}

fn exact_output_data_content_node_id(value: &str) -> Option<&str> {
    let node_id = value
        .trim()
        .strip_prefix("{{nodes.")?
        .strip_suffix(".output.data.content}}")?;
    (!node_id.is_empty()
        && node_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'))
    .then_some(node_id)
}
