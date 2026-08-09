use super::*;
use rusqlite::OptionalExtension;

#[derive(Clone, Debug)]
pub(super) struct ScheduledExecutionContext {
    pub(super) schedule_id: String,
    pub(super) project_id: String,
    pub(super) project_root: PathBuf,
    pub(super) scheduled_for_ms: Option<i64>,
}

pub(super) fn resolve_scheduled_project_context(
    schedule: &WorkflowScheduleRecord,
    persistence: &PersistenceEngine,
    project_folder_required: bool,
    workspace_root: &Path,
) -> Result<ScheduledExecutionContext, WorkflowRuntimeError> {
    let project_id = schedule
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            WorkflowRuntimeError::input(
                "This Routine is not attached to a Project. Choose a Project before running it."
                    .to_string(),
            )
        })?;
    let connection = persistence
        .open_connection()
        .map_err(WorkflowRuntimeError::database)?;
    let archived_at_ms = connection
        .query_row(
            "SELECT archived_at_ms FROM projects WHERE project_id=?1",
            rusqlite::params![project_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(WorkflowRuntimeError::database)?
        .ok_or_else(|| {
            WorkflowRuntimeError::input(
                "The Project attached to this Routine no longer exists.".to_string(),
            )
        })?;
    if archived_at_ms.is_some() {
        return Err(WorkflowRuntimeError::input(
            "The Project attached to this Routine is archived. Restore it before running the Routine."
                .to_string(),
        ));
    }
    let project_root = if project_folder_required {
        crate::projects::path_scope::single_active_project_root(persistence, project_id)
            .map_err(WorkflowRuntimeError::input)?
    } else {
        let scope = workspace_root.join(format!(
            "routine-scope-{}",
            crate::foundation::digest::sha256_hex(schedule.id.as_bytes())
        ));
        std::fs::create_dir_all(&scope).map_err(WorkflowRuntimeError::io)?;
        std::fs::canonicalize(&scope).map_err(WorkflowRuntimeError::io)?
    };
    Ok(ScheduledExecutionContext {
        schedule_id: schedule.id.clone(),
        project_id: project_id.to_string(),
        project_root,
        scheduled_for_ms: schedule.next_run_at_ms,
    })
}

pub(super) fn scheduled_run_request(
    schedule: &WorkflowScheduleRecord,
    compiled: &CompiledWorkflow,
    context: &ScheduledExecutionContext,
) -> Result<RunWorkflowRequest, WorkflowRuntimeError> {
    let mut request_value = if schedule.run_request.is_null() {
        json!({})
    } else {
        crate::routines::control::without_controls(&schedule.run_request)
            .map_err(WorkflowRuntimeError::input)?
    };
    let object = request_value.as_object_mut().ok_or_else(|| {
        WorkflowRuntimeError::input(
            "workflow_schedules.run_request_json must be a JSON object.".to_string(),
        )
    })?;
    object.insert(
        "workflowId".to_string(),
        Value::String(schedule.workflow_id.clone()),
    );
    object.insert(
        "workflowVersion".to_string(),
        json!(compiled.workflow_ir.workflow_version),
    );
    object
        .entry("inputs".to_string())
        .or_insert_with(|| json!({}));
    object
        .entry("outputs".to_string())
        .or_insert_with(|| json!({}));

    let mut request = serde_json::from_value::<RunWorkflowRequest>(request_value)
        .map_err(WorkflowRuntimeError::serialization)?;
    for node in &compiled.workflow_ir.nodes {
        let WorkflowNode::Input(input) = node else {
            continue;
        };
        request
            .inputs
            .entry(input.id.clone())
            .or_insert_with(|| InputBinding::Manual {
                value: json!({
                    "trigger": "scheduled_workflow",
                    "scheduleId": schedule.id,
                    "workflowId": schedule.workflow_id,
                    "workflowVersion": compiled.workflow_ir.workflow_version,
                    "inputNodeId": input.id,
                    "scheduleExpression": schedule.schedule_expression,
                    "scheduledAtMs": context.scheduled_for_ms.unwrap_or_else(unix_time_ms),
                    "projectId": context.project_id,
                    "projectRoot": context.project_root.to_string_lossy(),
                }),
            });
    }
    Ok(request)
}

pub(crate) fn retry_scheduled_workflow(
    schedule: &WorkflowScheduleRecord,
    instance_id: &str,
    persistence: &PersistenceEngine,
    gemma: GemmaService,
    mcp_registry: McpClientRegistry,
    app: tauri::AppHandle,
    workspace_root: &Path,
) -> Result<RunWorkflowResponse, WorkflowRuntimeError> {
    require_durable_workflow_actuation(persistence, "scheduled workflow recovery")?;
    let mut instance = persistence
        .load_execution_instance(instance_id)
        .map_err(WorkflowRuntimeError::database)?;
    if instance.status != ExecutionStatus::Pending {
        return Err(WorkflowRuntimeError::input(
            "The scheduled Workflow is not waiting for a transient retry.".to_string(),
        ));
    }
    let compiled = persistence
        .load_compiled_workflow(&instance.workflow_id, Some(instance.workflow_version))
        .map_err(WorkflowRuntimeError::database)?;
    let capabilities =
        crate::workflow_ir::review::workflow_review_capabilities(&compiled.workflow_ir);
    let context = resolve_scheduled_project_context(
        schedule,
        persistence,
        capabilities.project_file_read || capabilities.project_file_write,
        workspace_root,
    )?;
    let connection = persistence
        .open_connection()
        .map_err(WorkflowRuntimeError::database)?;
    let binding = connection
        .query_row(
            "SELECT e.project_id,r.schedule_id FROM execution_instances e JOIN routine_runs r ON r.execution_instance_id=e.id WHERE e.id=?1",
            rusqlite::params![instance_id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(WorkflowRuntimeError::database)?;
    if binding.0.as_deref() != Some(context.project_id.as_str()) || binding.1 != schedule.id {
        return Err(WorkflowRuntimeError::input(
            "The scheduled Workflow no longer matches its Project and Routine binding.".to_string(),
        ));
    }
    let request: RunWorkflowRequest = serde_json::from_value(instance.input_payload.clone())
        .map_err(WorkflowRuntimeError::serialization)?;
    let model = resolved_gemma_runtime_model(&app, gemma)?;
    let external_tools = McpRuntimeTools {
        registry: mcp_registry,
        persistence: persistence.clone(),
        knowledge_tools: Some(KnowledgeRuntimeTools),
        app: Some(app),
    };
    instance.status = ExecutionStatus::Pending;
    instance.active_node_id = None;
    instance.pause_context = None;
    instance.error = None;
    instance.completed_at_ms = None;
    instance.updated_at_ms = unix_time_ms();
    persistence
        .update_execution_instance(&instance)
        .map_err(WorkflowRuntimeError::database)?;
    let mut checkpoint = |current: &ExecutionInstance| {
        persistence
            .update_execution_instance(current)
            .map_err(WorkflowRuntimeError::database)
    };
    let mut progress = |_current: &ExecutionInstance,
                        _node_id: &str,
                        _step_index: usize,
                        _status: &str,
                        _message: &str| {};
    let mut result = execute_workflow(
        &compiled,
        &request,
        &model,
        &external_tools,
        workspace_root,
        &mut instance,
        &mut checkpoint,
        &mut progress,
        Some(persistence),
        None,
    );
    if let Err(error) = &mut result {
        if instance.status != ExecutionStatus::AwaitingApproval {
            instance.status = ExecutionStatus::Failed;
            instance.error = Some(json!({ "code": error.code, "message": error.message }));
            instance.active_node_id = None;
            finish_timing(&mut instance, true);
        }
    }
    persistence
        .update_execution_instance(&instance)
        .map_err(WorkflowRuntimeError::database)?;
    match result {
        Ok(outcome) => Ok(run_workflow_response(
            instance,
            outcome.execution_order,
            outcome.approval_request,
        )),
        Err(_) => {
            let execution_order = recorded_execution_order(&compiled.workflow_ir, &instance);
            Ok(run_workflow_response(instance, execution_order, None))
        }
    }
}
