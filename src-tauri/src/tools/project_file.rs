use crate::{
    foundation::digest::sha256_hex,
    p0_contracts::EvidenceClass,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::{
        task_runtime::{record_event, require_agent_runtime_task},
        task_tool_runtime::{
            TaskToolApprovalTier, TaskToolExecutionContext, TaskToolFuture, TaskToolMetadata,
            TaskToolRegistration, TaskToolRiskTier, TaskToolValidation,
        },
    },
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const OPERATION: &str = "read_project_file";
const DEFAULT_MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadProjectFileRequest {
    path: String,
    #[serde(default = "default_max_bytes")]
    max_bytes: usize,
}

fn default_max_bytes() -> usize {
    DEFAULT_MAX_BYTES
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectFileReceipt {
    pub(crate) canonical_path: String,
    pub(crate) content: String,
    pub(crate) byte_count: usize,
    pub(crate) content_sha256: String,
    pub(crate) verified: bool,
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: OPERATION,
        validate: validate_registration,
        validate_resolved: validate_registration,
        resolve: crate::tools::task_tool_runtime::identity_resolver,
        execute: execute_registration,
        planner_context: None,
        schema: input_schema,
        metadata: TaskToolMetadata {
            description: "Read one exact UTF-8 file from the Task's approved Project folders and verify its canonical identity, byte count, and SHA-256.",
            risk_tier: TaskToolRiskTier::FileRead,
            approval_tier: TaskToolApprovalTier::Background,
            agent_error_code: "project_file_read_failed",
            agent_error_boundary: "ProjectFileRead",
            execution_path: "The native read_project_file tool resolved the exact path inside the bound Project's active canonical roots, opened it without staging, bounded the bytes, rechecked file identity, and hashed the returned text.",
        },
    })
}

fn input_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "path":{
                "type":"string",
                "minLength":1,
                "maxLength":8192,
                "description":"Exact absolute Project file path, or a path relative to one unambiguous approved Project root."
            },
            "maxBytes":{
                "type":"integer",
                "minimum":1,
                "maximum":MAX_BYTES,
                "default":DEFAULT_MAX_BYTES
            }
        },
        "required":["path"],
        "additionalProperties":false
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request =
        serde_json::from_value::<ReadProjectFileRequest>(arguments).map_err(|_| {
            "read_project_file arguments do not match the registered schema.".to_string()
        })?;
    request.path = request.path.trim().to_string();
    if request.path.is_empty()
        || request.path.len() > 8_192
        || request.path.starts_with('~')
        || !(1..=MAX_BYTES).contains(&request.max_bytes)
        || Path::new(&request.path)
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(
            "read_project_file request is outside the bounded Project contract.".to_string(),
        );
    }
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: false,
    })
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request =
            serde_json::from_value::<ReadProjectFileRequest>(arguments).map_err(|_| {
                "read_project_file arguments do not match the registered schema.".to_string()
            })?;
        let execution_id = context
            .execution_id
            .ok_or_else(|| "Reading a Project file requires an active Task.".to_string())?;
        let task = require_agent_runtime_task(context.persistence, execution_id)?;
        let receipt = read_project_file(
            context.persistence,
            &task.project_id,
            &request.path,
            request.max_bytes,
        )?;
        record_event(
            context.persistence,
            &task.task_run_id,
            "project_file.read",
            EvidenceClass::VerifiedPostcondition,
            json!({
                "canonicalPath":receipt.canonical_path,
                "byteCount":receipt.byte_count,
                "contentSha256":receipt.content_sha256,
            }),
        )?;
        Ok(ExecuteCommandResponse {
            operation: OPERATION.to_string(),
            status: CommandStatus::Completed,
            message: serde_json::to_string(&receipt).map_err(|error| error.to_string())?,
            metrics: None,
            claims: vec![format!(
                "CLAIM project_file_read=true canonical_path={} byte_count={} content_sha256={}",
                receipt.canonical_path, receipt.byte_count, receipt.content_sha256
            )],
            verified: true,
            model_used: None,
        })
    })
}

pub(crate) fn read_project_file(
    persistence: &crate::db::PersistenceEngine,
    project_id: &str,
    raw_path: &str,
    max_bytes: usize,
) -> Result<ProjectFileReceipt, String> {
    let roots = active_project_roots(persistence, project_id)?;
    let target = resolve_unique_target(&roots, raw_path)?;
    let mut file = fs::File::open(&target)
        .map_err(|_| "The approved Project file could not be opened.".to_string())?;
    let opened = file
        .metadata()
        .map_err(|_| "The approved Project file identity could not be verified.".to_string())?;
    let before = fs::symlink_metadata(&target)
        .map_err(|_| "The approved Project file identity could not be verified.".to_string())?;
    if !opened.is_file()
        || before.file_type().is_symlink()
        || !same_file_identity(&opened, &before)
        || opened.len() > max_bytes as u64
    {
        return Err("The approved Project file is not a bounded regular file.".to_string());
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.by_ref()
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "The approved Project file could not be read.".to_string())?;
    let after = fs::symlink_metadata(&target)
        .map_err(|_| "The approved Project file identity changed while reading.".to_string())?;
    if bytes.len() > max_bytes
        || after.file_type().is_symlink()
        || !same_file_identity(&opened, &after)
        || opened.len() != after.len()
    {
        return Err("The approved Project file changed while it was being read.".to_string());
    }
    let content = String::from_utf8(bytes)
        .map_err(|_| "The approved Project file is not UTF-8 text.".to_string())?;
    Ok(ProjectFileReceipt {
        canonical_path: target.to_string_lossy().to_string(),
        byte_count: content.len(),
        content_sha256: sha256_hex(content.as_bytes()),
        content,
        verified: true,
    })
}

pub(crate) fn require_bound_path_in_active_project(
    persistence: &crate::db::PersistenceEngine,
    project_id: &str,
    canonical_path: &str,
) -> Result<(), String> {
    let target = Path::new(canonical_path);
    if !target.is_absolute()
        || target
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err("The bound file path is not an exact absolute Project path.".to_string());
    }
    active_project_roots(persistence, project_id)?
        .iter()
        .any(|root| target != root && target.starts_with(root))
        .then_some(())
        .ok_or_else(|| {
            "The bound file path is outside this Task's active Project folders.".to_string()
        })
}

fn active_project_roots(
    persistence: &crate::db::PersistenceEngine,
    project_id: &str,
) -> Result<Vec<PathBuf>, String> {
    let connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare("SELECT canonical_path FROM project_sources WHERE project_id=?1 AND grant_state='active' AND source_kind IN ('local_folder','knowledge_directory') ORDER BY canonical_path")
        .map_err(|error| error.to_string())?;
    let raw_roots = statement
        .query_map(params![project_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let mut roots = BTreeSet::new();
    for stored in raw_roots {
        let stored = PathBuf::from(stored);
        let metadata = fs::symlink_metadata(&stored)
            .map_err(|_| "An approved Project folder is unavailable.".to_string())?;
        let root = fs::canonicalize(&stored)
            .map_err(|_| "An approved Project folder is unavailable.".to_string())?;
        if root != stored || metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("An approved Project folder identity changed.".to_string());
        }
        roots.insert(root);
    }
    if roots.is_empty() {
        return Err("This Task has no approved Project folder for file reading.".to_string());
    }
    Ok(roots.into_iter().collect())
}

fn resolve_unique_target(roots: &[PathBuf], raw_path: &str) -> Result<PathBuf, String> {
    let requested = Path::new(raw_path);
    let mut matches = BTreeSet::new();
    for root in roots {
        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            root.join(requested)
        };
        let Ok(canonical) = fs::canonicalize(candidate) else {
            continue;
        };
        if canonical.starts_with(root) && canonical.is_file() {
            matches.insert(canonical);
        }
    }
    match matches.into_iter().collect::<Vec<_>>().as_slice() {
        [target] => Ok(target.clone()),
        [] => Err("The file is not inside an active approved Project folder.".to_string()),
        _ => {
            Err("The relative file path matches more than one approved Project folder.".to_string())
        }
    }
}

#[cfg(unix)]
fn same_file_identity(opened: &fs::Metadata, current: &fs::Metadata) -> bool {
    opened.dev() == current.dev() && opened.ino() == current.ino()
}

#[cfg(test)]
#[path = "project_file_tests.rs"]
mod tests;

#[cfg(not(unix))]
fn same_file_identity(opened: &fs::Metadata, current: &fs::Metadata) -> bool {
    opened.len() == current.len() && opened.modified().ok() == current.modified().ok()
}
