use super::*;

#[test]
fn recoverable_chat_delete_revokes_live_work_and_restores_only_inert_history() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_recoverable_chat_delete_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-delete-undo".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Keep this conversation".to_string()),
            dynamic_routing_override: None,
            workspace_id: Some(engine.workspace_id.clone()),
        })
        .unwrap();
    engine
        .insert_chat_message(
            &session.id,
            &session.agent_id,
            "user",
            "A user message that must survive Undo.",
        )
        .unwrap();
    engine
        .insert_chat_message(
            &session.id,
            &session.agent_id,
            "assistant",
            "An assistant reply that must survive Undo.",
        )
        .unwrap();
    let turn = ChatTurnPersistenceContext {
        turn_id: "turn-delete-undo".to_string(),
        generation_token: "generation-delete-undo".to_string(),
        session_id: session.id.clone(),
        agent_id: session.agent_id.clone(),
        provider_id: session.provider_id.clone(),
        model_id: session.model_id.clone(),
        parent_turn_id: None,
        root_turn_id: "turn-delete-undo".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.begin_chat_turn(&turn).unwrap();

    assert!(engine
        .stage_chat_session_deletion_by_id(&session.id)
        .unwrap());
    assert!(engine.select_chat_session_by_id(&session.id).is_err());
    assert!(engine.select_chat_messages(&session.id).unwrap().is_empty());

    let connection = engine.open_connection().unwrap();
    let active_session_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM chat_sessions WHERE id=?1",
            params![session.id],
            |row| row.get(0),
        )
        .unwrap();
    let archived_session_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM recoverable_chat_sessions WHERE id=?1",
            params![session.id],
            |row| row.get(0),
        )
        .unwrap();
    let archived_message_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM recoverable_chat_messages WHERE session_id=?1",
            params![session.id],
            |row| row.get(0),
        )
        .unwrap();
    let turn_status: String = connection
        .query_row(
            "SELECT status FROM chat_turns WHERE turn_id=?1",
            params![turn.turn_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active_session_count, 0);
    assert_eq!(archived_session_count, 1);
    assert_eq!(archived_message_count, 2);
    assert_eq!(turn_status, "cancelled");
    drop(connection);

    assert!(engine
        .undo_chat_session_deletion_by_id(&session.id)
        .unwrap());
    let restored = engine.select_chat_session_by_id(&session.id).unwrap();
    let restored_messages = engine.select_chat_messages(&session.id).unwrap();
    assert_eq!(restored.title, session.title);
    assert_eq!(restored.provider_id, session.provider_id);
    assert_eq!(restored.model_id, session.model_id);
    assert_eq!(restored_messages.len(), 2);
    assert_eq!(
        restored_messages[0].content,
        "A user message that must survive Undo."
    );
    assert_eq!(
        restored_messages[1].content,
        "An assistant reply that must survive Undo."
    );

    let connection = engine.open_connection().unwrap();
    let archived_rows: i64 = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM recoverable_chat_sessions WHERE id=?1) +
                (SELECT COUNT(*) FROM recoverable_chat_messages WHERE session_id=?1)",
            params![session.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived_rows, 0);
    drop(connection);
    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn expired_recoverable_chat_delete_is_purged_without_cumulative_rows() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_recoverable_chat_purge_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-delete-purge".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Delete permanently".to_string()),
            dynamic_routing_override: None,
            workspace_id: Some(engine.workspace_id.clone()),
        })
        .unwrap();
    engine
        .insert_chat_message(
            &session.id,
            &session.agent_id,
            "user",
            "This archived row must be purged.",
        )
        .unwrap();
    assert!(engine
        .stage_chat_session_deletion_by_id(&session.id)
        .unwrap());
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE recoverable_chat_sessions SET purge_after_ms=?2 WHERE id=?1",
            params![session.id, unix_time_ms() - 1],
        )
        .unwrap();
    drop(connection);

    engine.select_chat_sessions().unwrap();
    let connection = engine.open_connection().unwrap();
    let archived_rows: i64 = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM recoverable_chat_sessions) +
                (SELECT COUNT(*) FROM recoverable_chat_messages)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived_rows, 0);
    drop(connection);
    assert!(!engine
        .undo_chat_session_deletion_by_id(&session.id)
        .unwrap());
    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}
