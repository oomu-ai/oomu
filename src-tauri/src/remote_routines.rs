use crate::{db::PersistenceEngine, foundation::clock::unix_time_ms_i64 as unix_time_ms};
use rusqlite::OptionalExtension;
use serde_json::Value;

#[derive(Clone, Debug)]
pub(crate) struct RemoteRoutineApproval {
    pub code_hash: String,
    pub instance_id: String,
    pub approval_token: String,
    pub approve: bool,
}

impl PersistenceEngine {
    pub(crate) fn resolve_remote_routine_approval(
        &self,
        platform: &str,
        sender: &str,
        body: &str,
    ) -> Result<Option<RemoteRoutineApproval>, String> {
        let parts = body.split_whitespace().collect::<Vec<_>>();
        let ["/approve", code, decision] = parts.as_slice() else {
            return Ok(None);
        };
        let approve = match *decision {
            "approve" => true,
            "deny" | "reject" => false,
            _ => return Err("Use approve or deny for the exact approval decision.".to_string()),
        };
        let code_hash = crate::foundation::digest::sha256_hex(code.as_bytes());
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        let instance_id: Option<String> = connection
            .query_row(
                "SELECT execution_instance_id FROM routine_remote_approvals WHERE decision_code_hash=?1 AND channel_platform=?2 AND channel_owner_hash=?3 AND decided_at_ms IS NULL AND expires_at_ms>=?4",
                rusqlite::params![code_hash, platform, crate::foundation::digest::sha256_hex(sender.trim().as_bytes()), unix_time_ms()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some(instance_id) = instance_id else {
            return Err("That approval is missing, expired, already used, or belongs to another channel owner.".to_string());
        };
        let raw = crate::secret_store::get_routine_approval(&code_hash)?
            .ok_or_else(|| "The Keychain approval binding is unavailable.".to_string())?;
        let secret: Value = serde_json::from_str(&raw)
            .map_err(|_| "The Keychain approval binding is invalid.".to_string())?;
        if secret.get("instanceId").and_then(Value::as_str) != Some(instance_id.as_str()) {
            return Err("The approval binding does not match the exact task.".to_string());
        }
        let approval_token = secret
            .get("approvalToken")
            .and_then(Value::as_str)
            .ok_or_else(|| "The approval token is unavailable.".to_string())?
            .to_string();
        Ok(Some(RemoteRoutineApproval {
            code_hash,
            instance_id,
            approval_token,
            approve,
        }))
    }

    pub(crate) fn complete_remote_routine_approval(
        &self,
        resolution: &RemoteRoutineApproval,
    ) -> Result<(), String> {
        self.open_connection().map_err(|error| error.to_string())?.execute(
            "UPDATE routine_remote_approvals SET decided_at_ms=?2,decision=?3 WHERE decision_code_hash=?1 AND decided_at_ms IS NULL",
            rusqlite::params![resolution.code_hash, unix_time_ms(), if resolution.approve { "approve" } else { "reject" }],
        ).map_err(|error| error.to_string())?;
        crate::secret_store::delete_routine_approval(&resolution.code_hash)
    }

    pub(crate) fn reconcile_remote_workflow_task(&self, instance_id: &str) -> Result<(), String> {
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        let (status, error_json): (String, Option<String>) = connection
            .query_row(
                "SELECT status,error_json FROM execution_instances WHERE id=?1",
                rusqlite::params![instance_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| error.to_string())?;
        let state = match status.to_ascii_lowercase().as_str() {
            "pending" => "queued",
            "running" => "running",
            "awaitingapproval" | "awaiting_approval" => "awaiting_approval",
            "paused" => "blocked",
            "completed" => "completed",
            "cancelled" => "cancelled",
            _ => "failed",
        };
        let last_error = workflow_task_error_message(error_json.as_deref());
        connection.execute(
            "UPDATE task_runs SET state=?2,last_error=?3,updated_at_ms=?4,completed_at_ms=CASE WHEN ?2 IN ('completed','failed','cancelled') THEN ?4 ELSE completed_at_ms END,recovery_state='reconciled' WHERE runtime_kind='workflow' AND runtime_record_id=?1",
            rusqlite::params![instance_id, state, last_error, unix_time_ms()],
        ).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn cancel_remote_task(&self, task_run_id: &str) -> Result<(), String> {
        if !task_run_id.starts_with("taskrun_") {
            return Err("Invalid task run identifier.".to_string());
        }
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        let runtime: Option<(String, String)> = connection
            .query_row(
                "SELECT runtime_kind,runtime_record_id FROM task_runs WHERE task_run_id=?1",
                rusqlite::params![task_run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((runtime, id)) = runtime else {
            return Err("Task run not found.".to_string());
        };
        let now = unix_time_ms();
        let changed = match runtime.as_str() {
            "taskflow" => {
                connection.execute("UPDATE taskflow_steps SET status='cancelled' WHERE flow_id=?1 AND status IN ('queued','active')", rusqlite::params![id]).map_err(|error| error.to_string())?;
                connection.execute("UPDATE taskflows SET status='cancelled',updated_at_ms=?2 WHERE flow_id=?1 AND status NOT IN ('verified','cancelled')", rusqlite::params![id, now])
            }
            "agent" => connection.execute("UPDATE agent_executions SET status='cancelled',updated_at_ms=?2 WHERE execution_id=?1 AND status IN ('running','halted')", rusqlite::params![id, now]),
            "queued_message" => connection.execute("UPDATE message_queue SET status='cancelled',updated_at_ms=?2,error_message='Cancelled by remote owner.' WHERE CAST(id AS TEXT)=?1 AND status='queued'", rusqlite::params![id, now]),
            _ => return Err("This runtime does not support cancellation at its current safe boundary.".to_string()),
        }.map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err("The owning runtime rejected cancellation.".to_string());
        }
        connection.execute("UPDATE task_runs SET state='cancelled',updated_at_ms=?2,completed_at_ms=?2,recovery_state='reconciled' WHERE task_run_id=?1", rusqlite::params![task_run_id, now]).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(crate) fn handle_remote_routine_control(
        &self,
        body: &str,
    ) -> Result<Option<String>, String> {
        let parts = body.split_whitespace().collect::<Vec<_>>();
        if parts.is_empty() || !parts[0].starts_with('/') {
            return Ok(None);
        }
        let now = unix_time_ms();
        match parts.as_slice() {
            ["/routine", id, "status"] if id.starts_with("routine_task_") => {
                let connection = self.open_connection().map_err(|error| error.to_string())?;
                let routine: Option<(String, bool, Option<i64>)> = connection.query_row("SELECT label,is_active,next_run_at_ms FROM workflow_schedules WHERE id=?1 AND id LIKE 'routine_%'", rusqlite::params![id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional().map_err(|error| error.to_string())?;
                let Some((label, active, next)) = routine else { return Err("Routine not found.".to_string()); };
                Ok(Some(format!("{} is {}. Next run: {}.", label, if active { "active" } else { "paused" }, next.map(|value| value.to_string()).unwrap_or_else(|| "not scheduled".to_string()))))
            }
            ["/routine", id, "run"] if id.starts_with("routine_task_") => {
                let changed = self.open_connection().map_err(|error| error.to_string())?.execute("UPDATE workflow_schedules SET is_active=1,next_run_at_ms=?2,claimed_at_ms=NULL,paused_reason=NULL,updated_at_ms=?2 WHERE id=?1 AND id LIKE 'routine_%'", rusqlite::params![id, now]).map_err(|error| error.to_string())?;
                if changed == 0 { return Err("Routine not found.".to_string()); }
                Ok(Some("Routine queued to run now.".to_string()))
            }
            ["/routine", id, action @ ("pause" | "resume")] if id.starts_with("routine_task_") => {
                let active = *action == "resume";
                let changed = self.open_connection().map_err(|error| error.to_string())?.execute("UPDATE workflow_schedules SET is_active=?2,claimed_at_ms=NULL,paused_reason=?3,updated_at_ms=?4 WHERE id=?1 AND id LIKE 'routine_%'", rusqlite::params![id, active, if active { None::<String> } else { Some("Paused by remote owner".to_string()) }, now]).map_err(|error| error.to_string())?;
                if changed == 0 { return Err("Routine not found.".to_string()); }
                Ok(Some(if active { "Routine resumed." } else { "Routine paused." }.to_string()))
            }
            ["/task", id, "cancel"] => {
                self.cancel_remote_task(id)?;
                Ok(Some("Task cancellation was accepted by its owning runtime.".to_string()))
            }
            ["/approve", ..] => Err("Remote approval is blocked because no exact, unexpired approval binding was found. Open OOMU Tasks to review the action.".to_string()),
            _ => Ok(None),
        }
    }
}

fn workflow_task_error_message(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|message| !message.is_empty())
                .map(|message| message.chars().take(480).collect())
        })
        .or_else(|| Some("This Workflow stopped before it finished.".to_string()))
}

#[cfg(test)]
mod workflow_task_error_tests {
    use super::workflow_task_error_message;
    use crate::db::PersistenceEngine;
    use rusqlite::params;
    use serde_json::json;

    #[test]
    fn keeps_only_bounded_safe_workflow_error_copy() {
        assert_eq!(
            workflow_task_error_message(Some(
                r#"{"code":"official_page_fetch_failed","message":"The official page returned HTTP 403.","secret":"never project this"}"#,
            ))
            .as_deref(),
            Some("The official page returned HTTP 403.")
        );
        assert_eq!(
            workflow_task_error_message(Some("not-json")).as_deref(),
            Some("This Workflow stopped before it finished.")
        );
        assert_eq!(workflow_task_error_message(None), None);
    }

    #[test]
    fn reconciliation_projects_failure_copy_and_clears_it_after_completion() {
        let root = std::env::temp_dir().join(format!(
            "oomu-remote-routine-error-{}",
            crate::p0_contracts::TaskId::new()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let workflow_id = "workflow-error-projection";
        let instance_id = "wfi-error-projection";
        let task_run_id = "taskrun_77777777-7777-4777-8777-777777777777";
        let task_id = "task_77777777-7777-4777-8777-777777777777";
        let connection = engine.open_connection().unwrap();
        connection.execute(
            "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,is_active,created_at_ms,updated_at_ms,encryption_state) VALUES (?1,1,'Projection','','{}',1,1,1,'test')",
            params![workflow_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO execution_instances(id,workflow_id,workflow_version,status,error_json,created_at_ms,updated_at_ms,encryption_state) VALUES (?1,?2,1,'Failed',?3,1,1,'test')",
            params![instance_id,workflow_id,json!({"code":"official_page_fetch_failed","message":"The official page returned HTTP 403."}).to_string()],
        ).unwrap();
        connection.execute(
            "INSERT INTO task_runs(task_run_id,task_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,'workflow',?3,'running','routine',?2,'Projection',1,1,'reconciled')",
            params![task_run_id,task_id,instance_id],
        ).unwrap();
        drop(connection);

        engine.reconcile_remote_workflow_task(instance_id).unwrap();
        let connection = engine.open_connection().unwrap();
        let (state, error): (String, Option<String>) = connection
            .query_row(
                "SELECT state,last_error FROM task_runs WHERE task_run_id=?1",
                params![task_run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "failed");
        assert_eq!(
            error.as_deref(),
            Some("The official page returned HTTP 403.")
        );
        connection
            .execute(
                "UPDATE execution_instances SET status='Completed',error_json=NULL WHERE id=?1",
                params![instance_id],
            )
            .unwrap();
        drop(connection);

        engine.reconcile_remote_workflow_task(instance_id).unwrap();
        let (state, error): (String, Option<String>) = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT state,last_error FROM task_runs WHERE task_run_id=?1",
                params![task_run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "completed");
        assert!(error.is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
