use super::*;

#[test]
fn oauth_failure_transition_is_atomic_and_durable() {
    let root = std::env::temp_dir().join(format!(
        "oomu-oauth-failure-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let connector_id = create_account(&engine, "google_workspace", 1).unwrap();
    record_oauth_attempt(
        &engine,
        "oauth_test_attempt",
        &connector_id,
        "state-hash",
        "http://127.0.0.1:4000/oauth/callback",
        unix_time_ms_i64() + 60_000,
    )
    .unwrap();

    let pending = connection_status(&engine, &connector_id).unwrap();
    assert_eq!(pending.connection_state, "configured");
    assert_eq!(pending.last_probe_code.as_deref(), Some("oauth_started"));

    fail_oauth(
        &engine,
        "oauth_test_attempt",
        "google_token_invalid_request",
        false,
    )
    .unwrap();

    let connection = engine.open_connection().unwrap();
    let outcome: String = connection
        .query_row(
            "SELECT outcome FROM connector_oauth_attempts WHERE attempt_id='oauth_test_attempt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let (state, code): (String, String) = connection
        .query_row(
            "SELECT connection_state,last_probe_code FROM connector_accounts WHERE connector_id=?1",
            params![connector_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(outcome, "failed");
    assert_eq!(state, "disconnected");
    assert_eq!(code, "google_token_invalid_request");
    let failed = connection_status(&engine, &connector_id).unwrap();
    assert_eq!(failed.connection_state, "disconnected");
    assert_eq!(
        failed.last_probe_code.as_deref(),
        Some("google_token_invalid_request")
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn oauth_attempt_recovers_a_new_shell_retired_by_account_polling() {
    let root = std::env::temp_dir().join(format!(
        "oomu-oauth-account-poll-race-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let connector_id = create_account(&engine, "slack", 1).unwrap();

    let before_poll = connection_status(&engine, &connector_id).unwrap();
    let polled_accounts = list_accounts(&engine).unwrap();
    let after_poll = connection_status(&engine, &connector_id).unwrap();
    assert!(
        polled_accounts
            .iter()
            .all(|account| account.connector_id != connector_id),
        "before={before_poll:?} after={after_poll:?}"
    );
    assert_eq!(
        connection_status(&engine, &connector_id)
            .unwrap()
            .connection_state,
        "disconnected"
    );

    record_oauth_attempt(
        &engine,
        "oauth_poll_race",
        &connector_id,
        "state-hash",
        "http://localhost:53682/oauth/callback",
        unix_time_ms_i64() + 60_000,
    )
    .unwrap();

    let recovered = connection_status(&engine, &connector_id).unwrap();
    assert_eq!(recovered.connection_state, "configured");
    assert_eq!(recovered.last_probe_code.as_deref(), Some("oauth_started"));
    assert!(list_accounts(&engine)
        .unwrap()
        .iter()
        .any(|account| account.connector_id == connector_id));

    let _ = std::fs::remove_dir_all(root);
}
