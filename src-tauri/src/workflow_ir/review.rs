use crate::workflow_ir::{PermissionDeniedBehavior, PermissionKind, WorkflowIr, WorkflowNode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowReviewCapabilities {
    pub status: &'static str,
    pub calendar_create: bool,
    pub calendar_read: bool,
    pub email_draft: bool,
    pub email_read: bool,
    pub email_send: bool,
    pub official_web: bool,
    pub project_file_read: bool,
    pub project_file_write: bool,
}

impl WorkflowReviewCapabilities {
    fn ready() -> Self {
        Self {
            status: "ready",
            calendar_create: false,
            calendar_read: false,
            email_draft: false,
            email_read: false,
            email_send: false,
            official_web: false,
            project_file_read: false,
            project_file_write: false,
        }
    }

    fn unavailable() -> Self {
        Self {
            status: "unavailable",
            ..Self::ready()
        }
    }
}

pub fn workflow_review_capabilities(ir: &WorkflowIr) -> WorkflowReviewCapabilities {
    let mut capabilities = WorkflowReviewCapabilities::ready();
    for workflow_node in &ir.nodes {
        let WorkflowNode::McpTool(node) = workflow_node else {
            continue;
        };
        if reviewed_effect_requires_explicit_approval(&node.server_name, &node.tool_name) {
            let requires_denial_branch =
                effect_requires_denial_branch(&node.server_name, &node.tool_name);
            if !directly_guarded_by_exact_permission(ir, &node.id, requires_denial_branch) {
                return WorkflowReviewCapabilities::unavailable();
            }
            match (node.server_name.as_str(), node.tool_name.as_str()) {
                ("oomu_task_tools", "create_conflict_free_calendar_event") => {
                    capabilities.calendar_create = true;
                }
                ("oomu_task_tools", "send_system_email") => {
                    capabilities.email_send = true;
                }
                ("macos_applescript", "draft_system_email") => {
                    capabilities.email_draft = true;
                }
                _ => return WorkflowReviewCapabilities::unavailable(),
            }
            continue;
        }
        let Some(policy) = reviewed_tool_policy(&node.server_name, &node.tool_name) else {
            return WorkflowReviewCapabilities::unavailable();
        };
        match policy.action {
            ReviewedAction::ProjectFileRead | ReviewedAction::ProjectDirectoryRead => {
                capabilities.project_file_read = true;
            }
            ReviewedAction::VerifiedDocumentWrite => {
                if node.server_name == "oomu_task_tools"
                    && node.tool_name == "create_file"
                    && !matches!(
                        node.arguments
                            .pointer("/file/format")
                            .and_then(Value::as_str),
                        Some("md" | "pdf")
                    )
                {
                    return WorkflowReviewCapabilities::unavailable();
                }
                capabilities.project_file_write = true;
            }
            ReviewedAction::OfficialPageRead => capabilities.official_web = true,
            ReviewedAction::DeterministicAnalysis => {}
            ReviewedAction::NativePersonalDataRead => match node.tool_name.as_str() {
                "read_system_calendar" => capabilities.calendar_read = true,
                "read_system_emails" => capabilities.email_read = true,
                _ => return WorkflowReviewCapabilities::unavailable(),
            },
        }
    }
    capabilities
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewedAction {
    ProjectFileRead,
    ProjectDirectoryRead,
    VerifiedDocumentWrite,
    OfficialPageRead,
    DeterministicAnalysis,
    NativePersonalDataRead,
}

#[derive(Clone, Copy)]
pub(crate) struct ToolPolicy {
    pub(crate) action: ReviewedAction,
    pub(crate) path_keys: &'static [&'static str],
    pub(crate) extensions: &'static [&'static str],
}

pub(crate) fn reviewed_effect_requires_explicit_approval(server: &str, tool: &str) -> bool {
    matches!(
        (server.trim(), tool.trim()),
        (
            "oomu_task_tools",
            "create_conflict_free_calendar_event" | "send_system_email"
        ) | ("macos_applescript", "draft_system_email")
    )
}

pub(crate) fn effect_requires_denial_branch(server: &str, tool: &str) -> bool {
    server.trim() == "oomu_task_tools"
        && matches!(
            tool.trim(),
            "create_conflict_free_calendar_event" | "send_system_email"
        )
}

pub(crate) fn directly_guarded_by_exact_permission(
    ir: &WorkflowIr,
    effect_id: &str,
    require_denial_branch: bool,
) -> bool {
    let incoming = ir
        .edges
        .iter()
        .filter(|edge| edge.target_node_id == effect_id && edge.source_port == "approved")
        .collect::<Vec<_>>();
    let [approved_edge] = incoming.as_slice() else {
        return false;
    };
    let Some(permission) = ir.nodes.iter().find_map(|node| match node {
        WorkflowNode::Permission(permission)
            if permission.id == approved_edge.source_node_id
                && matches!(permission.permission, PermissionKind::McpTool)
                && (!require_denial_branch
                    || matches!(permission.on_denied, PermissionDeniedBehavior::Branch)) =>
        {
            Some(permission)
        }
        _ => None,
    }) else {
        return false;
    };
    let approved = ir
        .edges
        .iter()
        .filter(|edge| edge.source_node_id == permission.id && edge.source_port == "approved")
        .collect::<Vec<_>>();
    let denied = ir
        .edges
        .iter()
        .filter(|edge| edge.source_node_id == permission.id && edge.source_port == "denied")
        .collect::<Vec<_>>();
    let [approved] = approved.as_slice() else {
        return false;
    };
    if approved.target_node_id != effect_id {
        return false;
    }
    match permission.on_denied {
        PermissionDeniedBehavior::Fail => !require_denial_branch,
        PermissionDeniedBehavior::Branch => {
            let [denied] = denied.as_slice() else {
                return false;
            };
            ir.nodes.iter().any(|node| {
                matches!(node, WorkflowNode::Output(_))
                    && node_reaches(ir, &denied.target_node_id, node.id())
            }) && !node_reaches(ir, &denied.target_node_id, effect_id)
        }
    }
}

fn node_reaches(ir: &WorkflowIr, start: &str, target: &str) -> bool {
    if start == target {
        return true;
    }
    let mut pending = vec![start];
    let mut seen = BTreeSet::new();
    while let Some(node_id) = pending.pop() {
        if !seen.insert(node_id) {
            continue;
        }
        for edge in ir
            .edges
            .iter()
            .filter(|edge| edge.source_node_id == node_id)
        {
            if edge.target_node_id == target {
                return true;
            }
            pending.push(edge.target_node_id.as_str());
        }
    }
    false
}

pub(crate) fn reviewed_tool_policy(server: &str, tool: &str) -> Option<ToolPolicy> {
    match (server.trim(), tool.trim()) {
        ("local_filesystem", "read_file") => Some(ToolPolicy {
            action: ReviewedAction::ProjectFileRead,
            path_keys: &["/path"],
            extensions: &[],
        }),
        ("local_filesystem", "list_directory") => Some(ToolPolicy {
            action: ReviewedAction::ProjectDirectoryRead,
            path_keys: &["/path"],
            extensions: &[],
        }),
        ("local_filesystem", "write_file") => Some(ToolPolicy {
            action: ReviewedAction::VerifiedDocumentWrite,
            path_keys: &["/path"],
            extensions: &["md"],
        }),
        ("taskflow_native", "folder_read") => Some(ToolPolicy {
            action: ReviewedAction::ProjectDirectoryRead,
            path_keys: &["/folderPath", "/folder_path", "/path"],
            extensions: &[],
        }),
        ("taskflow_native", "preview_report") => Some(ToolPolicy {
            action: ReviewedAction::ProjectFileRead,
            path_keys: &["/reportPath", "/report_path", "/path"],
            extensions: &["md"],
        }),
        ("taskflow_native", "write_markdown_report") => Some(ToolPolicy {
            action: ReviewedAction::VerifiedDocumentWrite,
            path_keys: &["/reportPath", "/report_path", "/path"],
            extensions: &["md"],
        }),
        ("oomu_task_tools", "create_file") => Some(ToolPolicy {
            action: ReviewedAction::VerifiedDocumentWrite,
            path_keys: &["/file/destinationPath"],
            extensions: &["md", "pdf"],
        }),
        ("oomu_task_tools", "read_project_file") => Some(ToolPolicy {
            action: ReviewedAction::ProjectFileRead,
            path_keys: &["/path"],
            extensions: &[],
        }),
        ("oomu_task_tools", "fetch_official_page") => Some(ToolPolicy {
            action: ReviewedAction::OfficialPageRead,
            path_keys: &[],
            extensions: &[],
        }),
        ("oomu_task_tools", "analyze_supplier_exceptions") => Some(ToolPolicy {
            action: ReviewedAction::DeterministicAnalysis,
            path_keys: &[],
            extensions: &[],
        }),
        ("oomu_task_tools", "analyze_project_milestones") => Some(ToolPolicy {
            action: ReviewedAction::DeterministicAnalysis,
            path_keys: &[],
            extensions: &[],
        }),
        ("oomu_task_tools", "compose_evidence_report" | "validate_evidence_report") => {
            Some(ToolPolicy {
                action: ReviewedAction::DeterministicAnalysis,
                path_keys: &[],
                extensions: &[],
            })
        }
        ("macos_applescript", "read_system_emails" | "read_system_calendar") => Some(ToolPolicy {
            action: ReviewedAction::NativePersonalDataRead,
            path_keys: &[],
            extensions: &[],
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario_six_review_ir() -> WorkflowIr {
        serde_json::from_value(serde_json::json!({
            "schemaVersion":"1.0.0",
            "workflowId":"wf-scenario-six-review",
            "workflowVersion":1,
            "name":"Ship Test 06 — Supplier Exception Recovery",
            "description":"Registered task tools with exact native approvals.",
            "compiler":{"model":"gemma-4-e2b-qat"},
            "nodes":[
                {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
                {"kind":"mcp_tool","id":"read","label":"Read","serverName":"oomu_task_tools","toolName":"read_project_file","arguments":{"path":"/tmp/project/suppliers.json"}},
                {"kind":"mcp_tool","id":"analyze","label":"Analyze","serverName":"oomu_task_tools","toolName":"analyze_supplier_exceptions","arguments":{"content":"{{nodes.read.output.data.content}}"}},
                {"kind":"mcp_tool","id":"source","label":"Source","serverName":"oomu_task_tools","toolName":"fetch_official_page","arguments":{"url":"https://www.eia.gov/petroleum/gasdiesel/"}},
                {"kind":"mcp_tool","id":"validate","label":"Validate","serverName":"oomu_task_tools","toolName":"validate_evidence_report","arguments":{"content":"report"}},
                {"kind":"mcp_tool","id":"write","label":"Write","serverName":"oomu_task_tools","toolName":"create_file","arguments":{"file":{"title":"Report","content":"report","locale":"en-US","format":"md","destinationPath":"ship_test_06/report.md"}}},
                {"kind":"permission","id":"calendar-approval","label":"Calendar approval","permission":"mcp_tool","reason":"Create exact event","onDenied":"branch"},
                {"kind":"mcp_tool","id":"calendar","label":"Calendar","serverName":"oomu_task_tools","toolName":"create_conflict_free_calendar_event","arguments":{"calendarName":"OOMU Test"}},
                {"kind":"permission","id":"send-approval","label":"Send approval","permission":"mcp_tool","reason":"Send exact email","onDenied":"branch"},
                {"kind":"mcp_tool","id":"send","label":"Send","serverName":"oomu_task_tools","toolName":"send_system_email","arguments":{"to":"tester@example.com"}},
                {"kind":"output","id":"done","label":"Done","inputMapping":"done","outputSchema":{"type":"string"}},
                {"kind":"output","id":"calendar-declined","label":"Calendar declined","inputMapping":"declined","outputSchema":{"type":"string"}},
                {"kind":"output","id":"send-declined","label":"Send declined","inputMapping":"declined","outputSchema":{"type":"string"}}
            ],
            "edges":[
                {"id":"calendar-approved","sourceNodeId":"calendar-approval","sourcePort":"approved","targetNodeId":"calendar"},
                {"id":"calendar-denied","sourceNodeId":"calendar-approval","sourcePort":"denied","targetNodeId":"calendar-declined"},
                {"id":"send-approved","sourceNodeId":"send-approval","sourcePort":"approved","targetNodeId":"send"},
                {"id":"send-denied","sourceNodeId":"send-approval","sourcePort":"denied","targetNodeId":"send-declined"}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn reviewed_scope_recognizes_bounded_project_milestone_analysis() {
        let policy = reviewed_tool_policy("oomu_task_tools", "analyze_project_milestones")
            .expect("registered milestone analyzer policy");
        assert_eq!(policy.action, ReviewedAction::DeterministicAnalysis);
        assert!(policy.path_keys.is_empty());
    }

    #[test]
    fn reviewed_scope_recognizes_read_only_evidence_report_validation() {
        let policy = reviewed_tool_policy("oomu_task_tools", "validate_evidence_report")
            .expect("registered evidence-report validator policy");
        assert_eq!(policy.action, ReviewedAction::DeterministicAnalysis);
        assert!(policy.path_keys.is_empty());
    }

    #[test]
    fn reviewed_scope_recognizes_deterministic_evidence_report_composition() {
        let policy = reviewed_tool_policy("oomu_task_tools", "compose_evidence_report")
            .expect("registered evidence-report composer policy");
        assert_eq!(policy.action, ReviewedAction::DeterministicAnalysis);
        assert!(policy.path_keys.is_empty());
        assert!(policy.extensions.is_empty());
    }

    #[test]
    fn scenario_six_registered_tools_have_an_authoritative_ready_projection() {
        let capabilities = workflow_review_capabilities(&scenario_six_review_ir());
        assert_eq!(capabilities.status, "ready");
        assert!(capabilities.project_file_read);
        assert!(capabilities.project_file_write);
        assert!(capabilities.official_web);
        assert!(capabilities.calendar_create);
        assert!(capabilities.email_send);
    }
}
