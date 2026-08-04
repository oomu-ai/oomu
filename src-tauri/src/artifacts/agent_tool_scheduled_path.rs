use super::CreateFileEnvelope;
use crate::{
    projects::path_scope::{resolve_project_output_path, single_active_project_root},
    shield_gate::ExecuteCommandResponse,
    tools::task_tool_runtime::TASK_RUN_TIMESTAMP_TOKEN,
};
use chrono::{Local, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::params;
use serde_json::Value;
use std::{fs, path::Path};

pub(super) fn resolve_registration(
    persistence: &crate::db::PersistenceEngine,
    execution_id: Option<&str>,
    arguments: Value,
    _outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    let execution_id = execution_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Creating a file requires an active Task.".to_string())?;
    let task = crate::tools::task_runtime::require_agent_runtime_task(persistence, execution_id)?;
    resolve_registration_for_task(persistence, &task, arguments)
}

pub(super) fn resolve_registration_for_task(
    persistence: &crate::db::PersistenceEngine,
    task: &crate::tools::task_runtime::AgentRuntimeTaskBinding,
    arguments: Value,
) -> Result<Value, String> {
    let mut request = serde_json::from_value::<CreateFileEnvelope>(arguments)
        .map_err(|_| "create_file arguments do not match the registered schema.".to_string())?;
    let (created_at_ms, scheduled_for_ms, routine_timezone, runtime_kind, origin): (
        i64,
        Option<i64>,
        Option<String>,
        String,
        String,
    ) = persistence
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT t.created_at_ms,r.scheduled_for_ms,s.routine_timezone,t.runtime_kind,t.origin FROM task_runs t LEFT JOIN routine_runs r ON r.task_run_id=t.task_run_id LEFT JOIN workflow_schedules s ON s.id=r.schedule_id WHERE t.task_run_id=?1 AND t.project_id=?2",
            params![task.task_run_id, task.project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .map_err(|_| "The active Task timestamp could not be verified.".to_string())?;
    let scheduled_routine = runtime_kind == "workflow" && origin == "routine";
    let timestamp = if scheduled_routine {
        let occurrence_ms = scheduled_for_ms.ok_or_else(|| {
            "The scheduled Task occurrence could not be verified durably.".to_string()
        })?;
        let timezone = routine_timezone
            .as_deref()
            .ok_or_else(|| "The scheduled Task timezone is missing.".to_string())?
            .parse::<Tz>()
            .map_err(|_| "The scheduled Task timezone is invalid.".to_string())?;
        Utc.timestamp_millis_opt(occurrence_ms)
            .single()
            .ok_or_else(|| "The scheduled Task occurrence is invalid.".to_string())?
            .with_timezone(&timezone)
            .format("%Y-%m-%d_%H-%M")
            .to_string()
    } else {
        Local
            .timestamp_millis_opt(created_at_ms)
            .single()
            .ok_or_else(|| "The active Task timestamp is invalid.".to_string())?
            .format("%Y-%m-%d_%H-%M")
            .to_string()
    };
    let requested = request
        .file
        .destination_path
        .replace(TASK_RUN_TIMESTAMP_TOKEN, &timestamp);
    let requested = if scheduled_routine {
        requested
    } else {
        expand_direct_home_destination(&requested)?
    };
    request.file.destination_path = resolve_task_output_destination(
        persistence,
        &task.project_id,
        requested,
        scheduled_routine,
    )?;
    serde_json::to_value(request).map_err(|error| error.to_string())
}

fn resolve_task_output_destination(
    persistence: &crate::db::PersistenceEngine,
    project_id: &str,
    requested: String,
    scheduled_routine: bool,
) -> Result<String, String> {
    if !scheduled_routine && Path::new(&requested).is_absolute() {
        return Ok(requested);
    }
    let root = single_active_project_root(persistence, project_id)?;
    Ok(resolve_project_output_path(&root, &requested)?
        .to_string_lossy()
        .to_string())
}

fn expand_direct_home_destination(requested: &str) -> Result<String, String> {
    let Some(relative) = requested.strip_prefix("~/") else {
        return Ok(requested.to_string());
    };
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "Unable to resolve the user home directory.".to_string())?;
    Ok(home.join(relative).to_string_lossy().to_string())
}

pub(super) fn scheduled_workflow_task(
    persistence: &crate::db::PersistenceEngine,
    task_run_id: &str,
) -> Result<bool, String> {
    persistence
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT runtime_kind='workflow' AND origin='routine' FROM task_runs WHERE task_run_id=?1",
            params![task_run_id],
            |row| row.get(0),
        )
        .map_err(|_| "The active Task kind could not be verified.".to_string())
}

#[derive(Debug, Default)]
pub(super) struct OutputParentGuard {
    created: Option<std::path::PathBuf>,
}

impl OutputParentGuard {
    pub(super) fn commit(&mut self) {
        self.created = None;
    }
}

impl Drop for OutputParentGuard {
    fn drop(&mut self) {
        if let Some(path) = self.created.take() {
            let _ = fs::remove_dir(path);
        }
    }
}

pub(super) fn ensure_output_parent(
    persistence: &crate::db::PersistenceEngine,
    project_id: &str,
    destination_path: &str,
) -> Result<OutputParentGuard, String> {
    let root = single_active_project_root(persistence, project_id)?;
    let resolved = resolve_project_output_path(&root, destination_path)?;
    if resolved != Path::new(destination_path) {
        return Err("The scheduled file destination changed before creation.".to_string());
    }
    let parent = resolved
        .parent()
        .ok_or_else(|| "The scheduled file destination has no parent folder.".to_string())?;
    match fs::symlink_metadata(parent) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            Ok(OutputParentGuard::default())
        }
        Ok(_) => Err("The scheduled file parent must be a real folder.".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let binding = crate::shield_gate::bind_approved_external_directory_creation(
                &parent.to_string_lossy(),
            )
            .map_err(|error| error.message)?;
            let created = crate::shield_gate::create_bound_approved_external_directory(&binding)
                .map_err(|error| error.message)?;
            if created != parent {
                return Err(
                    "The scheduled file parent changed while OOMU was creating it.".to_string(),
                );
            }
            Ok(OutputParentGuard {
                created: Some(created),
            })
        }
        Err(_) => Err("The scheduled file parent could not be inspected safely.".to_string()),
    }
}
