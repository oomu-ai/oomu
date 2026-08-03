use super::*;

pub(crate) const MAX_AGENT_EXECUTION_RECOVERY_STATE_IDS: usize = 64;
const MAX_AGENT_EXECUTION_RECOVERY_ID_BYTES: usize = 128;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionRecoveryStateRecord {
    pub execution_id: String,
    pub plan_id: String,
    pub session_id: String,
    pub root_turn_id: String,
    pub failed_turn_id: String,
    pub generation_token: String,
    pub status: String,
    pub terminal_phase: Option<String>,
    pub terminal_verified: bool,
    pub verified_complete: bool,
}

fn valid_recovery_execution_id(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_AGENT_EXECUTION_RECOVERY_ID_BYTES {
        return false;
    }
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | ':' | '-')
        })
}

pub(crate) fn clean_agent_execution_recovery_state_query(
    session_id: String,
    execution_ids: Vec<String>,
) -> Result<(String, Vec<String>), AgenticLoopError> {
    let session_id = clean_session_config_id(session_id)?;
    if execution_ids.len() > MAX_AGENT_EXECUTION_RECOVERY_STATE_IDS {
        return Err(AgenticLoopError::from_persistence(format!(
            "Recovery-state lookup accepts at most {MAX_AGENT_EXECUTION_RECOVERY_STATE_IDS} execution IDs."
        )));
    }

    let mut cleaned = Vec::with_capacity(execution_ids.len());
    for execution_id in execution_ids {
        let execution_id = execution_id.trim();
        if !valid_recovery_execution_id(execution_id) {
            return Err(AgenticLoopError::from_persistence(
                "Recovery-state lookup received an invalid execution ID.".to_string(),
            ));
        }
        if !cleaned.iter().any(|existing| existing == execution_id) {
            cleaned.push(execution_id.to_string());
        }
    }
    Ok((session_id, cleaned))
}

fn terminal_verified_evidence(
    status: &str,
    phase: Option<&str>,
    payload_json: Option<&str>,
) -> bool {
    if status != "completed" || phase != Some("completed") {
        return false;
    }
    payload_json
        .and_then(|payload| serde_json::from_str::<Value>(payload).ok())
        .and_then(|payload| payload.as_object()?.get("verified")?.as_bool())
        == Some(true)
}

impl PersistenceEngine {
    pub fn select_agent_execution_recovery_states(
        &self,
        session_id: &str,
        execution_ids: &[String],
    ) -> rusqlite::Result<Vec<AgentExecutionRecoveryStateRecord>> {
        if execution_ids.is_empty() {
            return Ok(Vec::new());
        }
        if execution_ids.len() > MAX_AGENT_EXECUTION_RECOVERY_STATE_IDS {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "Recovery-state lookup accepts at most {MAX_AGENT_EXECUTION_RECOVERY_STATE_IDS} execution IDs."
            )));
        }

        let placeholders = std::iter::repeat("?")
            .take(execution_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT executions.execution_id, executions.plan_id, executions.session_id,
                    executions.root_turn_id, executions.turn_id, executions.generation_token,
                    executions.status, terminal.phase, terminal.payload_json
             FROM agent_executions executions
             JOIN chat_sessions sessions
               ON sessions.id=executions.session_id
              AND sessions.agent_id=executions.agent_id
              AND sessions.workspace_id=?
             LEFT JOIN agent_execution_logs terminal
               ON terminal.id=(
                    SELECT logs.id
                    FROM agent_execution_logs logs
                    WHERE logs.execution_id=executions.execution_id
                      AND logs.plan_id=executions.plan_id
                      AND logs.phase IN ('completed','failed','halted','restart_recovery_ready')
                    ORDER BY logs.id DESC
                    LIMIT 1
               )
             WHERE executions.session_id=?
               AND executions.execution_id IN ({placeholders})
             ORDER BY executions.updated_at_ms DESC, executions.execution_id ASC"
        );
        let mut parameters = Vec::<rusqlite::types::Value>::with_capacity(execution_ids.len() + 2);
        parameters.push(self.workspace_id.clone().into());
        parameters.push(session_id.to_string().into());
        parameters.extend(execution_ids.iter().cloned().map(Into::into));

        let connection = self.open_connection()?;
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(parameters), |row| {
            let execution_id = row.get::<_, String>(0)?;
            let plan_id = row.get::<_, String>(1)?;
            let session_id = row.get::<_, String>(2)?;
            let root_turn_id = row.get::<_, String>(3)?;
            let failed_turn_id = row.get::<_, String>(4)?;
            let generation_token = row.get::<_, String>(5)?;
            let status = row.get::<_, String>(6)?;
            let terminal_phase = row.get::<_, Option<String>>(7)?;
            let terminal_payload = row.get::<_, Option<String>>(8)?;
            let terminal_verified = terminal_verified_evidence(
                &status,
                terminal_phase.as_deref(),
                terminal_payload.as_deref(),
            );
            Ok(AgentExecutionRecoveryStateRecord {
                execution_id,
                plan_id,
                session_id,
                root_turn_id,
                failed_turn_id,
                generation_token,
                status,
                terminal_phase,
                terminal_verified,
                verified_complete: terminal_verified,
            })
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_state_query_is_bounded_and_rejects_unsafe_ids() {
        let valid = clean_agent_execution_recovery_state_query(
            " session-1 ".to_string(),
            vec!["execution-1".to_string(), " execution-1 ".to_string()],
        )
        .unwrap();
        assert_eq!(
            valid,
            ("session-1".to_string(), vec!["execution-1".to_string()])
        );

        assert!(clean_agent_execution_recovery_state_query(
            "session-1".to_string(),
            vec!["execution/foreign".to_string()],
        )
        .is_err());
        assert!(clean_agent_execution_recovery_state_query(
            "session-1".to_string(),
            vec!["execution-1".to_string(); MAX_AGENT_EXECUTION_RECOVERY_STATE_IDS + 1],
        )
        .is_err());
    }

    #[test]
    fn terminal_verification_requires_completed_structured_evidence() {
        assert_eq!(
            terminal_verified_evidence(
                "completed",
                Some("completed"),
                Some(r#"{"verified":true}"#)
            ),
            true
        );
        assert_eq!(
            terminal_verified_evidence(
                "completed",
                Some("completed"),
                Some(r#"{"verified":false}"#)
            ),
            false
        );
        assert_eq!(
            terminal_verified_evidence("completed", Some("completed"), Some("not-json")),
            false
        );
        assert_eq!(
            terminal_verified_evidence("halted", Some("completed"), Some(r#"{"verified":true}"#)),
            false
        );
    }

    #[test]
    fn recovery_state_record_serializes_with_the_frontend_contract() {
        let serialized = serde_json::to_value(AgentExecutionRecoveryStateRecord {
            execution_id: "execution-1".to_string(),
            plan_id: "plan-1".to_string(),
            session_id: "session-1".to_string(),
            root_turn_id: "turn-root".to_string(),
            failed_turn_id: "turn-failed".to_string(),
            generation_token: "generation-1".to_string(),
            status: "completed".to_string(),
            terminal_phase: Some("completed".to_string()),
            terminal_verified: true,
            verified_complete: true,
        })
        .unwrap();
        assert_eq!(serialized["executionId"], "execution-1");
        assert_eq!(serialized["planId"], "plan-1");
        assert_eq!(serialized["sessionId"], "session-1");
        assert_eq!(serialized["rootTurnId"], "turn-root");
        assert_eq!(serialized["failedTurnId"], "turn-failed");
        assert_eq!(serialized["generationToken"], "generation-1");
        assert_eq!(serialized["terminalPhase"], "completed");
        assert_eq!(serialized["terminalVerified"], true);
        assert_eq!(serialized["verifiedComplete"], true);
        assert!(serialized.get("execution_id").is_none());

        let without_terminal = serde_json::to_value(AgentExecutionRecoveryStateRecord {
            execution_id: "execution-2".to_string(),
            plan_id: "plan-2".to_string(),
            session_id: "session-2".to_string(),
            root_turn_id: "turn-root-2".to_string(),
            failed_turn_id: "turn-failed-2".to_string(),
            generation_token: "generation-2".to_string(),
            status: "running".to_string(),
            terminal_phase: None,
            terminal_verified: false,
            verified_complete: false,
        })
        .unwrap();
        assert!(without_terminal["terminalPhase"].is_null());
        assert_eq!(without_terminal["terminalVerified"], false);
    }
}
