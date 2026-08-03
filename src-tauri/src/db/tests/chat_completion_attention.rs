use super::*;

#[test]
fn completion_attention_is_terminal_turn_bound_and_exactly_once() {
    let temp_dir = std::env::temp_dir().join(format!("oomu-chat-attention-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-attention".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Attention".to_string()),
            dynamic_routing_override: None,
            workspace_id: Some(engine.workspace_id.clone()),
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-attention".to_string(),
        generation_token: "generation-attention".to_string(),
        session_id: session.id.clone(),
        agent_id: "agent-attention".to_string(),
        provider_id: "local_model".to_string(),
        model_id: "gemma-test".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-attention".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.begin_chat_turn(&context).unwrap();
    assert!(engine
        .record_chat_completion_attention(&session.id, &context.turn_id)
        .is_err());
    engine.finish_chat_turn(&context, "completed").unwrap();
    assert_eq!(
        engine
            .record_chat_completion_attention(&session.id, &context.turn_id)
            .unwrap(),
        (1, true),
    );
    assert_eq!(
        engine
            .record_chat_completion_attention(&session.id, &context.turn_id)
            .unwrap(),
        (1, false),
    );
    assert_eq!(
        engine
            .set_chat_session_unread_completion(&session.id, false)
            .unwrap(),
        0,
    );
    assert_eq!(
        engine
            .record_chat_completion_attention(&session.id, &context.turn_id)
            .unwrap(),
        (0, false),
    );
    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}
