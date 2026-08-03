use super::{
    hash_approval_token, normalize_mcp_text_writer_arguments, resolve_json_templates,
    PermissionDecision, ResolvePermissionRequest, WorkflowRuntimeError,
};
use crate::mcp::client::McpToolApprovalBinding;
use crate::{
    db::PersistenceEngine,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tool_security::classify_mcp_tool_call,
    tools::task_tool_runtime::{
        self, TaskToolExecutionContext, TaskToolValidation, ValidatedTaskToolRequest,
    },
    workflow_ir::{
        ExecutionInstance, ExecutionStatus, McpToolNode, NodeExecutionPayload, PermissionKind,
        PermissionNode, WorkflowEdge, WorkflowNode,
    },
};
#[cfg(test)]
use crate::{p0_contracts::EvidenceClass, tools::task_runtime};
#[cfg(test)]
use rusqlite::params;
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tauri::Manager;

mod effect_journal;
use effect_journal::{
    claim_effect_execution, halt_for_effect_verification, release_unchanged_effect, reserve_effect,
    verify_effect, EffectReservation,
};
#[cfg(test)]
use effect_journal::{reserve_effect_for_task, ReservedEffect};

pub(super) const SERVER_NAME: &str = "oomu_task_tools";

pub(super) fn is_registered_task_server(server_name: &str) -> bool {
    server_name.trim().eq_ignore_ascii_case(SERVER_NAME)
}

fn is_exact_workflow_effect(server_name: &str, tool_name: &str) -> bool {
    crate::routines::reviewed_effect_requires_explicit_approval(server_name, tool_name)
}

pub(super) fn validate_registered_task_arguments(
    tool_name: &str,
    arguments: &Value,
) -> Result<TaskToolValidation, WorkflowRuntimeError> {
    task_tool_runtime::validate_if_registered(tool_name, arguments.clone())
        .ok_or_else(|| {
            WorkflowRuntimeError::new(
                "workflow_registered_task_unknown",
                format!("Registered Workflow tool {tool_name} is unavailable."),
            )
        })?
        .map_err(|message| WorkflowRuntimeError::new("workflow_registered_task_invalid", message))
}

pub(super) fn reviewed_routine_scope_for_call(
    approval_ledger: Option<&PersistenceEngine>,
    instance_id: &str,
    tool: &McpToolNode,
    arguments: &Value,
) -> Result<bool, WorkflowRuntimeError> {
    let Some(approval_ledger) = approval_ledger else {
        return Ok(false);
    };
    let reviewed = crate::routines::verify_reviewed_workflow_scope(
        approval_ledger,
        instance_id,
        &tool.id,
        &tool.tool_name,
        arguments,
    )
    .map_err(WorkflowRuntimeError::runtime)?;
    let required = crate::routines::reviewed_workflow_scope_required(approval_ledger, instance_id)
        .map_err(WorkflowRuntimeError::runtime)?;
    let exact_effect_can_request_approval =
        is_exact_workflow_effect(&tool.server_name, &tool.tool_name);
    if required && !reviewed && !exact_effect_can_request_approval {
        return Err(WorkflowRuntimeError::new(
            "workflow_routine_scope_denied",
            "This scheduled step is outside the Workflow scope you reviewed.".to_string(),
        ));
    }
    Ok(reviewed)
}

pub(super) fn exact_permission_effect_context(
    permission: &PermissionNode,
    outgoing: &HashMap<&str, Vec<&WorkflowEdge>>,
    node_by_id: &HashMap<&str, &WorkflowNode>,
    memory: &HashMap<String, Value>,
) -> Result<Option<Value>, WorkflowRuntimeError> {
    let approved = outgoing
        .get(permission.id.as_str())
        .into_iter()
        .flatten()
        .filter(|edge| edge.source_port == "approved")
        .collect::<Vec<_>>();
    let [edge] = approved.as_slice() else {
        return Ok(None);
    };
    let Some(WorkflowNode::McpTool(tool)) = node_by_id.get(edge.target_node_id.as_str()).copied()
    else {
        return Ok(None);
    };
    if !matches!(permission.permission, PermissionKind::McpTool)
        || !is_exact_workflow_effect(&tool.server_name, &tool.tool_name)
    {
        return Ok(None);
    }
    let arguments = normalize_mcp_text_writer_arguments(
        &tool.tool_name,
        resolve_json_templates(&tool.arguments, memory)?,
    );
    if is_registered_task_server(&tool.server_name) {
        validate_registered_task_arguments(&tool.tool_name, &arguments)?;
    }
    let classification = classify_mcp_tool_call(&tool.server_name, &tool.tool_name, None);
    Ok(Some(serde_json::json!({
        "actionType": "mcp_tool",
        "serverName": tool.server_name,
        "toolName": tool.tool_name,
        "arguments": arguments,
        "actionLabel": permission.label,
        "capabilityReason": permission.reason,
        "capabilityRiskTier": classification.tier.as_str(),
        "approvalScope": "exact_workflow_effect",
    })))
}

pub(super) fn approved_permission_predecessor(
    tool: &McpToolNode,
    incoming: &HashMap<&str, Vec<&WorkflowEdge>>,
    selected_edges: &HashSet<String>,
    node_by_id: &HashMap<&str, &WorkflowNode>,
    payloads: &HashMap<String, NodeExecutionPayload>,
) -> Option<String> {
    if !is_exact_workflow_effect(&tool.server_name, &tool.tool_name) {
        return None;
    }
    let predecessors = incoming
        .get(tool.id.as_str())
        .into_iter()
        .flatten()
        .filter(|edge| edge.source_port == "approved" && selected_edges.contains(&edge.id))
        .filter_map(|edge| {
            matches!(
                node_by_id.get(edge.source_node_id.as_str()).copied(),
                Some(WorkflowNode::Permission(_))
            )
            .then_some(edge.source_node_id.as_str())
        })
        .filter(|node_id| {
            payloads.get(*node_id).is_some_and(|payload| {
                payload.status == ExecutionStatus::Completed
                    && payload
                        .output
                        .as_ref()
                        .and_then(|output| output.pointer("/data/decision"))
                        .and_then(Value::as_str)
                        == Some("approve")
            })
        })
        .collect::<Vec<_>>();
    let [predecessor] = predecessors.as_slice() else {
        return None;
    };
    payloads
        .contains_key(*predecessor)
        .then(|| (*predecessor).to_string())
}

pub(super) fn permission_pause_context(
    permission: &PermissionNode,
    exact_effect: Option<Value>,
    input: Option<Value>,
    token_hash: String,
    approval_token: &str,
) -> Result<(Value, Value), WorkflowRuntimeError> {
    // Only the closed semantic summary is rendered. Inputs remain in the
    // durable pause record for exact continuation, never in approval copy.
    let context = exact_effect.unwrap_or_else(|| {
        serde_json::json!({
            "actionType": "workflow_permission",
            "permissionKind": &permission.permission,
            "actionLabel": &permission.label,
            "capabilityReason": &permission.reason,
        })
    });
    let mut paused = context.clone();
    let object = paused
        .as_object_mut()
        .ok_or_else(WorkflowRuntimeError::approval_state_invalid)?;
    object.insert("nodeId".to_string(), Value::String(permission.id.clone()));
    object.insert(
        "permission".to_string(),
        serde_json::to_value(&permission.permission)
            .map_err(WorkflowRuntimeError::serialization)?,
    );
    object.insert(
        "reason".to_string(),
        Value::String(permission.reason.clone()),
    );
    object.insert(
        "onDenied".to_string(),
        serde_json::to_value(&permission.on_denied).map_err(WorkflowRuntimeError::serialization)?,
    );
    object.insert("input".to_string(), input.unwrap_or(Value::Null));
    object.insert("approvalTokenHash".to_string(), Value::String(token_hash));
    object.insert(
        "approvalToken".to_string(),
        Value::String(approval_token.to_string()),
    );
    object.insert(
        "approvalMessage".to_string(),
        Value::String(permission.reason.clone()),
    );
    object.insert("approvalContext".to_string(), context.clone());
    Ok((context, paused))
}

pub(super) fn record_bound_mcp_approval(
    request: &ResolvePermissionRequest,
    instance: &ExecutionInstance,
    persistence: &PersistenceEngine,
    node_id: &str,
) -> Result<(), WorkflowRuntimeError> {
    let Some(context) = instance.pause_context.as_ref() else {
        return Ok(());
    };
    if context.get("actionType").and_then(Value::as_str) != Some("mcp_tool") {
        return Ok(());
    }
    let tool_name = context
        .get("toolName")
        .and_then(Value::as_str)
        .ok_or_else(WorkflowRuntimeError::approval_state_invalid)?;
    let arguments = context
        .get("arguments")
        .ok_or_else(WorkflowRuntimeError::approval_state_invalid)?;
    let binding = context
        .get("mcpApprovalBinding")
        .and_then(|value| serde_json::from_value::<McpToolApprovalBinding>(value.clone()).ok());
    let material = workflow_mcp_approval_material(arguments, binding.as_ref());
    let decision = match request.decision {
        PermissionDecision::Approve => "approve",
        PermissionDecision::Reject => "deny",
    };
    persistence
        .record_workflow_approval(
            &hash_approval_token(&request.approval_token),
            &instance.id,
            node_id,
            tool_name,
            &material,
            decision,
        )
        .map_err(WorkflowRuntimeError::database)?;

    if request.decision != PermissionDecision::Approve {
        return Ok(());
    }
    let compiled = persistence
        .load_compiled_workflow(&instance.workflow_id, Some(instance.workflow_version))
        .map_err(WorkflowRuntimeError::database)?;
    let Some(tool) = compiled.workflow_ir.nodes.iter().find_map(|candidate| {
        matches!(candidate, WorkflowNode::McpTool(tool) if tool.id == node_id).then(|| {
            let WorkflowNode::McpTool(tool) = candidate else {
                unreachable!("matched MCP Workflow node")
            };
            tool
        })
    }) else {
        return Err(WorkflowRuntimeError::approval_state_invalid());
    };
    let Some(material) = workflow_version_mcp_approval_material(tool, arguments, binding.as_ref())?
    else {
        return Ok(());
    };
    persistence
        .record_workflow_version_approval(
            &hash_approval_token(&request.approval_token),
            &instance.workflow_id,
            instance.workflow_version,
            node_id,
            &tool.server_name,
            &tool.tool_name,
            &material,
        )
        .map_err(WorkflowRuntimeError::database)
}

pub(super) fn workflow_mcp_approval_material(
    arguments: &Value,
    approval_binding: Option<&McpToolApprovalBinding>,
) -> Value {
    serde_json::json!({
        "arguments": arguments,
        "approvalBinding": approval_binding,
    })
}

/// Produce the durable review scope for a saved Workflow version.
///
/// Generic MCP calls remain exact: their stable server/tool/destination
/// binding and resolved arguments must match. Registered Task tools can carry
/// generated content and timestamped filenames between runs, so their scope is
/// instead bound to the immutable authored arguments plus only the resolved
/// target fields. Exact native effects (Calendar, Mail, and other explicitly
/// approved operations) are intentionally excluded and continue to ask on
/// every run.
pub(super) fn workflow_version_mcp_approval_material(
    tool: &McpToolNode,
    arguments: &Value,
    approval_binding: Option<&McpToolApprovalBinding>,
) -> Result<Option<Value>, WorkflowRuntimeError> {
    if !is_registered_task_server(&tool.server_name) {
        return Ok(Some(serde_json::json!({
            "schema": "workflow_version_mcp_review_v1",
            "serverName": tool.server_name,
            "toolName": tool.tool_name,
            "exactCall": workflow_mcp_approval_material(arguments, approval_binding),
        })));
    }

    validate_registered_task_arguments(&tool.tool_name, arguments)?;
    let Some(approval_tier) = task_tool_runtime::approval_tier(&tool.tool_name) else {
        return Ok(None);
    };
    if approval_tier == crate::tools::task_tool_runtime::TaskToolApprovalTier::Explicit {
        return Ok(None);
    }
    let runtime_targets = bounded_runtime_targets(&tool.arguments, arguments)?;
    Ok(Some(serde_json::json!({
        "schema": "workflow_version_task_tool_review_v1",
        "serverName": tool.server_name,
        "toolName": tool.tool_name,
        "authoredArguments": tool.arguments,
        "runtimeTargets": runtime_targets,
    })))
}

fn bounded_runtime_targets(
    authored: &Value,
    resolved: &Value,
) -> Result<Value, WorkflowRuntimeError> {
    fn collect(
        authored: Option<&Value>,
        resolved: &Value,
        key: Option<&str>,
    ) -> Result<Option<Value>, WorkflowRuntimeError> {
        if key.is_some_and(is_material_target_key) {
            return normalize_material_target(authored, resolved).map(Some);
        }
        match resolved {
            Value::Object(object) => {
                let authored_object = authored.and_then(Value::as_object);
                let mut targets = Map::new();
                for (child_key, child_value) in object {
                    if let Some(value) = collect(
                        authored_object.and_then(|candidate| candidate.get(child_key)),
                        child_value,
                        Some(child_key),
                    )? {
                        targets.insert(child_key.clone(), value);
                    }
                }
                Ok((!targets.is_empty()).then_some(Value::Object(targets)))
            }
            Value::Array(items) => {
                let authored_items = authored.and_then(Value::as_array);
                let mut targets = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    if let Some(value) = collect(
                        authored_items.and_then(|values| values.get(index)),
                        item,
                        None,
                    )? {
                        targets.push(value);
                    }
                }
                Ok((!targets.is_empty()).then_some(Value::Array(targets)))
            }
            _ => Ok(None),
        }
    }

    Ok(collect(Some(authored), resolved, None)?.unwrap_or_else(|| serde_json::json!({})))
}

fn is_material_target_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "url"
            | "urls"
            | "fallbackurl"
            | "fallbackurls"
            | "host"
            | "hostname"
            | "origin"
            | "path"
            | "filepath"
            | "targetpath"
            | "destinationpath"
            | "outputpath"
            | "attachmentpath"
            | "directory"
            | "destinationdirectory"
            | "outputdirectory"
            | "root"
            | "projectroot"
            | "recipient"
            | "to"
            | "cc"
            | "bcc"
            | "channel"
            | "conversation"
            | "calendar"
            | "calendarname"
            | "subject"
            | "eventtitle"
    )
}

fn normalize_material_target(
    authored: Option<&Value>,
    resolved: &Value,
) -> Result<Value, WorkflowRuntimeError> {
    match (authored, resolved) {
        (Some(Value::String(authored)), Value::String(resolved))
            if authored.contains(task_tool_runtime::TASK_RUN_TIMESTAMP_TOKEN) =>
        {
            if authored == resolved || runtime_timestamp_matches_template(authored, resolved) {
                Ok(Value::String(authored.clone()))
            } else {
                Err(WorkflowRuntimeError::new(
                    "workflow_reusable_approval_target_changed",
                    "A timestamped Workflow target resolved outside its saved filename pattern."
                        .to_string(),
                ))
            }
        }
        (_, resolved) => Ok(resolved.clone()),
    }
}

fn runtime_timestamp_matches_template(template: &str, candidate: &str) -> bool {
    let Some((prefix, suffix)) = template.split_once(task_tool_runtime::TASK_RUN_TIMESTAMP_TOKEN)
    else {
        return false;
    };
    let Some(timestamp) = candidate
        .strip_prefix(prefix)
        .and_then(|value| value.strip_suffix(suffix))
    else {
        return false;
    };
    timestamp.len() == 16
        && timestamp
            .bytes()
            .enumerate()
            .all(|(index, byte)| match index {
                4 | 7 | 13 => byte == b'-',
                10 => byte == b'_',
                _ => byte.is_ascii_digit(),
            })
}

#[cfg(test)]
mod reusable_approval_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn timestamped_task_output_reuses_immutable_pattern_not_generated_content() {
        let _ = crate::artifacts::register_file_task_tool();
        let tool = McpToolNode {
            id: "write-md".to_string(),
            label: "Write operations brief".to_string(),
            server_name: SERVER_NAME.to_string(),
            tool_name: "create_file".to_string(),
            arguments: json!({
                "file": {
                    "title": "Operations brief",
                    "content": "{{nodes.brief.output}}",
                    "locale": "en-US",
                    "format": "md",
                    "destinationPath": "/Project/output/operations_brief_<YYYY-MM-DD_HH-mm>.md"
                }
            }),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: None,
        };
        let unresolved = workflow_version_mcp_approval_material(&tool, &tool.arguments, None)
            .expect("the unchanged saved template is valid before Task runtime resolution")
            .expect("create_file requires reusable Workflow review");
        let first = workflow_version_mcp_approval_material(
            &tool,
            &json!({
                "file": {
                    "title": "Operations brief",
                    "content": "First generated brief",
                    "locale": "en-US",
                    "format": "md",
                    "destinationPath": "/Project/output/operations_brief_2026-07-22_11-45.md"
                }
            }),
            None,
        )
        .unwrap()
        .unwrap();
        let later = workflow_version_mcp_approval_material(
            &tool,
            &json!({
                "file": {
                    "title": "Operations brief",
                    "content": "New evidence from a later run",
                    "locale": "en-US",
                    "format": "md",
                    "destinationPath": "/Project/output/operations_brief_2026-07-23_08-05.md"
                }
            }),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(unresolved, first);
        assert_eq!(first, later);
        assert_eq!(
            first.pointer("/runtimeTargets/file/destinationPath"),
            Some(&json!(
                "/Project/output/operations_brief_<YYYY-MM-DD_HH-mm>.md"
            ))
        );

        let escaped = workflow_version_mcp_approval_material(
            &tool,
            &json!({
                "file": {
                    "title": "Operations brief",
                    "content": "same content",
                    "locale": "en-US",
                    "format": "md",
                    "destinationPath": "/tmp/operations_brief_2026-07-23_08-05.md"
                }
            }),
            None,
        )
        .expect_err("a runtime target outside the authored pattern must not reuse approval");
        assert_eq!(escaped.code, "workflow_reusable_approval_target_changed");
    }

    #[test]
    fn explicit_calendar_and_mail_effects_never_create_reusable_grants() {
        let _ = crate::tools::system_calendar_event::register_task_tool();
        let _ = crate::tools::system_mail_send::register_task_tool();
        let calendar = McpToolNode {
            id: "calendar".to_string(),
            label: "Create event".to_string(),
            server_name: SERVER_NAME.to_string(),
            tool_name: "create_conflict_free_calendar_event".to_string(),
            arguments: json!({
                "calendarName": "OOMU Test",
                "title": "Supplier Exception Follow-up",
                "day": "next_weekday",
                "windowStartLocal": "14:00",
                "windowEndLocal": "18:00",
                "durationMinutes": 30,
                "location": "",
                "notes": "Report",
                "availability": "tentative"
            }),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: None,
        };
        let mail = McpToolNode {
            id: "send".to_string(),
            label: "Send report".to_string(),
            server_name: SERVER_NAME.to_string(),
            tool_name: "send_system_email".to_string(),
            arguments: json!({
                "to": "recipient@example.com",
                "subject": "OOMU Test — Supplier Exception",
                "body": "Report attached"
            }),
            input_schema: None,
            output_schema: None,
            system_timeout_ms: None,
        };

        assert!(
            workflow_version_mcp_approval_material(&calendar, &calendar.arguments, None)
                .unwrap()
                .is_none()
        );
        assert!(
            workflow_version_mcp_approval_material(&mail, &mail.arguments, None)
                .unwrap()
                .is_none()
        );
    }
}

pub(super) fn verify_predecessor_mcp_approval(
    persistence: &PersistenceEngine,
    instance_id: &str,
    permission_node_id: &str,
    tool: &McpToolNode,
    arguments: &Value,
    approval_binding: Option<&McpToolApprovalBinding>,
) -> Result<bool, WorkflowRuntimeError> {
    let material = workflow_mcp_approval_material(arguments, approval_binding);
    let exact_match = persistence
        .verify_workflow_approval(instance_id, permission_node_id, &tool.tool_name, &material)
        .map_err(WorkflowRuntimeError::database)?;
    if exact_match || !is_exact_workflow_effect(&tool.server_name, &tool.tool_name) {
        return Ok(exact_match);
    }

    // The authored permission step binds the exact resolved arguments before
    // the MCP client mints its short-lived native Shield binding. Verify that
    // human decision, then let the client activate the binding for this one
    // call. Arbitrary MCP tools cannot use this compatibility path.
    let authored_material = workflow_mcp_approval_material(arguments, None);
    persistence
        .verify_workflow_approval(
            instance_id,
            permission_node_id,
            &tool.tool_name,
            &authored_material,
        )
        .map_err(WorkflowRuntimeError::database)
}

pub(super) fn execute_registered_task_tool(
    app: &tauri::AppHandle,
    execution_id: &str,
    node_id: &str,
    label: &str,
    tool_name: &str,
    arguments: Value,
    timeout_ms: u64,
) -> Result<Value, WorkflowRuntimeError> {
    let validation = validate_registered_task_arguments(tool_name, &arguments)?;
    let unresolved = ValidatedTaskToolRequest {
        operation: registered_operation(tool_name)?,
        arguments: validation.arguments,
        potentially_effectful: validation.potentially_effectful,
    };
    let persistence = app.state::<PersistenceEngine>().inner().clone();
    let request = task_tool_runtime::resolve(&persistence, Some(execution_id), unresolved, &[])
        .map_err(|message| {
            WorkflowRuntimeError::new("workflow_registered_task_resolution_failed", message)
        })?;
    let reviewed_scope_required =
        crate::routines::reviewed_workflow_scope_required(&persistence, execution_id)
            .map_err(WorkflowRuntimeError::runtime)?;
    if reviewed_scope_required
        && !task_tool_runtime::requires_explicit_approval(tool_name)
        && !crate::routines::verify_reviewed_workflow_scope(
            &persistence,
            execution_id,
            node_id,
            tool_name,
            &request.arguments,
        )
        .map_err(WorkflowRuntimeError::runtime)?
    {
        return Err(WorkflowRuntimeError::new(
            "workflow_routine_scope_denied",
            "This scheduled step resolved outside the Workflow scope you reviewed.".to_string(),
        ));
    }
    let identity = app
        .state::<crate::sovereign_identity::SovereignIdentity>()
        .inner()
        .clone();
    let app_handle = app.clone();
    let effect = request
        .potentially_effectful()
        .then(|| {
            reserve_effect(
                &persistence,
                execution_id,
                node_id,
                tool_name,
                &request.arguments,
            )
        })
        .transpose()?;
    let effect = match effect {
        Some(EffectReservation::Replay(result)) => return Ok(result),
        Some(EffectReservation::Execute(effect)) => {
            if let Some(result) = claim_effect_execution(&persistence, &effect)? {
                return Ok(result);
            }
            Some(effect)
        }
        None => None,
    };
    let execution_id = execution_id.to_string();
    let tool_name_owned = tool_name.to_string();
    let worker_label = label.to_string();
    let call = async move {
        tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            task_tool_runtime::execute(
                TaskToolExecutionContext {
                    persistence: &persistence,
                    identity: &identity,
                    app: Some(&app_handle),
                    execution_id: Some(&execution_id),
                    plan_id: None,
                    objective: Some(&worker_label),
                    session_id: None,
                    model_route: None,
                },
                request,
            ),
        )
        .await
    };
    let response = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(call),
        Err(_) => match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(call),
            Err(raw) => {
                let error = WorkflowRuntimeError::runtime(raw.to_string());
                if let Some(effect) = effect.as_ref() {
                    return halt_for_effect_verification(
                        app.state::<PersistenceEngine>().inner(),
                        effect,
                        error.code,
                    );
                }
                return Err(error);
            }
        },
    };
    let response = match response {
        Ok(Ok(response)) => response,
        Ok(Err(raw)) => {
            let normalized = task_tool_runtime::normalize_agent_error(&tool_name_owned, &raw);
            let retry_safe_unchanged =
                task_tool_runtime::parse_retry_safe_unchanged_error(&tool_name_owned, &normalized)
                    .is_some();
            let error = task_tool_error(&tool_name_owned, &raw);
            if let Some(effect) = effect.as_ref() {
                if retry_safe_unchanged {
                    release_unchanged_effect(
                        app.state::<PersistenceEngine>().inner(),
                        effect,
                        error.code,
                    )?;
                    return Err(error);
                }
                return halt_for_effect_verification(
                    app.state::<PersistenceEngine>().inner(),
                    effect,
                    error.code,
                );
            }
            return Err(error);
        }
        Err(_) => {
            let error = WorkflowRuntimeError::node_timeout(node_id, label, timeout_ms);
            if let Some(effect) = effect.as_ref() {
                return halt_for_effect_verification(
                    app.state::<PersistenceEngine>().inner(),
                    effect,
                    error.code,
                );
            }
            return Err(error);
        }
    };
    let result = match verified_result(response) {
        Ok(result) => result,
        Err(error) => {
            if let Some(effect) = effect.as_ref() {
                return halt_for_effect_verification(
                    app.state::<PersistenceEngine>().inner(),
                    effect,
                    error.code,
                );
            }
            return Err(error);
        }
    };
    if let Some(effect) = effect.as_ref() {
        if let Err(error) = verify_effect(app.state::<PersistenceEngine>().inner(), effect, &result)
        {
            return halt_for_effect_verification(
                app.state::<PersistenceEngine>().inner(),
                effect,
                error.code,
            );
        }
    }
    Ok(result)
}

fn registered_operation(tool_name: &str) -> Result<&'static str, WorkflowRuntimeError> {
    task_tool_runtime::registered_operations()
        .into_iter()
        .find(|operation| *operation == tool_name)
        .ok_or_else(|| {
            WorkflowRuntimeError::new(
                "workflow_registered_task_unknown",
                format!("Registered Workflow tool {tool_name} is unavailable."),
            )
        })
}

fn verified_result(response: ExecuteCommandResponse) -> Result<Value, WorkflowRuntimeError> {
    if !matches!(response.status, CommandStatus::Completed) || !response.verified {
        return Err(WorkflowRuntimeError::new(
            "workflow_registered_task_unverified",
            format!(
                "Registered Workflow tool {} did not return a verified result.",
                response.operation
            ),
        ));
    }
    let operation = response.operation;
    let parsed = serde_json::from_str::<Value>(&response.message)
        .unwrap_or_else(|_| Value::String(response.message));
    let mut result = match parsed {
        Value::Object(object) => object,
        value => {
            let mut object = Map::new();
            object.insert("result".to_string(), value);
            object
        }
    };
    if operation == "create_file" && !result.contains_key("structuredContent") {
        result.insert(
            "structuredContent".to_string(),
            Value::Object(result.clone()),
        );
    }
    Ok(Value::Object(result))
}

fn task_tool_error(operation: &str, raw: &str) -> WorkflowRuntimeError {
    let normalized = task_tool_runtime::normalize_agent_error(operation, raw);
    let Some(error) = task_tool_runtime::parse_agent_error(&normalized) else {
        return WorkflowRuntimeError::new("workflow_registered_task_failed", normalized);
    };
    WorkflowRuntimeError::new(stable_error_code(&error.code), error.message)
}

fn stable_error_code(code: &str) -> &'static str {
    match code {
        "network_unavailable" => "network_unavailable",
        "dns_resolution_failed" => "dns_resolution_failed",
        "network_timeout" => "network_timeout",
        "connection_failed" => "connection_failed",
        "calendar_action_denied" => "calendar_action_denied",
        "calendar_not_found" => "calendar_not_found",
        "calendar_permission_denied" => "calendar_permission_denied",
        "calendar_permission_restricted" => "calendar_permission_restricted",
        "calendar_permission_write_only" => "calendar_permission_write_only",
        "calendar_permission_unavailable" => "calendar_permission_unavailable",
        "calendar_authorization_timeout" => "calendar_authorization_timeout",
        "mail_automation_permission_required" => "mail_automation_permission_required",
        "mail_automation_timeout" => "mail_automation_timeout",
        "mail_automation_unavailable" => "mail_automation_unavailable",
        "mail_send_unverified" | "mail_send_result_unverified" => "mail_send_result_unverified",
        "file_creation_failed" => "file_creation_failed",
        "official_page_fetch_failed" => "official_page_fetch_failed",
        _ => "workflow_registered_task_failed",
    }
}

#[cfg(test)]
mod effect_journal_tests {
    use super::*;
    use crate::{
        p0_contracts::{TaskId, TaskRunId},
        projects::{CreateProjectRequest, ProjectDataPolicy},
    };
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn exact_workflow_effect_allowlist_matches_supported_approval_tools_only() {
        assert!(is_exact_workflow_effect(
            "macos_applescript",
            "draft_system_email"
        ));
        assert!(is_exact_workflow_effect(
            "oomu_task_tools",
            "create_conflict_free_calendar_event"
        ));
        assert!(is_exact_workflow_effect(
            "oomu_task_tools",
            "send_system_email"
        ));
        assert!(!is_exact_workflow_effect(
            "macos_applescript",
            "read_system_emails"
        ));
        assert!(!is_exact_workflow_effect(
            "macos_applescript",
            "send_system_email"
        ));
    }

    struct JournalFixture {
        root: PathBuf,
        persistence: PersistenceEngine,
        task_run_id: String,
    }

    impl JournalFixture {
        fn new(label: &str) -> Self {
            crate::tasks::register_runtime_bridge().unwrap();
            let task_run_id = TaskRunId::new().to_string();
            let root = std::env::temp_dir().join(format!(
                "oomu-workflow-effect-{label}-{}",
                task_run_id.trim_start_matches("taskrun_")
            ));
            let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
            let project = crate::projects::repository::create(
                &persistence,
                CreateProjectRequest {
                    name: format!("Workflow effect {label}"),
                    description: String::new(),
                    data_policy: ProjectDataPolicy::LocalOnly,
                },
            )
            .unwrap();
            let task_id = TaskId::new().to_string();
            persistence
                .open_connection()
                .unwrap()
                .execute(
                    "INSERT INTO task_runs(task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'workflow',?4,'running','routine',?2,'Effect journal test',1,1,'reconciled')",
                    params![task_run_id, task_id, project.project_id, format!("execution-{label}")],
                )
                .unwrap();
            Self {
                root,
                persistence,
                task_run_id,
            }
        }

        fn reserve(
            &self,
            node_id: &str,
            operation: &str,
            arguments: &Value,
        ) -> Result<EffectReservation, WorkflowRuntimeError> {
            reserve_effect_for_task(
                &self.persistence,
                &self.task_run_id,
                node_id,
                operation,
                arguments,
            )
        }

        fn finish(self) {
            drop(self.persistence);
            let _ = std::fs::remove_dir_all(self.root);
        }
    }

    fn executable(reservation: EffectReservation) -> ReservedEffect {
        match reservation {
            EffectReservation::Execute(effect) => effect,
            EffectReservation::Replay(_) => panic!("expected a fresh effect reservation"),
        }
    }

    #[test]
    fn verified_effect_result_is_durably_replayed_after_runtime_restart() {
        let fixture = JournalFixture::new("verified-replay");
        let arguments = json!({"to":"recipient@example.com","subject":"Status"});
        let effect = executable(
            fixture
                .reserve("mail", "draft_system_email", &arguments)
                .unwrap(),
        );
        assert!(claim_effect_execution(&fixture.persistence, &effect)
            .unwrap()
            .is_none());
        let result = json!({
            "operation": "draft_system_email",
            "verified": true,
            "draftId": "draft-verified-1",
        });
        verify_effect(&fixture.persistence, &effect, &result).unwrap();

        let root = fixture.root.clone();
        let task_run_id = fixture.task_run_id.clone();
        drop(fixture.persistence);
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let replay = reserve_effect_for_task(
            &persistence,
            &task_run_id,
            "mail",
            "draft_system_email",
            &arguments,
        )
        .unwrap();
        match replay {
            EffectReservation::Replay(replayed) => assert_eq!(replayed, result),
            EffectReservation::Execute(_) => panic!("verified Mail effect must not execute twice"),
        }
        let (state, receipt_count): (String, i64) = persistence
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT e.state,(SELECT COUNT(*) FROM task_events WHERE task_run_id=e.task_run_id AND json_extract(event_json,'$.eventType')='workflow.effect.verified') FROM task_effects e WHERE e.task_run_id=?1 AND e.idempotency_key=?2",
                params![task_run_id, effect.key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "verified");
        assert_eq!(receipt_count, 1);
        drop(persistence);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ambiguous_calendar_and_mail_effects_require_verification_without_reexecution() {
        let fixture = JournalFixture::new("ambiguous-native-effects");
        for (node_id, operation) in [
            ("calendar", "create_system_calendar_event"),
            ("mail", "draft_system_email"),
        ] {
            let arguments = json!({"identity": operation});
            let effect = executable(fixture.reserve(node_id, operation, &arguments).unwrap());
            assert!(claim_effect_execution(&fixture.persistence, &effect)
                .unwrap()
                .is_none());
            let error = fixture.reserve(node_id, operation, &arguments).unwrap_err();
            assert_eq!(error.code, "workflow_effect_verification_required");
            let state: String = fixture
                .persistence
                .open_connection()
                .unwrap()
                .query_row(
                    "SELECT state FROM task_effects WHERE task_run_id=?1 AND idempotency_key=?2",
                    params![fixture.task_run_id, effect.key],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(state, "executed");
        }
        let (state, recovery_state, audit_count): (String, String, i64) = fixture
            .persistence
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT state,recovery_state,(SELECT COUNT(*) FROM task_events WHERE task_run_id=task_runs.task_run_id AND json_extract(event_json,'$.eventType')='workflow.effect.verification_required') FROM task_runs WHERE task_run_id=?1",
                params![fixture.task_run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, "blocked");
        assert_eq!(recovery_state, "recoverable");
        assert_eq!(audit_count, 2);
        fixture.finish();
    }

    #[test]
    fn event_first_receipt_recovers_the_crash_window_without_reexecuting() {
        let fixture = JournalFixture::new("event-first-crash-window");
        let arguments = json!({"calendarName":"Work","title":"Review"});
        let effect = executable(
            fixture
                .reserve("calendar", "create_system_calendar_event", &arguments)
                .unwrap(),
        );
        claim_effect_execution(&fixture.persistence, &effect).unwrap();
        let result = json!({
            "operation": "create_system_calendar_event",
            "verified": true,
            "eventId": "event-verified-1",
        });
        let digest = crate::foundation::digest::sha256_hex(&serde_json::to_vec(&result).unwrap());
        task_runtime::record_event(
            &fixture.persistence,
            &fixture.task_run_id,
            "workflow.effect.verified",
            EvidenceClass::VerifiedPostcondition,
            json!({
                "idempotencyKey": effect.key.clone(),
                "effectKind": effect.operation.clone(),
                "resultDigest": digest,
                "result": result.clone(),
            }),
        )
        .unwrap();

        let replay = fixture
            .reserve("calendar", "create_system_calendar_event", &arguments)
            .unwrap();
        match replay {
            EffectReservation::Replay(replayed) => assert_eq!(replayed, result),
            EffectReservation::Execute(_) => panic!("receipt-backed Calendar effect was replayed"),
        }
        let state: String = fixture
            .persistence
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT state FROM task_effects WHERE task_run_id=?1 AND idempotency_key=?2",
                params![fixture.task_run_id, effect.key],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "verified");
        fixture.finish();
    }

    #[test]
    fn verified_unchanged_failure_releases_only_its_exact_effect_claim() {
        let fixture = JournalFixture::new("verified-unchanged");
        let arguments = json!({"calendarName":"Missing","title":"Review"});
        let effect = executable(
            fixture
                .reserve("calendar", "create_system_calendar_event", &arguments)
                .unwrap(),
        );
        claim_effect_execution(&fixture.persistence, &effect).unwrap();
        release_unchanged_effect(&fixture.persistence, &effect, "calendar_not_found").unwrap();
        let count: i64 = fixture
            .persistence
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM task_effects WHERE task_run_id=?1 AND idempotency_key=?2",
                params![fixture.task_run_id, effect.key],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert!(matches!(
            fixture
                .reserve("calendar", "create_system_calendar_event", &arguments)
                .unwrap(),
            EffectReservation::Execute(_)
        ));
        fixture.finish();
    }

    #[test]
    fn receipt_integrity_failure_enters_recoverable_verification_only_state() {
        let fixture = JournalFixture::new("receipt-integrity");
        let arguments = json!({"to":"recipient@example.com","subject":"Status"});
        let effect = executable(
            fixture
                .reserve("mail", "draft_system_email", &arguments)
                .unwrap(),
        );
        claim_effect_execution(&fixture.persistence, &effect).unwrap();
        verify_effect(
            &fixture.persistence,
            &effect,
            &json!({
                "operation": "draft_system_email",
                "verified": true,
                "draftId": "draft-original",
            }),
        )
        .unwrap();
        let connection = fixture.persistence.open_connection().unwrap();
        let raw: String = connection
            .query_row(
                "SELECT event_json FROM task_events WHERE task_run_id=?1 AND json_extract(event_json,'$.eventType')='workflow.effect.verified'",
                params![fixture.task_run_id],
                |row| row.get(0),
            )
            .unwrap();
        let mut event: Value = serde_json::from_str(&raw).unwrap();
        event["payload"]["result"]["draftId"] = json!("draft-tampered");
        connection
            .execute(
                "UPDATE task_events SET event_json=?2 WHERE task_run_id=?1 AND json_extract(event_json,'$.eventType')='workflow.effect.verified'",
                params![fixture.task_run_id, serde_json::to_string(&event).unwrap()],
            )
            .unwrap();
        drop(connection);

        let error = fixture
            .reserve("mail", "draft_system_email", &arguments)
            .unwrap_err();
        assert_eq!(error.code, "workflow_effect_verification_required");
        let (state, recovery_state, effect_state): (String, String, String) = fixture
            .persistence
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT state,recovery_state,(SELECT state FROM task_effects WHERE task_run_id=task_runs.task_run_id AND idempotency_key=?2) FROM task_runs WHERE task_run_id=?1",
                params![fixture.task_run_id, effect.key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, "blocked");
        assert_eq!(recovery_state, "recoverable");
        assert_eq!(effect_state, "executed");
        fixture.finish();
    }

    #[test]
    fn verified_create_file_exposes_canonical_structured_path() {
        let result = verified_result(ExecuteCommandResponse {
            operation: "create_file".to_string(),
            status: CommandStatus::Completed,
            message: json!({
                "path": "/approved/report.md",
                "sha256": "abc",
                "byteLength": 12
            })
            .to_string(),
            metrics: None,
            claims: Vec::new(),
            verified: true,
            model_used: None,
        })
        .expect("verified create_file receipt");
        assert_eq!(
            result.pointer("/structuredContent/path"),
            Some(&json!("/approved/report.md"))
        );
        assert_eq!(result.get("path"), Some(&json!("/approved/report.md")));
        assert!(result.get("operation").is_none());
        assert!(result.get("verified").is_none());
    }

    #[test]
    fn production_analysis_and_official_receipts_cross_the_strict_report_boundary() {
        let supplier_fixture = r#"{
          "audit_year": 2026,
          "quarter": "Q2",
          "suppliers": [
            {"name":"North Harbor Logistics","historical_settled_rate":45000,"active_quote":46500,"status":"PENDING_RECONCILIATION"}
          ]
        }"#;
        let milestone_fixture = r#"[
          {"milestone_id":"R1","name":"Security review","target_date":"2026-07-06","status":"COMPLETED","owner":"Morgan Lee"},
          {"milestone_id":"R2","name":"Release readiness","target_date":"2026-07-15","status":"IN_PROGRESS","owner":"Sam Rivera","dependencies":["R1"]}
        ]"#;
        let supplier_analysis =
            crate::tools::supplier_exception::analyze_supplier_fixture(supplier_fixture)
                .expect("production supplier analysis");
        let milestone_analysis =
            crate::tools::milestone_analysis::analyze_milestone_fixture(milestone_fixture)
                .expect("production milestone analysis");
        let official_content = "Current transport conditions show limited delays.";
        let official_receipt =
            serde_json::to_value(crate::tools::official_page::OfficialPageReceipt {
                requested_url: "https://transport.example.gov/current".to_string(),
                selected_url: "https://transport.example.gov/current".to_string(),
                attempted_urls: vec!["https://transport.example.gov/current".to_string()],
                fallback_used: false,
                final_url: "https://transport.example.gov/current".to_string(),
                accessed_at_utc: "2026-07-21T14:06:07.000Z".to_string(),
                status_code: 200,
                content_type: "text/html".to_string(),
                content: official_content.to_string(),
                content_sha256: crate::foundation::digest::sha256_hex(official_content.as_bytes()),
                content_bytes: official_content.len(),
                content_truncated: false,
            })
            .expect("production official-page receipt");

        let verified_output = |operation: &str, message: Value| {
            verified_result(ExecuteCommandResponse {
                operation: operation.to_string(),
                status: CommandStatus::Completed,
                message: message.to_string(),
                metrics: None,
                claims: Vec::new(),
                verified: true,
                model_used: None,
            })
            .expect("verified registered-task output")
        };
        let supplier_analysis = verified_output("analyze_supplier_exceptions", supplier_analysis);
        let milestone_analysis = verified_output("analyze_project_milestones", milestone_analysis);
        let official_receipt = verified_output("fetch_official_page", official_receipt);

        for output in [&supplier_analysis, &milestone_analysis, &official_receipt] {
            assert!(output.get("operation").is_none());
            assert!(output.get("verified").is_none());
        }

        let content = r#"# Executive summary

The 2026 Q2 review found one exception across one supplier.

## Supplier variance

| Supplier | Historical settled rate | Active quote | Variance | Status |
|---|---:|---:|---:|---|
| North Harbor Logistics | $45,000 | $46,500 | $1,500 | PENDING_RECONCILIATION |

## Milestone risks

| ID | Milestone | Target date | Status | Owner | Dependencies |
|---|---|---|---|---|---|
| R1 | Security review | 2026-07-06 | COMPLETED | Morgan Lee | None |
| R2 | Release readiness | 2026-07-15 | IN_PROGRESS | Sam Rivera | R1 |

## Current evidence

- https://transport.example.gov/current — accessed 2026-07-21T14:06:07.000Z

## Next actions

- Reconcile the open supplier exception.
"#;
        let _ = crate::tools::evidence_report_validation::register_task_tool();
        let validation = task_tool_runtime::validate_if_registered(
            "validate_evidence_report",
            json!({
                "content": content,
                "supplierAnalysis": supplier_analysis,
                "milestoneAnalysis": milestone_analysis,
                "officialPageReceipts": [official_receipt],
                "requiredSections": [
                    "Executive summary",
                    "Supplier variance",
                    "Milestone risks",
                    "Current evidence",
                    "Next actions"
                ]
            }),
        )
        .expect("registered report validator")
        .expect("production producer shapes satisfy the strict consumer schema");
        assert!(!validation.potentially_effectful);
    }
}
