use super::*;

pub(super) fn ensure_agent_task_run(
    transaction: &rusqlite::Transaction<'_>,
    execution_id: &str,
    project_id: &str,
    agent_id: &str,
    now: i64,
) -> rusqlite::Result<()> {
    let task_id = crate::p0_contracts::TaskId::new().to_string();
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    transaction.execute(
        "INSERT INTO task_runs (
            task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,
            origin,correlation_id,summary,last_error,created_at_ms,updated_at_ms,
            completed_at_ms,recovery_state
         ) VALUES (?1,?2,?3,'agent',?4,'running','agent',?2,?5,NULL,?6,?6,NULL,'reconciled')
         ON CONFLICT(runtime_kind,runtime_record_id) DO NOTHING",
        params![
            task_run_id,
            task_id,
            project_id,
            execution_id,
            format!("Agent execution {agent_id}"),
            now,
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
impl PersistenceEngine {
    pub fn finalize_agent_execution(
        &self,
        execution_id: &str,
        plan_id: &str,
        context: &ChatTurnPersistenceContext,
        context_json: &str,
        terminal_status: &str,
        receipt: Option<&str>,
        log_level: &str,
        log_phase: &str,
        log_message: &str,
        payload_json: Option<&str>,
    ) -> rusqlite::Result<()> {
        validate_chat_turn_context_fields(context)?;
        if !matches!(terminal_status, "completed" | "failed" | "halted") {
            return Err(rusqlite::Error::InvalidParameterName(
                "unsupported agent execution terminal status".to_string(),
            ));
        }
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let ownership: Option<(String, String)> = transaction
            .query_row(
                "SELECT turns.workspace_id,executions.project_id
                 FROM agent_executions executions
                 JOIN chat_turns turns ON turns.turn_id=executions.turn_id
                                      AND turns.generation_token=executions.generation_token
                 JOIN chat_sessions sessions ON sessions.id=turns.session_id
                                            AND sessions.workspace_id=turns.workspace_id
                 WHERE executions.execution_id=?1 AND executions.plan_id=?2
                   AND executions.context_json=?3 AND executions.status='running'
                   AND turns.turn_id=?4 AND turns.generation_token=?5
                   AND turns.session_id=?6 AND turns.agent_id=?7
                   AND turns.provider_id=?8 AND turns.model_id=?9
                   AND turns.root_turn_id=?10 AND turns.turn_kind=?11
                   AND COALESCE(turns.parent_turn_id,'')=COALESCE(?12,'')
                   AND turns.status IN ('running','completed','escalated')
                   AND sessions.agent_id=turns.agent_id",
                params![
                    execution_id,
                    plan_id,
                    context_json,
                    context.turn_id,
                    context.generation_token,
                    context.session_id,
                    context.agent_id,
                    context.provider_id,
                    context.model_id,
                    context.root_turn_id,
                    context.turn_kind,
                    context.parent_turn_id,
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((workspace_id, project_id)) = ownership else {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution terminal persistence lost immutable ownership".to_string(),
            ));
        };
        let now = unix_time_ms();
        ensure_agent_task_run(
            &transaction,
            execution_id,
            &project_id,
            &context.agent_id,
            now,
        )?;
        if let Some(receipt) = receipt {
            transaction.execute(
                "INSERT INTO chat_messages (
                    workspace_id,session_id,agent_id,role,content,provider_id,model_id,
                    metadata_json,is_compacted,compaction_type,timestamp_ms
                 ) VALUES (?1,?2,?3,'assistant',?4,?5,?6,?7,0,'raw',?8)",
                params![
                    workspace_id,
                    context.session_id,
                    context.agent_id,
                    receipt,
                    context.provider_id,
                    context.model_id,
                    payload_json,
                    now,
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO agent_execution_logs (
                execution_id,plan_id,session_id,agent_id,level,phase,message,
                payload_json,created_at_ms,encryption_state
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                execution_id,
                plan_id,
                context.session_id,
                context.agent_id,
                log_level,
                log_phase,
                log_message,
                payload_json,
                now,
                get_current_encryption_state(),
            ],
        )?;
        let changed = transaction.execute(
            "UPDATE agent_executions SET status=?1,updated_at_ms=?2
             WHERE execution_id=?3 AND status='running'",
            params![terminal_status, now, execution_id],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "agent execution was cancelled before terminal persistence".to_string(),
            ));
        }
        synchronize_owned_lifecycle(
            &transaction,
            &workspace_id,
            execution_id,
            plan_id,
            context,
            terminal_status,
            log_message,
            payload_json,
            now,
        )?;
        transaction.commit()
    }
}

#[allow(clippy::too_many_arguments)]
fn synchronize_owned_lifecycle(
    transaction: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    execution_id: &str,
    plan_id: &str,
    context: &ChatTurnPersistenceContext,
    terminal_status: &str,
    safe_error: &str,
    payload_json: Option<&str>,
    now: i64,
) -> rusqlite::Result<()> {
    let (turn_status, task_state, plan_state, action_state, recovery_state) = match terminal_status
    {
        "completed" => ("completed", "completed", "completed", None, "reconciled"),
        "halted" => (
            "escalated",
            "blocked",
            "recoverable",
            Some("blocked"),
            "recoverable",
        ),
        _ => ("failed", "failed", "failed", Some("failed"), "reconciled"),
    };
    transaction.execute(
        "UPDATE chat_turns SET status=?1,completed_at_ms=?2
         WHERE turn_id=?3 AND generation_token=?4
           AND status IN ('running','completed','escalated')",
        params![turn_status, now, context.turn_id, context.generation_token],
    )?;
    transaction.execute(
        "UPDATE chat_messages
         SET metadata_json=json_set(COALESCE(metadata_json,'{}'),'$.turnState',?1)
         WHERE workspace_id=?2 AND session_id=?3
           AND role='user'
           AND json_extract(metadata_json,'$.turnId')=?4
           AND json_extract(metadata_json,'$.generationToken')=?5",
        params![
            turn_status,
            workspace_id,
            context.session_id,
            context.turn_id,
            context.generation_token,
        ],
    )?;
    if let Some(action_state) = action_state {
        transaction.execute(
            "UPDATE actions SET status=?1,output=COALESCE(?2,output)
             WHERE plan_id=?3 AND status='running'",
            params![action_state, payload_json, plan_id],
        )?;
    }
    transaction.execute(
        "UPDATE plan_generation_states SET status=?1,generated_text=?2,timestamp_ms=?3
         WHERE plan_id=?4",
        params![plan_state, safe_error, now, plan_id],
    )?;
    let last_error = (terminal_status != "completed").then_some(safe_error);
    let completed_at = (terminal_status != "halted").then_some(now);
    transaction.execute(
        "UPDATE task_runs SET state=?1,last_error=?2,updated_at_ms=?3,
             completed_at_ms=?4,recovery_state=?5
         WHERE runtime_kind='agent' AND runtime_record_id=?6",
        params![
            task_state,
            last_error,
            now,
            completed_at,
            recovery_state,
            execution_id,
        ],
    )?;
    Ok(())
}
