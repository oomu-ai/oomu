use super::CreateRoutineRequest;
#[path = "authority_match.rs"]
mod authority_match;
pub(crate) use crate::workflow_ir::review::reviewed_effect_requires_explicit_approval;
use crate::workflow_ir::review::{
    directly_guarded_by_exact_permission, effect_requires_denial_branch, reviewed_tool_policy,
    ReviewedAction,
};
pub use crate::workflow_ir::review::{workflow_review_capabilities, WorkflowReviewCapabilities};
use crate::{db::PersistenceEngine, workflow_ir::WorkflowNode};
use authority_match::{arguments_template_matches, verify_path_authority};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, fs, net::IpAddr, path::Path};

pub const REVIEWED_WORKFLOW_SCOPE_MODE: &str = "reviewed_workflow_scope";
pub const TERMINAL_DELIVERY_NODE_ID: &str = "$routine_terminal_delivery";
pub const TERMINAL_DELIVERY_TOOL: &str = "routine_terminal_delivery";
const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ReviewedWorkflowScopeManifest {
    schema_version: u32,
    mode: String,
    schedule_id: String,
    workflow_id: String,
    workflow_version: u32,
    project_id: String,
    project_roots: Vec<String>,
    nodes: Vec<ReviewedNodeAuthority>,
    terminal_delivery: Option<TerminalDeliveryAuthority>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ReviewedNodeAuthority {
    node_id: String,
    server_name: String,
    tool_name: String,
    action: ReviewedAction,
    arguments_template: Value,
    path_pointer: Option<String>,
    allowed_extensions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TerminalDeliveryAuthority {
    platform: String,
    destination_sha256: String,
}

pub(super) fn derive_for_create(
    engine: &PersistenceEngine,
    schedule_id: &str,
    request: &CreateRoutineRequest,
) -> Result<Value, String> {
    require_mode_only(&request.authority)?;
    let manifest = derive_from_state(
        engine,
        schedule_id,
        &request.workflow_id,
        request.workflow_version,
        &request.project_id,
        &request.delivery_target,
    )?;
    serde_json::to_value(manifest).map_err(|error| error.to_string())
}

pub(super) fn rebind_for_duplicate(
    engine: &PersistenceEngine,
    source_schedule_id: &str,
    new_schedule_id: &str,
) -> Result<Value, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let state = connection
        .query_row(
            "SELECT workflow_id,workflow_version,project_id,delivery_target_json,authority_json FROM workflow_schedules WHERE id=?1",
            params![source_schedule_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<u32>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Routine was not found.".to_string())?;
    let source: ReviewedWorkflowScopeManifest = serde_json::from_str(&state.4)
        .map_err(|_| "Routine authority must be reviewed again.".to_string())?;
    if source.schedule_id != source_schedule_id || source.mode != REVIEWED_WORKFLOW_SCOPE_MODE {
        return Err("Routine authority must be reviewed again.".to_string());
    }
    let version = state
        .1
        .ok_or_else(|| "Routine workflow version is unavailable.".to_string())?;
    let project_id = state
        .2
        .ok_or_else(|| "Routine Project scope is unavailable.".to_string())?;
    let delivery =
        serde_json::from_str(&state.3).map_err(|_| "Routine delivery is invalid.".to_string())?;
    let rebound = derive_from_state(
        engine,
        new_schedule_id,
        &state.0,
        version,
        &project_id,
        &delivery,
    )?;
    serde_json::to_value(rebound).map_err(|error| error.to_string())
}

pub(super) fn delivery_matches_manifest(authority: &Value, delivery: &Value) -> bool {
    let Ok(manifest) = serde_json::from_value::<ReviewedWorkflowScopeManifest>(authority.clone())
    else {
        return false;
    };
    parse_terminal_delivery(delivery).ok() == Some(manifest.terminal_delivery)
}

pub fn verify_reviewed_workflow_scope(
    engine: &PersistenceEngine,
    workflow_instance_id: &str,
    node_id: &str,
    tool_name: &str,
    arguments: &Value,
) -> Result<bool, String> {
    let Some((schedule_id, workflow_id, workflow_version, project_id, delivery, authority)) =
        load_linked_state(engine, workflow_instance_id)?
    else {
        return Ok(false);
    };
    let manifest: ReviewedWorkflowScopeManifest = serde_json::from_str(&authority)
        .map_err(|_| "Stored Routine authority is invalid.".to_string())?;
    if manifest.mode != REVIEWED_WORKFLOW_SCOPE_MODE
        || manifest.schedule_id != schedule_id
        || manifest.workflow_id != workflow_id
        || manifest.workflow_version != workflow_version
        || manifest.project_id != project_id
    {
        return Ok(false);
    }
    let delivery: Value = serde_json::from_str(&delivery)
        .map_err(|_| "Stored Routine delivery is invalid.".to_string())?;
    let current = derive_from_state(
        engine,
        &schedule_id,
        &workflow_id,
        workflow_version,
        &project_id,
        &delivery,
    )?;
    if current != manifest {
        return Ok(false);
    }
    if node_id == TERMINAL_DELIVERY_NODE_ID && tool_name == TERMINAL_DELIVERY_TOOL {
        return Ok(terminal_delivery_arguments_match(
            manifest.terminal_delivery.as_ref(),
            arguments,
        ));
    }
    let Some(node) = manifest
        .nodes
        .iter()
        .find(|node| node.node_id == node_id && node.tool_name == tool_name)
    else {
        return Ok(false);
    };
    if !arguments_template_matches(node, arguments, &manifest.project_roots) {
        return Ok(false);
    }
    match node.action {
        ReviewedAction::OfficialPageRead => Ok(public_https_argument(arguments)),
        ReviewedAction::DeterministicAnalysis | ReviewedAction::NativePersonalDataRead => Ok(true),
        ReviewedAction::ProjectFileRead
        | ReviewedAction::ProjectDirectoryRead
        | ReviewedAction::VerifiedDocumentWrite => {
            verify_path_authority(node, arguments, &manifest.project_roots)
        }
    }
}

pub fn reviewed_workflow_scope_required(
    engine: &PersistenceEngine,
    workflow_instance_id: &str,
) -> Result<bool, String> {
    let Some((schedule_id, workflow_id, workflow_version, project_id, _, authority)) =
        load_linked_state(engine, workflow_instance_id)?
    else {
        return Ok(false);
    };
    let manifest: ReviewedWorkflowScopeManifest = serde_json::from_str(&authority)
        .map_err(|_| "Stored Routine authority is invalid.".to_string())?;
    Ok(manifest.mode == REVIEWED_WORKFLOW_SCOPE_MODE
        && manifest.schedule_id == schedule_id
        && manifest.workflow_id == workflow_id
        && manifest.workflow_version == workflow_version
        && manifest.project_id == project_id)
}

fn derive_from_state(
    engine: &PersistenceEngine,
    schedule_id: &str,
    workflow_id: &str,
    workflow_version: u32,
    project_id: &str,
    delivery: &Value,
) -> Result<ReviewedWorkflowScopeManifest, String> {
    validate_compiled_project_binding(engine, workflow_id, workflow_version, project_id)?;
    let compiled = engine
        .load_compiled_workflow(workflow_id, Some(workflow_version))
        .map_err(|_| "The selected compiled Workflow version is unavailable.".to_string())?;
    if compiled.workflow_ir.workflow_id != workflow_id
        || compiled.workflow_ir.workflow_version != workflow_version
    {
        return Err("The selected compiled Workflow identity does not match.".to_string());
    }
    let roots = canonical_project_roots(engine, project_id)?;
    let mut nodes = Vec::new();
    for workflow_node in &compiled.workflow_ir.nodes {
        let WorkflowNode::McpTool(node) = workflow_node else {
            continue;
        };
        let Some(policy) = reviewed_tool_policy(&node.server_name, &node.tool_name) else {
            if reviewed_effect_requires_explicit_approval(&node.server_name, &node.tool_name)
                && directly_guarded_by_exact_permission(
                    &compiled.workflow_ir,
                    &node.id,
                    effect_requires_denial_branch(&node.server_name, &node.tool_name),
                )
            {
                continue;
            }
            return Err(
                "This Workflow contains an action that cannot be included in its automatic authority review. Add a direct approval step or remove the action before scheduling."
                    .to_string(),
            );
        };
        let path_pointer = if policy.path_keys.is_empty() {
            None
        } else {
            Some(require_path_template(
                &node.arguments,
                policy.path_keys,
                policy.action,
                policy.extensions,
                &roots,
            )?)
        };
        nodes.push(ReviewedNodeAuthority {
            node_id: node.id.clone(),
            server_name: node.server_name.clone(),
            tool_name: node.tool_name.clone(),
            action: policy.action,
            arguments_template: node.arguments.clone(),
            path_pointer,
            allowed_extensions: policy
                .extensions
                .iter()
                .map(|value| value.to_string())
                .collect(),
        });
    }
    Ok(ReviewedWorkflowScopeManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        mode: REVIEWED_WORKFLOW_SCOPE_MODE.to_string(),
        schedule_id: schedule_id.to_string(),
        workflow_id: workflow_id.to_string(),
        workflow_version,
        project_id: project_id.to_string(),
        project_roots: roots,
        nodes,
        terminal_delivery: parse_terminal_delivery(delivery)?,
    })
}

fn require_mode_only(authority: &Value) -> Result<(), String> {
    let object = authority
        .as_object()
        .ok_or_else(|| "Routine authority review is required.".to_string())?;
    if object.len() != 1
        || object.get("mode").and_then(Value::as_str) != Some(REVIEWED_WORKFLOW_SCOPE_MODE)
    {
        return Err(
            "Routine authority must contain only the reviewed Workflow scope mode.".to_string(),
        );
    }
    Ok(())
}

fn validate_compiled_project_binding(
    engine: &PersistenceEngine,
    workflow_id: &str,
    version: u32,
    project_id: &str,
) -> Result<(), String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let state = connection
        .query_row(
            "SELECT b.project_id,b.compilation_status,b.workflow_ir_json IS NOT NULL,EXISTS(SELECT 1 FROM projects p WHERE p.project_id=?3 AND p.archived_at_ms IS NULL) FROM workflow_blueprints b WHERE b.workflow_id=?1 AND b.version=?2",
            params![workflow_id, version, project_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((bound_project_id, compilation_status, has_ir, project_available)) = state else {
        return Err("routine_workflow_version_unavailable".to_string());
    };
    let Some(bound_project_id) = bound_project_id else {
        return Err("routine_workflow_project_binding_required".to_string());
    };
    if bound_project_id != project_id {
        return Err("routine_workflow_project_mismatch".to_string());
    }
    if compilation_status != "Compiled" || !has_ir || !project_available {
        return Err("routine_workflow_version_unavailable".to_string());
    }
    Ok(())
}

fn canonical_project_roots(
    engine: &PersistenceEngine,
    project_id: &str,
) -> Result<Vec<String>, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare("SELECT canonical_path FROM project_sources WHERE project_id=?1 AND grant_state='active' AND source_kind IN ('local_folder','knowledge_directory') ORDER BY canonical_path")
        .map_err(|error| error.to_string())?;
    let paths = statement
        .query_map(params![project_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let mut roots = BTreeSet::new();
    for path in paths {
        let metadata = fs::symlink_metadata(&path).map_err(|_| {
            "A Project folder is unavailable. Review the Project before scheduling.".to_string()
        })?;
        let canonical = fs::canonicalize(&path).map_err(|_| {
            "A Project folder is unavailable. Review the Project before scheduling.".to_string()
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() || canonical != Path::new(&path)
        {
            return Err(
                "A Project folder identity changed. Choose the folder again before scheduling."
                    .to_string(),
            );
        }
        roots.insert(canonical.to_string_lossy().to_string());
    }
    Ok(roots.into_iter().collect())
}

fn require_path_template(
    arguments: &Value,
    keys: &[&str],
    action: ReviewedAction,
    extensions: &[&str],
    roots: &[String],
) -> Result<String, String> {
    if roots.is_empty() {
        return Err(
            "This Workflow uses files, but the Project has no approved folder.".to_string(),
        );
    }
    if !arguments.is_object() {
        return Err("A reviewed file node has invalid arguments.".to_string());
    }
    let matches = keys
        .iter()
        .filter_map(|pointer| {
            arguments
                .pointer(pointer)
                .and_then(Value::as_str)
                .map(|value| (*pointer, value))
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 || matches[0].1.trim().is_empty() {
        return Err("A reviewed file node must name exactly one file location.".to_string());
    }
    let (pointer, template) = matches[0];
    if matches!(action, ReviewedAction::VerifiedDocumentWrite)
        && !Path::new(template).is_absolute()
        && roots.len() != 1
    {
        return Err(
            "A relative Workflow output needs exactly one approved Project folder.".to_string(),
        );
    }
    if pointer == "/file/destinationPath"
        && !matches!(
            arguments.pointer("/file/format").and_then(Value::as_str),
            Some("md" | "pdf")
        )
    {
        return Err(
            "Only reviewed Markdown and PDF creation can run without another approval.".to_string(),
        );
    }
    if !template.contains("{{") {
        let node = ReviewedNodeAuthority {
            node_id: String::new(),
            server_name: String::new(),
            tool_name: String::new(),
            action,
            arguments_template: Value::Null,
            path_pointer: Some(pointer.to_string()),
            allowed_extensions: extensions.iter().map(|value| value.to_string()).collect(),
        };
        if !verify_path_authority(&node, arguments, roots)? {
            return Err("A reviewed file node leaves the Project's approved folders.".to_string());
        }
    }
    Ok(pointer.to_string())
}

fn load_linked_state(
    engine: &PersistenceEngine,
    instance_id: &str,
) -> Result<Option<(String, String, u32, String, String, String)>, String> {
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT s.id,s.workflow_id,s.workflow_version,s.project_id,s.delivery_target_json,s.authority_json FROM routine_runs r JOIN workflow_schedules s ON s.id=r.schedule_id JOIN execution_instances e ON e.id=r.execution_instance_id WHERE r.execution_instance_id=?1 AND e.workflow_id=s.workflow_id AND e.workflow_version=s.workflow_version AND e.project_id=s.project_id",
            params![instance_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn public_https_argument(arguments: &Value) -> bool {
    let Some(raw) = arguments.get("url").and_then(Value::as_str) else {
        return false;
    };
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host_str().map(|host| host.to_ascii_lowercase()) else {
        return false;
    };
    if host == "localhost" || host.ends_with(".local") || host.ends_with(".internal") {
        return false;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            !(ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified())
        }
        Ok(IpAddr::V6(ip)) => {
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local())
        }
        Err(_) => true,
    }
}

fn parse_terminal_delivery(delivery: &Value) -> Result<Option<TerminalDeliveryAuthority>, String> {
    let Some(object) = delivery.as_object() else {
        return Err("Routine delivery must be an object.".to_string());
    };
    if object.is_empty() {
        return Ok(None);
    }
    if object.len() != 2 {
        return Err("Routine delivery contains unsupported fields.".to_string());
    }
    let platform = object
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let destination = object
        .get("destination")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if !matches!(platform, "telegram" | "discord" | "slack")
        || destination.is_empty()
        || destination.len() > 512
    {
        return Err("Routine delivery destination is invalid.".to_string());
    }
    Ok(Some(TerminalDeliveryAuthority {
        platform: platform.to_string(),
        destination_sha256: crate::foundation::digest::sha256_hex(destination.as_bytes()),
    }))
}

fn terminal_delivery_arguments_match(
    expected: Option<&TerminalDeliveryAuthority>,
    arguments: &Value,
) -> bool {
    let Some(expected) = expected else {
        return false;
    };
    let Some(object) = arguments.as_object() else {
        return false;
    };
    if object.len() != 2 {
        return false;
    }
    let platform = object
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let destination = object
        .get("destination")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    platform == expected.platform
        && crate::foundation::digest::sha256_hex(destination.as_bytes())
            == expected.destination_sha256
}
