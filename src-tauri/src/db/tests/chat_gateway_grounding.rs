use super::*;

#[test]
fn gateway_message_receipts_prevent_replay_and_release_failed_delivery() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_gateway_dedupe_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    assert!(engine
        .claim_gateway_message("slack", "slack-message-1", unix_time_ms())
        .unwrap());
    assert!(!engine
        .claim_gateway_message("slack", "slack-message-1", unix_time_ms())
        .unwrap());
    engine
        .finish_gateway_message("slack", "slack-message-1", false)
        .unwrap();
    assert!(engine
        .claim_gateway_message("slack", "slack-message-1", unix_time_ms())
        .unwrap());
    engine
        .finish_gateway_message("slack", "slack-message-1", true)
        .unwrap();
    assert!(!engine
        .claim_gateway_message("slack", "slack-message-1", unix_time_ms())
        .unwrap());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn chat_history_restores_only_the_latest_encrypted_public_grounding_context() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_public_grounding_history_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    for (turn, fact) in [
        ("first", "Older approved public fact."),
        ("second", "Latest approved public fact."),
    ] {
        let context = format!("Local Web Search Context\nQuery: {turn}\nEngine: test\n\n{fact}");
        let metadata = json!({
            "publicGroundingAttachments": [{
                "name": "local_web_search.md",
                "mime_type": "text/markdown",
                "byte_count": context.len(),
                "data_base64": null,
                "text": context,
                "approved_file_receipt": null
            }]
        });
        engine
            .insert_chat_message_with_metadata(
                "session-grounding",
                "agent-a",
                "user",
                &format!("{turn} search\n\nAttached files:\n- local_web_search.md"),
                Some("gemini"),
                Some("gemini-test"),
                Some(&metadata),
            )
            .unwrap();
        engine
            .insert_chat_message(
                "session-grounding",
                "agent-a",
                "assistant",
                &format!("Finished the {turn} search."),
            )
            .unwrap();
    }

    let history = engine.get_chat_history("session-grounding", 20).unwrap();
    assert_eq!(history.len(), 4);
    assert!(history[0].attachments.is_empty());
    assert!(history[1].attachments.is_empty());
    assert_eq!(history[2].attachments.len(), 1);
    assert!(history[2].attachments[0]
        .text
        .as_deref()
        .is_some_and(|text| text.contains("Latest approved public fact.")));
    assert!(history[3].attachments.is_empty());

    drop(engine);
    let _ = std::fs::remove_dir_all(temp_dir);
}
