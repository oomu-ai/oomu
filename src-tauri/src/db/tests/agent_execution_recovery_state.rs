use super::*;

fn session_fixture(engine: &PersistenceEngine, suffix: &str) -> ChatSessionRecord {
    engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: format!("agent-{suffix}"),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some(format!("Recovery state {suffix}")),
            dynamic_routing_override: None,
            workspace_id: Some(engine.workspace_id.clone()),
        })
        .unwrap()
}

fn execution_fixture(
    engine: &PersistenceEngine,
    suffix: &str,
    session: &ChatSessionRecord,
) -> (String, String, ChatTurnPersistenceContext, String) {
    let plan_id = format!("plan-{suffix}");
    let execution_id = format!("execution-{suffix}");
    let context = ChatTurnPersistenceContext {
        turn_id: format!("turn-{suffix}"),
        generation_token: format!("generation-{suffix}"),
        session_id: session.id.clone(),
        agent_id: session.agent_id.clone(),
        provider_id: "local_model".to_string(),
        model_id: "gemma-test".to_string(),
        parent_turn_id: None,
        root_turn_id: format!("turn-{suffix}"),
        turn_kind: "root".to_string(),
    };
    let context_json = json!({
        "plan": { "id": plan_id },
        "turn_context": { "sessionId": session.id },
    })
    .to_string();
    engine
        .accept_chat_turn(AcceptChatTurnRequest {
            turn_id: context.turn_id.clone(),
            generation_token: context.generation_token.clone(),
            parent_turn_id: context.parent_turn_id.clone(),
            root_turn_id: context.root_turn_id.clone(),
            turn_kind: context.turn_kind.clone(),
            session_id: context.session_id.clone(),
            agent_id: context.agent_id.clone(),
            provider_id: context.provider_id.clone(),
            model_id: context.model_id.clone(),
            message: format!("Run the {suffix} task."),
        })
        .unwrap();
    engine
        .begin_agent_execution(&execution_id, &plan_id, &context, &context_json)
        .unwrap();
    (execution_id, plan_id, context, context_json)
}

#[test]
fn recovery_state_projection_distinguishes_verified_completion_from_halt() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_agent_execution_recovery_state_{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = session_fixture(&engine, "shared");
    let (completed_id, completed_plan, completed_context, completed_json) =
        execution_fixture(&engine, "verified", &session);
    engine
        .finalize_agent_execution(
            &completed_id,
            &completed_plan,
            &completed_context,
            &completed_json,
            "completed",
            Some("Verified completion"),
            "info",
            "completed",
            "Execution completed.",
            Some(r#"{"verified":true}"#),
        )
        .unwrap();
    let (halted_id, halted_plan, halted_context, halted_json) =
        execution_fixture(&engine, "halted", &session);
    engine
        .finalize_agent_execution(
            &halted_id,
            &halted_plan,
            &halted_context,
            &halted_json,
            "halted",
            None,
            "warn",
            "halted",
            "Approval remains pending.",
            Some(r#"{"verified":true}"#),
        )
        .unwrap();

    let states = engine
        .select_agent_execution_recovery_states(
            &session.id,
            &[completed_id.clone(), halted_id.clone()],
        )
        .unwrap();
    let completed = states
        .iter()
        .find(|state| state.execution_id == completed_id)
        .unwrap();
    assert_eq!(completed.plan_id, completed_plan);
    assert_eq!(completed.status, "completed");
    assert_eq!(completed.terminal_phase.as_deref(), Some("completed"));
    assert!(completed.terminal_verified);
    assert!(completed.verified_complete);
    let halted = states
        .iter()
        .find(|state| state.execution_id == halted_id)
        .unwrap();
    assert_eq!(halted.status, "halted");
    assert_eq!(halted.terminal_phase.as_deref(), Some("halted"));
    assert!(!halted.terminal_verified);
    assert!(!halted.verified_complete);
    let messages = engine.select_chat_messages(&session.id).unwrap();
    for (turn_id, expected_state) in [
        (completed_context.turn_id, "completed"),
        (halted_context.turn_id, "escalated"),
    ] {
        let user = messages
            .iter()
            .find(|message| {
                message.role == "user"
                    && message.metadata_json.as_deref().is_some_and(|metadata| {
                        metadata.contains(&format!("\"turnId\":\"{turn_id}\""))
                    })
            })
            .unwrap();
        assert!(user
            .metadata_json
            .as_deref()
            .unwrap()
            .contains(&format!("\"turnState\":\"{expected_state}\"")));
    }

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn recovery_state_projection_excludes_foreign_sessions_and_unrequested_executions() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_agent_execution_recovery_ownership_{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let owned_session = session_fixture(&engine, "owned-session");
    let foreign_session = session_fixture(&engine, "foreign-session");
    let (owned_id, _, _, _) = execution_fixture(&engine, "owned", &owned_session);
    let (foreign_id, _, _, _) = execution_fixture(&engine, "foreign", &foreign_session);

    let states = engine
        .select_agent_execution_recovery_states(
            &owned_session.id,
            &[owned_id.clone(), foreign_id.clone()],
        )
        .unwrap();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].execution_id, owned_id);
    assert!(engine
        .select_agent_execution_recovery_states(&owned_session.id, &[foreign_id])
        .unwrap()
        .is_empty());

    let foreign_workspace_id =
        crate::security::firewall::workspace_id_for_root("/tmp/foreign-recovery-state");
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE chat_sessions SET workspace_id=?1 WHERE id=?2",
            params![foreign_workspace_id, owned_session.id],
        )
        .unwrap();
    drop(connection);
    assert!(engine
        .select_agent_execution_recovery_states(&owned_session.id, &[owned_id])
        .unwrap()
        .is_empty());

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn sprint_304_recovery_state_preserves_exact_turn_generation_and_session_ownership() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_sprint_304_recovery_identity_{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let chat_a = session_fixture(&engine, "sprint-304-chat-a");
    let chat_b = session_fixture(&engine, "sprint-304-chat-b");
    let (execution_a, _, context_a, _) = execution_fixture(&engine, "sprint-304-a", &chat_a);
    let (execution_b, _, _, _) = execution_fixture(&engine, "sprint-304-b", &chat_b);

    let states = engine
        .select_agent_execution_recovery_states(&chat_a.id, &[execution_a.clone(), execution_b])
        .unwrap();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].execution_id, execution_a);
    assert_eq!(states[0].session_id, chat_a.id);
    assert_eq!(states[0].root_turn_id, context_a.root_turn_id);
    assert_eq!(states[0].failed_turn_id, context_a.turn_id);
    assert_eq!(states[0].generation_token, context_a.generation_token);

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn malformed_or_unverified_terminal_evidence_never_projects_verified_completion() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_agent_execution_recovery_evidence_{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = session_fixture(&engine, "evidence");
    let mut execution_ids = Vec::new();
    for (suffix, payload) in [
        ("malformed", "not-json"),
        ("unverified", r#"{"verified":false}"#),
        ("missing", r#"{"outputs":1}"#),
    ] {
        let (execution_id, plan_id, context, context_json) =
            execution_fixture(&engine, suffix, &session);
        engine
            .finalize_agent_execution(
                &execution_id,
                &plan_id,
                &context,
                &context_json,
                "completed",
                None,
                "info",
                "completed",
                "Execution claimed completion.",
                Some(payload),
            )
            .unwrap();
        execution_ids.push(execution_id);
    }

    let states = engine
        .select_agent_execution_recovery_states(&session.id, &execution_ids)
        .unwrap();
    assert_eq!(states.len(), 3);
    assert!(states.iter().all(|state| !state.verified_complete));
    assert!(states.iter().all(|state| !state.terminal_verified));

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}
