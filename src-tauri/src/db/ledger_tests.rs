use super::*;

#[test]
fn dynamic_local_chat_metadata_is_an_exact_ledger_fallback_without_audit() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_dynamic_local_ledger_{}",
        crate::foundation::clock::unix_time_ns_u128()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create ledger fixture");
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite"))
        .expect("initialize persistence");
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-local".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            title: Some("Dynamic local".to_string()),
            dynamic_routing_override: Some(true),
            workspace_id: None,
        })
        .expect("create dynamic session");

    engine
        .insert_chat_message_with_metadata(
            &session.id,
            "agent-local",
            "assistant",
            "Exact local answer.",
            Some("local_model"),
            Some("gemma-4-12B-it-qat-q4_0-gguf"),
            Some(&json!({
                "eventKind": "dynamic_routing",
                "executingProviderId": "local_model",
                "executingModelId": "gemma-4-12B-it-qat-q4_0-gguf",
                "promptTokens": 1200,
                "completionTokens": 300,
            })),
        )
        .expect("persist dynamic local assistant row");

    let stats = engine
        .sovereign_ledger_stats(None)
        .expect("read ledger stats without a local audit row");
    assert_eq!(stats.total_local_turns, 1);
    assert_eq!(stats.total_cloud_turns, 0);
    assert_eq!(stats.protected_input_tokens, 1200);
    assert_eq!(stats.protected_output_tokens, 300);
    assert!((stats.estimated_api_savings - 0.003).abs() < f64::EPSILON);

    let _ = std::fs::remove_dir_all(temp_dir);
}
