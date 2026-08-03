use super::*;

impl PersistenceEngine {
    pub fn cancel_agent_execution_remaining_work(
        &self,
        execution_id: &str,
        session_id: &str,
    ) -> rusqlite::Result<usize> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let (plan_id, step_index, status, payload_json): (String, i64, String, String) =
            transaction.query_row(
                "SELECT executions.plan_id,state.current_step_index,executions.status,
                    (SELECT logs.payload_json FROM agent_execution_logs logs
                     WHERE logs.execution_id=executions.execution_id
                       AND logs.payload_json IS NOT NULL
                     ORDER BY logs.id DESC LIMIT 1)
             FROM agent_executions executions
             JOIN plan_generation_states state ON state.plan_id=executions.plan_id
             WHERE executions.execution_id=?1 AND executions.session_id=?2",
                params![execution_id, session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        let receipt = serde_json::from_str::<Value>(&payload_json).map_err(|_| {
            rusqlite::Error::InvalidParameterName(
                "agent recovery receipt is unavailable".to_string(),
            )
        })?;
        if status == "cancelled"
            && receipt.get("schema").and_then(Value::as_str)
                == Some(crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA)
            && receipt.get("executionId").and_then(Value::as_str) == Some(execution_id)
            && receipt.get("planId").and_then(Value::as_str) == Some(plan_id.as_str())
            && receipt.get("recoveryAction").and_then(Value::as_str)
                == Some("remaining_work_cancelled")
            && receipt.get("code").and_then(Value::as_str)
                == Some("agent_execution_remaining_work_cancelled")
        {
            let completed_step_count = receipt
                .pointer("/context/completedStepCount")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName(
                        "agent execution cancellation receipt is invalid".to_string(),
                    )
                })?;
            return Ok(completed_step_count);
        }
        if status != "halted" {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution is not awaiting remaining-work recovery".to_string(),
            ));
        }
        let modern_recovery_receipt = receipt.get("schema").and_then(Value::as_str)
            == Some(crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA);
        let safe_recovery =
            recovery_receipt_authorizes_resume(&payload_json, execution_id, &plan_id).is_some();
        let recovery_action = receipt.get("recoveryAction").and_then(Value::as_str);
        if !modern_recovery_receipt
            || !safe_recovery
            || recovery_action != Some("resume_same_execution")
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution is not awaiting safe remaining-work recovery".to_string(),
            ));
        }
        let evidence = agent_action_recovery_evidence(&transaction, &plan_id)?;
        if evidence.has_uncertain_effect {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution has an unresolved action effect and cannot be cancelled from recovery"
                    .to_string(),
            ));
        }
        let completed_step_count = usize::try_from(step_index).map_err(|_| {
            rusqlite::Error::InvalidParameterName(
                "agent execution checkpoint is invalid".to_string(),
            )
        })?;
        let cancellation_payload = serde_json::json!({
            "schema": crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA,
            "executionId": execution_id,
            "planId": plan_id,
            "code": "agent_execution_remaining_work_cancelled",
            "boundary": "AgentExecutionRecovery",
            "recoverable": false,
            "recoveryAction": "remaining_work_cancelled",
            "message": "The remaining work was stopped. Any completed, verified steps were kept.",
            "context": {
                "cancelled": true,
                "completedStepCount": completed_step_count,
            },
            "changedState": if completed_step_count > 0 { "checkpoint_saved" } else { "none" },
        })
        .to_string();
        let now = unix_time_ms();
        let changed = transaction.execute(
            "UPDATE agent_executions SET status='cancelled',updated_at_ms=?1
             WHERE execution_id=?2 AND session_id=?3 AND status='halted'",
            params![now, execution_id, session_id],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution remaining work could not be cancelled".to_string(),
            ));
        }
        let message_changed = transaction.execute(
            "UPDATE chat_messages SET content=?1,metadata_json=?1,timestamp_ms=?2
             WHERE id=(
                 SELECT id FROM chat_messages
                 WHERE session_id=?3 AND content=?4 AND metadata_json=?4
                 ORDER BY id DESC LIMIT 1
             )",
            params![cancellation_payload, now, session_id, payload_json],
        )?;
        if message_changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution cancellation message compare-and-swap failed".to_string(),
            ));
        }
        transaction.execute(
            "UPDATE chat_turns SET status='completed',completed_at_ms=?1
             WHERE (turn_id,generation_token)=(
                 SELECT turn_id,generation_token FROM agent_executions WHERE execution_id=?2
             ) AND status='escalated'",
            params![now, execution_id],
        )?;
        transaction.execute(
            "UPDATE task_runs SET state='cancelled',last_error='Remaining work stopped by user.',
                    completed_at_ms=?1,updated_at_ms=?1,recovery_state='reconciled'
             WHERE runtime_kind='agent' AND runtime_record_id=?2",
            params![now, execution_id],
        )?;
        transaction.execute(
            "UPDATE plan_generation_states SET status='cancelled',
                    generated_text='Remaining work stopped by user.',timestamp_ms=?1
             WHERE plan_id=?2",
            params![now, plan_id],
        )?;
        transaction.execute(
            "INSERT INTO agent_execution_logs (
                execution_id,plan_id,session_id,agent_id,level,phase,message,
                payload_json,created_at_ms,encryption_state
             ) SELECT execution_id,plan_id,session_id,agent_id,'info','cancelled',
                      'Remaining work was stopped by the user.',?1,?2,?3
               FROM agent_executions WHERE execution_id=?4",
            params![
                cancellation_payload,
                now,
                get_current_encryption_state(),
                execution_id,
            ],
        )?;
        transaction.commit()?;
        Ok(completed_step_count)
    }
}
