use super::*;

fn persist_claimed_assistant(
    provider_id: &str,
    model_id: &str,
    content: &str,
) -> (PathBuf, String) {
    let root = std::env::temp_dir().join(format!(
        "oomu-assistant-canonical-{}-{}",
        provider_id,
        unix_time_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let state_path = root.join("state.sqlite");
    let engine = PersistenceEngine::initialize_at(state_path).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: format!("agent-{provider_id}"),
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            title: Some("Canonical assistant persistence".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: format!("turn-{provider_id}"),
        generation_token: format!("generation-{provider_id}"),
        session_id: session.id.clone(),
        agent_id: session.agent_id.clone(),
        provider_id: session.provider_id.clone(),
        model_id: session.model_id.clone(),
        parent_turn_id: None,
        root_turn_id: format!("turn-{provider_id}"),
        turn_kind: "root".to_string(),
    };
    engine.ensure_chat_turn_for_native_action(&context).unwrap();
    engine.begin_or_claim_chat_turn_response(&context).unwrap();
    engine
        .complete_claimed_chat_turn(CompleteClaimedChatTurnRequest {
            context: context.clone(),
            role: "assistant".to_string(),
            content: content.to_string(),
            message_provider_id: context.provider_id.clone(),
            message_model_id: context.model_id.clone(),
            metadata: json!({"turnId": context.turn_id}),
            session_title: None,
            session_provider_id: context.provider_id,
            session_model_id: context.model_id,
            status: "completed".to_string(),
        })
        .unwrap();
    drop(engine);
    (root, session.id)
}

#[test]
fn local_native_turn_persists_only_canonical_assistant_content() {
    let content = concat!(
        "Local answer.\n",
        "<tool_call>{\"name\":\"read_file\"}</tool_call>\n",
        "Still visible.</tool_result>\n",
        "```xml\n<tool_call>{\"literal\":true}</tool_call>\n```"
    );
    let (root, session_id) = persist_claimed_assistant("local_model", "gemma-4-e4b", content);
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let messages = engine.select_chat_messages(&session_id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].content,
        concat!(
            "Local answer.\n\nStill visible.\n",
            "```xml\n<tool_call>{\"literal\":true}</tool_call>\n```"
        )
    );
    drop(engine);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cloud_turn_hydrates_canonically_after_database_relaunch() {
    let content = concat!(
        "Cloud answer.\n",
        "<execution_receipt>{\"secret\":true}</execution_receipt>\n",
        "Durable result.\n",
        "<function_call>{\"name\":\"internal_only\"}"
    );
    let (root, session_id) = persist_claimed_assistant("anthropic", "claude-fable-5", content);

    let relaunched = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let hydrated = relaunched.select_chat_messages(&session_id).unwrap();
    assert_eq!(hydrated.len(), 1);
    assert_eq!(hydrated[0].content, "Cloud answer.\n\nDurable result.");
    assert!(!hydrated[0].content.contains("secret"));
    assert!(!hydrated[0].content.contains("internal_only"));
    drop(relaunched);
    std::fs::remove_dir_all(root).unwrap();
}
