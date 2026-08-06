use super::*;

#[test]
fn execution_checkpoint_requires_exact_plan_and_exact_completed_receipt_count() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-execution-checkpoint-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let plan_json = r#"{"id":"plan-checkpoint","steps":[1,2]}"#;
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO plan_generation_states (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms) VALUES (?1,?2,1,'checkpointed','one complete',?3)",
            params!["plan-checkpoint", plan_json, unix_time_ms()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms) VALUES (?1,'create_decision_pack','{}',?2,'completed',?3)",
            params![
                "plan-checkpoint",
                r#"{"operation":"create_decision_pack","status":"completed","message":"done","metrics":null,"claims":[],"verified":true,"model_used":null}"#,
                unix_time_ms()
            ],
        )
        .unwrap();
    drop(connection);

    let checkpoint = engine
        .load_plan_execution_checkpoint("plan-checkpoint", plan_json, 2)
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.next_step_index, 1);
    assert_eq!(checkpoint.completed_actions.len(), 1);
    assert!(engine
        .load_plan_execution_checkpoint("plan-checkpoint", "{}", 2)
        .is_err());

    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE plan_generation_states SET current_step_index=2 WHERE plan_id='plan-checkpoint'",
            [],
        )
        .unwrap();
    drop(connection);
    assert!(engine
        .load_plan_execution_checkpoint("plan-checkpoint", plan_json, 2)
        .is_err());
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn completed_action_receipt_and_plan_cursor_commit_together() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-atomic-action-checkpoint-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let plan_json = r#"{"id":"plan-atomic","steps":[1]}"#;
    let output = r#"{"operation":"create_decision_pack","status":"completed","message":"done","metrics":null,"claims":[],"verified":true,"model_used":null}"#;
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO plan_generation_states (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms) VALUES (?1,?2,0,'running','running',?3)",
            params!["plan-atomic", plan_json, unix_time_ms()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms) VALUES (?1,'create_decision_pack','{}',NULL,'started_effectful',?2)",
            params!["plan-atomic", unix_time_ms()],
        )
        .unwrap();
    let action_id = connection.last_insert_rowid();
    drop(connection);

    engine
        .commit_completed_agent_action_checkpoint(
            action_id,
            "plan-atomic",
            plan_json,
            1,
            output,
            "Completed step 1 of 1.",
        )
        .unwrap();

    let checkpoint = engine
        .load_plan_execution_checkpoint("plan-atomic", plan_json, 1)
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.next_step_index, 1);
    assert_eq!(
        checkpoint.completed_actions,
        vec![(action_id, output.into())]
    );
    let connection = engine.open_connection().unwrap();
    let state: (String, i64, String, String) = connection
        .query_row(
            "SELECT a.status,p.current_step_index,p.status,p.generated_text
             FROM actions a JOIN plan_generation_states p ON p.plan_id=a.plan_id
             WHERE a.id=?1",
            params![action_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        (
            "completed".into(),
            1,
            "checkpointed".into(),
            "Completed step 1 of 1.".into()
        )
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn failed_plan_cursor_write_rolls_back_completed_action_receipt() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu-action-checkpoint-rollback-{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let plan_json = r#"{"id":"plan-rollback","steps":[1]}"#;
    let output = r#"{"operation":"create_decision_pack","status":"completed","message":"done","metrics":null,"claims":[],"verified":true,"model_used":null}"#;
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO plan_generation_states (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms) VALUES (?1,?2,0,'running','running',?3)",
            params!["plan-rollback", plan_json, unix_time_ms()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms) VALUES (?1,'create_decision_pack','{}',NULL,'started_effectful',?2)",
            params!["plan-rollback", unix_time_ms()],
        )
        .unwrap();
    let action_id = connection.last_insert_rowid();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_checkpoint_advance
             BEFORE UPDATE OF current_step_index ON plan_generation_states
             BEGIN
                 SELECT RAISE(ABORT, 'injected checkpoint write failure');
             END;",
        )
        .unwrap();
    drop(connection);

    assert!(engine
        .commit_completed_agent_action_checkpoint(
            action_id,
            "plan-rollback",
            plan_json,
            1,
            output,
            "Completed step 1 of 1.",
        )
        .is_err());

    let connection = engine.open_connection().unwrap();
    let state: (String, Option<String>, i64, String) = connection
        .query_row(
            "SELECT a.status,a.output,p.current_step_index,p.status
             FROM actions a JOIN plan_generation_states p ON p.plan_id=a.plan_id
             WHERE a.id=?1",
            params![action_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        ("started_effectful".into(), None, 0, "running".into())
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn interrupted_agent_execution_resumes_the_same_turn_and_execution_record() {
    let temp_dir = std::env::temp_dir().join(format!("oomu-agent-resume-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-resume".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Resume supplier pack".to_string()),
            dynamic_routing_override: None,
            workspace_id: Some(engine.workspace_id.clone()),
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-resume".to_string(),
        generation_token: "generation-resume".to_string(),
        session_id: session.id,
        agent_id: "agent-resume".to_string(),
        provider_id: "local_model".to_string(),
        model_id: "gemma-test".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-resume".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.begin_chat_turn(&context).unwrap();
    engine.finish_chat_turn(&context, "completed").unwrap();
    engine
        .begin_agent_execution(
            "execution-resume",
            "plan-resume",
            &context,
            r#"{"durable":true}"#,
        )
        .unwrap();

    engine.mark_interrupted_actions().unwrap();
    assert_eq!(
        engine
            .load_resumable_agent_execution_request("execution-resume")
            .unwrap(),
        r#"{"durable":true}"#
    );
    engine
        .resume_agent_execution(
            "execution-resume",
            "plan-resume",
            &context,
            r#"{"durable":true}"#,
        )
        .unwrap();

    let connection = engine.open_connection().unwrap();
    let (execution_status, turn_status): (String, String) = connection
        .query_row(
            "SELECT a.status,t.status FROM agent_executions a JOIN chat_turns t ON t.turn_id=a.turn_id AND t.generation_token=a.generation_token WHERE a.execution_id='execution-resume'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(execution_status, "running");
    assert_eq!(turn_status, "running");
    drop(connection);
    assert!(engine
        .resume_agent_execution(
            "execution-resume",
            "plan-resume",
            &context,
            r#"{"durable":true}"#,
        )
        .is_err());
    let _ = std::fs::remove_dir_all(temp_dir);
}

fn assert_halted_terminal_state(engine: &PersistenceEngine, receipt: &str) {
    let connection = engine.open_connection().unwrap();
    let states: (String, String, String, String, String, Option<String>) = connection
        .query_row(
            "SELECT e.status,t.status,p.status,a.status,r.state,r.last_error
             FROM agent_executions e
             JOIN chat_turns t ON t.turn_id=e.turn_id
             JOIN plan_generation_states p ON p.plan_id=e.plan_id
             JOIN actions a ON a.plan_id=e.plan_id
             JOIN task_runs r ON r.runtime_kind='agent' AND r.runtime_record_id=e.execution_id
             WHERE e.execution_id='execution-terminal'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(states.0, "halted");
    assert_eq!(states.1, "escalated");
    assert_eq!(states.2, "recoverable");
    assert_eq!(states.3, "prepared_effectful");
    assert_eq!(states.4, "blocked");
    assert_eq!(
        states.5.as_deref(),
        Some("Recent official freight evidence was not verified.")
    );
    let (content, metadata, log_payload): (String, String, String) = connection
        .query_row(
            "SELECT m.content,m.metadata_json,l.payload_json
             FROM chat_messages m JOIN agent_execution_logs l ON l.execution_id='execution-terminal'
             WHERE m.content=?1 AND l.phase='halted' ORDER BY l.id DESC LIMIT 1",
            params![receipt],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(content, receipt);
    assert_eq!(metadata, receipt);
    assert_eq!(log_payload, receipt);
}

#[test]
fn agent_terminal_transaction_synchronizes_receipt_plan_action_turn_and_task() {
    let temp_dir = std::env::temp_dir().join(format!("oomu-agent-terminal-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-terminal".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Terminal lifecycle".to_string()),
            dynamic_routing_override: None,
            workspace_id: Some(engine.workspace_id.clone()),
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-terminal".to_string(),
        generation_token: "generation-terminal".to_string(),
        session_id: session.id,
        agent_id: "agent-terminal".to_string(),
        provider_id: "local_model".to_string(),
        model_id: "gemma-test".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-terminal".to_string(),
        turn_kind: "root".to_string(),
    };
    let context_json = r#"{"durable":true}"#;
    engine.begin_chat_turn(&context).unwrap();
    engine.finish_chat_turn(&context, "completed").unwrap();
    engine
        .begin_agent_execution(
            "execution-terminal",
            "plan-terminal",
            &context,
            context_json,
        )
        .unwrap();
    let receipt = r#"{"schema":"oomu.agent_execution_recovery.v1","executionId":"execution-terminal","planId":"plan-terminal","code":"decision_pack_research_evidence_unavailable","boundary":"DecisionPack","recoverable":true,"recoveryAction":"resume_same_execution","message":"Recent official freight evidence was not verified.","context":{"subject":"freight","attemptCount":3,"pageCount":4,"verifiedInputCount":2},"changedState":"none"}"#;
    let connection = engine.open_connection().unwrap();
    connection.execute("INSERT INTO plan_generation_states (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms) VALUES ('plan-terminal','{}',0,'running','running',1)", []).unwrap();
    connection.execute("INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms) VALUES ('plan-terminal','create_decision_pack','{}',NULL,'prepared_effectful',1)", []).unwrap();
    drop(connection);

    engine
        .finalize_agent_execution(
            "execution-terminal",
            "plan-terminal",
            &context,
            context_json,
            "halted",
            Some(receipt),
            "error",
            "halted",
            "Recent official freight evidence was not verified.",
            Some(receipt),
        )
        .unwrap();

    assert_halted_terminal_state(&engine, receipt);

    let resume_cursor = engine
        .resume_agent_execution(
            "execution-terminal",
            "plan-terminal",
            &context,
            context_json,
        )
        .unwrap();
    assert!(resume_cursor > 0);
    let resumed_logs = engine
        .select_agent_execution_logs_after("execution-terminal", resume_cursor, 100)
        .unwrap();
    assert!(resumed_logs.iter().any(|log| log.phase == "resumed"));
    assert!(resumed_logs.iter().all(|log| !log.is_terminal()));
    let connection = engine.open_connection().unwrap();
    let resumed: (String, String, String, String, Option<String>) = connection
        .query_row(
            "SELECT e.status,t.status,p.status,r.state,r.last_error
             FROM agent_executions e JOIN chat_turns t ON t.turn_id=e.turn_id
             JOIN plan_generation_states p ON p.plan_id=e.plan_id
             JOIN task_runs r ON r.runtime_kind='agent' AND r.runtime_record_id=e.execution_id
             WHERE e.execution_id='execution-terminal'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        resumed,
        (
            "running".into(),
            "running".into(),
            "running".into(),
            "running".into(),
            None
        )
    );
    drop(connection);

    engine
        .finalize_agent_execution(
            "execution-terminal",
            "plan-terminal",
            &context,
            context_json,
            "completed",
            None,
            "info",
            "completed",
            "Execution completed after resume.",
            None,
        )
        .unwrap();
    let completed_logs = engine
        .select_agent_execution_logs_after("execution-terminal", resume_cursor, 100)
        .unwrap();
    assert!(completed_logs.iter().any(|log| log.phase == "resumed"));
    assert!(completed_logs
        .iter()
        .any(AgentExecutionLogRecord::is_terminal));
    assert_eq!(
        completed_logs.last().map(|log| log.phase.as_str()),
        Some("completed")
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn failed_agent_replan_requires_safe_receipt_and_no_uncertain_action_effect() {
    let temp_dir = std::env::temp_dir().join(format!("oomu-agent-replan-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();

    for (suffix, recovery_action, action_status, expected) in [
        ("new", Some("start_new_plan"), None, true),
        ("legacy", None, None, true),
        (
            "prepared",
            Some("start_new_plan"),
            Some("prepared_effectful"),
            true,
        ),
        (
            "read-only",
            Some("start_new_plan"),
            Some("started_read_only"),
            true,
        ),
        (
            "effect-started",
            Some("start_new_plan"),
            Some("started_effectful"),
            false,
        ),
        ("legacy-action", None, Some("prepared_effectful"), false),
        ("legacy-failed", None, Some("failed"), false),
        ("review", Some("review_external_changes"), None, false),
        (
            "review-external",
            Some("review_external_changes"),
            Some("started_effectful"),
            true,
        ),
        (
            "decision-pack-checkpoint-review",
            Some("review_external_changes"),
            Some("completed"),
            true,
        ),
        (
            "completed",
            Some("start_new_plan"),
            Some("completed"),
            false,
        ),
    ] {
        let agent_id = format!("agent-replan-{suffix}");
        let execution_id = format!("execution-replan-{suffix}");
        let plan_id = format!("plan-replan-{suffix}");
        let objective = format!("Prepare the supplier decision pack ({suffix})");
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: agent_id.clone(),
                provider_id: "local_model".to_string(),
                model_id: "gemma-test".to_string(),
                title: Some(format!("Replan {suffix}")),
                dynamic_routing_override: None,
                workspace_id: Some(engine.workspace_id.clone()),
            })
            .unwrap();
        let session_id = session.id.clone();
        let context = ChatTurnPersistenceContext {
            turn_id: format!("turn-replan-{suffix}"),
            generation_token: format!("generation-replan-{suffix}"),
            session_id: session_id.clone(),
            agent_id,
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            parent_turn_id: None,
            root_turn_id: format!("turn-replan-{suffix}"),
            turn_kind: "root".to_string(),
        };
        let plan_steps = (suffix == "decision-pack-checkpoint-review").then(|| {
            json!([
                {"tool":{"operation":"create_decision_pack"}},
                {"tool":{"operation":"create_conflict_free_calendar_event"}},
                {"tool":{"operation":"draft_decision_pack_email"}}
            ])
        });
        let context_json = json!({
            "plan": {"id": plan_id, "objective": objective, "steps": plan_steps},
            "turn_context": {"sessionId": session_id},
        })
        .to_string();
        engine.begin_chat_turn(&context).unwrap();
        engine.finish_chat_turn(&context, "completed").unwrap();
        engine
            .begin_agent_execution(&execution_id, &plan_id, &context, &context_json)
            .unwrap();

        let connection = engine.open_connection().unwrap();
        connection
            .execute(
                "INSERT INTO plan_generation_states
                 (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms)
                 VALUES (?1,'{}',?2,'running','running',1)",
                params![
                    plan_id,
                    i64::from(suffix == "decision-pack-checkpoint-review")
                ],
            )
            .unwrap();
        if let Some(action_status) = action_status {
            connection
                .execute(
                    "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
                     VALUES (?1,'create_decision_pack','{}',?2,?3,1)",
                    params![
                        plan_id,
                        (action_status == "completed")
                            .then_some(r#"{"status":"completed","verified":true}"#),
                        action_status,
                    ],
                )
                .unwrap();
        }
        if suffix == "decision-pack-checkpoint-review" {
            connection
                .execute(
                    "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
                     VALUES (?1,'create_conflict_free_calendar_event','{}',NULL,'prepared_effectful',2)",
                    params![plan_id],
                )
                .unwrap();
        }
        drop(connection);

        let mut receipt = json!({
            "schema": "oomu.agent_execution_recovery.v1",
            "executionId": execution_id,
            "planId": plan_id,
            "code": "preflight_verification_failed",
            "boundary": "MlcVerifier",
            "recoverable": false,
            "message": "Execution stopped at a safe boundary.",
            "context": {},
            "changedState": if suffix == "review-external" {
                "external_changes"
            } else if suffix == "decision-pack-checkpoint-review" {
                "checkpoint_saved"
            } else {
                "none"
            },
        });
        if let Some(recovery_action) = recovery_action {
            receipt
                .as_object_mut()
                .unwrap()
                .insert("recoveryAction".to_string(), json!(recovery_action));
        }
        let receipt = receipt.to_string();
        engine
            .finalize_agent_execution(
                &execution_id,
                &plan_id,
                &context,
                &context_json,
                "failed",
                Some(&receipt),
                "error",
                "failed",
                "Execution stopped at a safe boundary.",
                Some(&receipt),
            )
            .unwrap();

        let prepared = engine
            .prepare_agent_execution_replan(&execution_id, &session_id)
            .unwrap();
        if expected {
            assert_eq!(prepared.as_deref(), Some(objective.as_str()));
        } else {
            assert_eq!(prepared, None);
        }
        assert_eq!(
            engine
                .prepare_agent_execution_replan(&execution_id, "another-session")
                .unwrap(),
            None
        );
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn completed_agent_action_receipt_reuse_is_bound_to_the_exact_objective() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-agent-receipt-reuse-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let objective = "Prepare the exact supplier decision pack";
    let plan_id = "plan-receipt-reuse";
    let execution_id = "execution-receipt-reuse";
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-receipt-reuse".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Receipt reuse".to_string()),
            dynamic_routing_override: None,
            workspace_id: Some(engine.workspace_id.clone()),
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-receipt-reuse".to_string(),
        generation_token: "generation-receipt-reuse".to_string(),
        session_id: session.id,
        agent_id: "agent-receipt-reuse".to_string(),
        provider_id: "local_model".to_string(),
        model_id: "gemma-test".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-receipt-reuse".to_string(),
        turn_kind: "root".to_string(),
    };
    let context_json = json!({
        "plan": {"id": plan_id, "objective": objective},
        "turn_context": {"sessionId": context.session_id},
    })
    .to_string();
    engine.begin_chat_turn(&context).unwrap();
    engine
        .begin_agent_execution(execution_id, plan_id, &context, &context_json)
        .unwrap();
    let output = r#"{"operation":"create_decision_pack","status":"completed","message":"{}","metrics":null,"claims":[],"verified":true,"model_used":null}"#;
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
             VALUES (?1,'create_decision_pack','{}',?2,'completed',1)",
            params![plan_id, output],
        )
        .unwrap();

    assert_eq!(
        engine
            .completed_agent_action_outputs_for_objective("create_decision_pack", objective)
            .unwrap(),
        vec![output.to_string()]
    );
    assert!(engine
        .completed_agent_action_outputs_for_objective(
            "create_decision_pack",
            "Prepare another supplier decision pack",
        )
        .unwrap()
        .is_empty());
    let _ = std::fs::remove_dir_all(temp_dir);
}
