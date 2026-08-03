use super::{native_drop::claim_latest_dropped_local_context_with_store, *};

fn grant_request(
    result: &PickerGrantResult,
    session_id: &str,
    turn_id: &str,
) -> LocalContextRequest {
    LocalContextRequest {
        grant_id: result.grant_id.clone().expect("issued grant"),
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
    }
}

#[test]
fn latest_drop_claim_is_native_ordered_one_use_and_turn_bound() {
    let root = std::env::temp_dir().join(format!(
        "oomu-local-context-latest-drop-{}-{}",
        std::process::id(),
        unix_time_ms()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let first = root.join("first.txt");
    let latest = root.join("latest.txt");
    fs::write(&first, "first content").unwrap();
    fs::write(&latest, "latest content").unwrap();
    let store = LocalContextGrantStore::default();
    register_dropped_local_context(&store, &[first]).unwrap();
    register_dropped_local_context(&store, std::slice::from_ref(&latest)).unwrap();

    let issued = claim_latest_dropped_local_context_with_store(
        &store,
        ClaimLatestDroppedLocalContextRequest {
            session_id: "session-latest".to_string(),
            turn_id: "turn-latest".to_string(),
        },
    )
    .unwrap();
    assert_eq!(issued.results[0].name, "latest.txt");
    let older = claim_latest_dropped_local_context_with_store(
        &store,
        ClaimLatestDroppedLocalContextRequest {
            session_id: "session-latest".to_string(),
            turn_id: "turn-latest".to_string(),
        },
    )
    .unwrap();
    assert_eq!(older.results[0].name, "first.txt");
    let response = read_local_context_with_store(
        &store,
        grant_request(&issued.results[0], "session-latest", "turn-latest"),
    )
    .unwrap();
    assert_eq!(response.text, "latest content");
    assert_eq!(
        read_local_context_with_store(
            &store,
            grant_request(&issued.results[0], "session-latest", "turn-latest"),
        )
        .unwrap_err(),
        "local_context_grant_invalid_or_expired"
    );

    register_dropped_local_context(&store, std::slice::from_ref(&latest)).unwrap();
    let scoped = claim_latest_dropped_local_context_with_store(
        &store,
        ClaimLatestDroppedLocalContextRequest {
            session_id: "session-bound".to_string(),
            turn_id: "turn-bound".to_string(),
        },
    )
    .unwrap();
    assert_eq!(
        read_local_context_with_store(
            &store,
            grant_request(&scoped.results[0], "wrong-session", "turn-bound"),
        )
        .unwrap_err(),
        "local_context_grant_scope_mismatch"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn latest_drop_claim_rejects_expiry_and_renderer_path_fields() {
    let store = LocalContextGrantStore::default();
    store.state.lock().unwrap().pending_drops.insert(
        "expired".to_string(),
        PendingLocalContextDrop {
            paths: vec![PathBuf::from("/tmp/not-opened")],
            expires_at_ms: unix_time_ms().saturating_sub(1),
            sequence: 1,
        },
    );
    assert_eq!(
        claim_latest_dropped_local_context_with_store(
            &store,
            ClaimLatestDroppedLocalContextRequest {
                session_id: "session-expired".to_string(),
                turn_id: "turn-expired".to_string(),
            },
        )
        .unwrap_err(),
        "local_context_drop_invalid_or_expired"
    );
    for forbidden in ["path", "dropId"] {
        let mut request = serde_json::json!({"sessionId": "session-a", "turnId": "turn-a"});
        request[forbidden] = serde_json::json!("renderer-controlled");
        assert!(serde_json::from_value::<ClaimLatestDroppedLocalContextRequest>(request).is_err());
    }
}
