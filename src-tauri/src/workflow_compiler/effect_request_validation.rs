use super::specialist_composer::{
    follow_up_bindings, requests_report_attachment, FollowUpBindings,
};
use crate::workflow_ir::{McpToolNode, WorkflowIr};
use serde_json::Value;
use std::collections::HashSet;

const TASK_SERVER: &str = "oomu_task_tools";

pub(super) fn validate(
    ir: &WorkflowIr,
    tools: &[&McpToolNode],
    calendars: &[&McpToolNode],
    sends: &[&McpToolNode],
    prompt: &str,
) -> Result<(), String> {
    let Some(expected) = follow_up_bindings(prompt) else {
        return Ok(());
    };
    validate_calendar(calendars, &expected)?;
    let send = validate_mail(sends, &expected)?;
    if requests_report_attachment(prompt) && !has_verified_attachment(ir, tools, send) {
        return Err(
            "The requested mail attachment must use the verified create_file receipt path."
                .to_string(),
        );
    }
    Ok(())
}

fn validate_calendar(
    calendars: &[&McpToolNode],
    expected: &FollowUpBindings,
) -> Result<(), String> {
    let [calendar] = calendars else {
        return Err("The requested Calendar effect must bind exactly one event.".to_string());
    };
    let matches = calendar
        .arguments
        .get("calendarName")
        .and_then(Value::as_str)
        == Some(expected.calendar_name.as_str())
        && calendar.arguments.get("title").and_then(Value::as_str)
            == Some(expected.event_title.as_str())
        && calendar
            .arguments
            .get("durationMinutes")
            .and_then(Value::as_u64)
            == Some(u64::from(expected.duration_minutes))
        && calendar
            .arguments
            .get("windowStartLocal")
            .and_then(Value::as_str)
            == Some(expected.window_start_local.as_str())
        && calendar.arguments.get("day").and_then(Value::as_str) == Some("next_weekday");
    matches.then_some(()).ok_or_else(|| {
        "The Calendar effect must preserve the requested calendar, title, duration, day, and start time exactly."
            .to_string()
    })
}

fn validate_mail<'a>(
    sends: &[&'a McpToolNode],
    expected: &FollowUpBindings,
) -> Result<&'a McpToolNode, String> {
    let [send] = sends else {
        return Err("The requested mail effect must bind exactly one send.".to_string());
    };
    let matches = send.arguments.get("to").and_then(Value::as_str)
        == Some(expected.recipient.as_str())
        && send.arguments.get("subject").and_then(Value::as_str) == Some(expected.subject.as_str());
    matches.then_some(*send).ok_or_else(|| {
        "The mail effect must preserve the requested recipient and subject exactly.".to_string()
    })
}

fn has_verified_attachment(ir: &WorkflowIr, tools: &[&McpToolNode], send: &McpToolNode) -> bool {
    let attachment = send.arguments.get("attachmentPath").and_then(Value::as_str);
    tools.iter().copied().any(|tool| {
        let expected = format!(
            "{{{{nodes.{}.output.data.structuredContent.path}}}}",
            tool.id
        );
        tool.server_name == TASK_SERVER
            && tool.tool_name == "create_file"
            && attachment == Some(expected.as_str())
            && node_reaches(ir, &tool.id, &send.id)
    })
}

fn node_reaches(ir: &WorkflowIr, start: &str, target: &str) -> bool {
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
