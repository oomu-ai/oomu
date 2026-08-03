use super::*;

impl PersistenceEngine {
    pub fn insert_scheduled_execution_instance(
        &self,
        instance: &ExecutionInstance,
        project_id: &str,
        schedule_id: &str,
        scheduled_for_ms: Option<i64>,
    ) -> rusqlite::Result<()> {
        let project_id = project_id.trim();
        let schedule_id = schedule_id.trim();
        if project_id.is_empty() || schedule_id.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "Scheduled workflow execution requires a Project and Routine identity.".to_string(),
            ));
        }
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        write_execution_instance(&transaction, instance, false)?;
        let bound = transaction.execute(
            "UPDATE execution_instances SET project_id=?2 WHERE id=?1 AND project_id IS NULL",
            params![instance.id, project_id],
        )?;
        if bound != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        let task_id = crate::p0_contracts::TaskId::new().to_string();
        let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
        transaction.execute(
            "INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'workflow',?4,'running','routine',?2,?5,?6,?6,'reconciled')",
            params![
                task_run_id,
                task_id,
                project_id,
                instance.id,
                format!("Workflow run {}", instance.workflow_id),
                unix_time_ms()
            ],
        )?;
        transaction.execute(
            "INSERT INTO routine_runs (schedule_id,execution_instance_id,task_run_id,scheduled_for_ms,created_at_ms) VALUES (?1,?2,?3,?4,?5)",
            params![schedule_id, instance.id, task_run_id, scheduled_for_ms, unix_time_ms()],
        )?;
        transaction.commit()
    }

    pub fn load_workflow_schedule(&self, id: &str) -> rusqlite::Result<WorkflowScheduleRecord> {
        let connection = self.open_connection()?;
        select_workflow_schedule_by_id(&connection, id.trim())
    }
}
