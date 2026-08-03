use super::*;

pub(super) fn project_workflow_task(
    engine: &PersistenceEngine,
    instance_id: &str,
    reconcile_state: bool,
) -> Result<String, String> {
    let task_run_id = register_workflow_task(engine, instance_id)?;
    engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE routine_runs SET task_run_id=?2 WHERE execution_instance_id=?1",
            rusqlite::params![instance_id, task_run_id],
        )
        .map_err(|error| error.to_string())?;
    if reconcile_state {
        engine.reconcile_remote_workflow_task(instance_id)?;
    }
    Ok(task_run_id)
}

pub(super) fn scheduled_approval(
    response: &workflow_runtime::RunWorkflowResponse,
) -> Option<&workflow_runtime::ApprovalRequest> {
    (response.instance.status == ExecutionStatus::AwaitingApproval)
        .then_some(response.approval_request.as_ref())
        .flatten()
}
