use super::*;
use std::sync::Barrier;

#[test]
fn background_auto_turn_can_claim_a_derived_response() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_auto_turn_response_claim_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-auto-turn".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-auto-turn".to_string(),
            title: Some("Background completion".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let parent = ChatTurnPersistenceContext {
        turn_id: "turn-auto-turn-parent".to_string(),
        generation_token: "generation-auto-turn-parent".to_string(),
        session_id: session.id,
        agent_id: session.agent_id,
        provider_id: "local_model".to_string(),
        model_id: "gemma-auto-turn".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-auto-turn-parent".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.ensure_chat_turn_for_native_action(&parent).unwrap();
    engine.begin_or_claim_chat_turn_response(&parent).unwrap();

    let completion = ChatTurnPersistenceContext {
        turn_id: "turn-auto-turn-completion".to_string(),
        generation_token: "generation-auto-turn-completion".to_string(),
        session_id: parent.session_id.clone(),
        agent_id: parent.agent_id.clone(),
        provider_id: parent.provider_id.clone(),
        model_id: parent.model_id.clone(),
        parent_turn_id: Some(parent.turn_id.clone()),
        root_turn_id: parent.root_turn_id.clone(),
        turn_kind: AUTO_TURN_KIND.to_string(),
    };

    engine
        .begin_or_claim_chat_turn_response(&completion)
        .unwrap();
    let stored = engine
        .select_chat_turn_context(&completion.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.turn_kind, AUTO_TURN_KIND);
    assert_eq!(stored.parent_turn_id, Some(parent.turn_id));
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn background_auto_turn_requires_a_parent() {
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-orphan-auto-turn".to_string(),
        generation_token: "generation-orphan-auto-turn".to_string(),
        session_id: "session-orphan-auto-turn".to_string(),
        agent_id: "agent-orphan-auto-turn".to_string(),
        provider_id: "local_model".to_string(),
        model_id: "gemma-auto-turn".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-root".to_string(),
        turn_kind: AUTO_TURN_KIND.to_string(),
    };

    let error = validate_chat_turn_context_fields(&context).unwrap_err();
    assert!(error.to_string().contains("require a parent turn"));
}

#[test]
fn response_claim_migration_marks_every_legacy_turn_claimed() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "
            CREATE TABLE chat_turns (
                turn_id TEXT PRIMARY KEY,
                created_at_ms INTEGER NOT NULL
            );
            INSERT INTO chat_turns (turn_id, created_at_ms)
            VALUES ('legacy-running-turn', 42), ('legacy-completed-turn', 84);
            ",
        )
        .unwrap();

    connection
        .execute_batch(CHAT_TURN_RESPONSE_CLAIM_MIGRATION)
        .unwrap();

    assert!(column_exists(&connection, "chat_turns", "response_claimed_at_ms").unwrap());
    let claims = connection
        .prepare("SELECT response_claimed_at_ms FROM chat_turns ORDER BY created_at_ms ASC")
        .unwrap()
        .query_map([], |row| row.get::<_, Option<i64>>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(claims, vec![Some(42), Some(84)]);
}

#[test]
fn stale_migration_preflight_rechecks_after_waiting_for_writer() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_migration_writer_race_{}-{}",
        std::process::id(),
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("state.sqlite");
    let engine = PersistenceEngine::initialize_at(db_path).unwrap();
    let connection = engine.open_connection().unwrap();
    connection
        .execute_batch(
            "
            BEGIN IMMEDIATE;
            DELETE FROM schema_migration_ledger WHERE sequence >= 25;
            ALTER TABLE chat_turns DROP COLUMN response_claimed_at_ms;
            COMMIT;
            ",
        )
        .unwrap();
    verify_migration_ledger(&connection).unwrap();
    verify_schema_invariants(&connection, 24).unwrap();
    drop(connection);

    let key = get_database_key().unwrap();
    let mut first_connection = engine.open_connection_with_key(&key).unwrap();
    let mut second_connection = engine.open_connection_with_key(&key).unwrap();
    let first_engine = engine.clone();
    let second_engine = engine.clone();
    let (first_inside_tx, first_inside_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let apply_count = Arc::new(AtomicUsize::new(0));

    let first_count = Arc::clone(&apply_count);
    let first = std::thread::spawn(move || {
        first_engine.apply_migration(&mut first_connection, &key, MIGRATIONS[24], |transaction| {
            first_inside_tx.send(()).unwrap();
            release_first_rx.recv().unwrap();
            first_count.fetch_add(1, Ordering::SeqCst);
            transaction.execute_batch(CHAT_TURN_RESPONSE_CLAIM_MIGRATION)
        })
    });

    first_inside_rx.recv().unwrap();
    let second_count = Arc::clone(&apply_count);
    let second = std::thread::spawn(move || {
        second_engine.apply_migration(
            &mut second_connection,
            &get_database_key().unwrap(),
            MIGRATIONS[24],
            |transaction| {
                second_count.fetch_add(1, Ordering::SeqCst);
                transaction.execute_batch(CHAT_TURN_RESPONSE_CLAIM_MIGRATION)
            },
        )
    });

    // The first writer deliberately holds the immediate transaction long
    // enough for the second connection to complete its stale fast-path
    // read and wait for the writer lock.
    std::thread::sleep(Duration::from_millis(150));
    release_first_tx.send(()).unwrap();

    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
    assert_eq!(apply_count.load(Ordering::SeqCst), 1);

    let verified = engine.open_connection().unwrap();
    verify_migration_ledger(&verified).unwrap();
    verify_schema_invariants(&verified, 25).unwrap();
    let completed: i64 = verified
        .query_row(
            "SELECT COUNT(*) FROM schema_migration_ledger WHERE sequence = 25 AND state = 'completed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(completed, 1);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn native_action_turn_response_can_be_claimed_exactly_once() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_native_turn_handoff_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-handoff".to_string(),
            provider_id: "provider-handoff".to_string(),
            model_id: "model-handoff".to_string(),
            title: Some("Native handoff".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-handoff".to_string(),
        generation_token: "generation-handoff".to_string(),
        session_id: session.id,
        agent_id: session.agent_id,
        provider_id: session.provider_id,
        model_id: session.model_id,
        parent_turn_id: None,
        root_turn_id: "turn-handoff".to_string(),
        turn_kind: "root".to_string(),
    };

    engine.ensure_chat_turn_for_native_action(&context).unwrap();
    let unclaimed: Option<i64> = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT response_claimed_at_ms FROM chat_turns WHERE turn_id = ?1",
            params![context.turn_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unclaimed, None);

    engine.begin_or_claim_chat_turn_response(&context).unwrap();
    let claimed: Option<i64> = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT response_claimed_at_ms FROM chat_turns WHERE turn_id = ?1",
            params![context.turn_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(claimed.is_some());

    let replay_error = engine
        .begin_or_claim_chat_turn_response(&context)
        .unwrap_err();
    assert!(is_chat_turn_response_claim_conflict(&replay_error));
    assert!(engine.ensure_chat_turn_for_native_action(&context).is_err());
    engine.finish_chat_turn(&context, "completed").unwrap();
    engine.ensure_chat_turn_for_native_action(&context).unwrap();

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn concurrent_chat_response_claim_has_exactly_one_winner() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_response_claim_race_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-claim-race".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-race".to_string(),
            title: Some("Response claim race".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-claim-race".to_string(),
        generation_token: "generation-claim-race".to_string(),
        session_id: session.id,
        agent_id: session.agent_id,
        provider_id: session.provider_id,
        model_id: session.model_id,
        parent_turn_id: None,
        root_turn_id: "turn-claim-race".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.ensure_chat_turn_for_native_action(&context).unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let workers = (0..2)
        .map(|_| {
            let worker_engine = engine.clone();
            let worker_context = context.clone();
            let worker_barrier = barrier.clone();
            std::thread::spawn(move || {
                worker_barrier.wait();
                worker_engine.begin_or_claim_chat_turn_response(&worker_context)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let conflicts = results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .filter(|error| is_chat_turn_response_claim_conflict(error))
        .count();
    assert_eq!(conflicts, 1);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn native_dynamic_turn_claim_atomically_binds_concrete_route_and_rejects_token_rebinding() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_dynamic_turn_claim_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-dynamic-claim".to_string(),
            provider_id: "configured-cloud".to_string(),
            model_id: "configured-model".to_string(),
            title: Some("Dynamic response claim".to_string()),
            dynamic_routing_override: Some(true),
            workspace_id: None,
        })
        .unwrap();
    let prebound = ChatTurnPersistenceContext {
        turn_id: "turn-dynamic-claim".to_string(),
        generation_token: "generation-dynamic-claim".to_string(),
        session_id: session.id,
        agent_id: session.agent_id,
        provider_id: "dynamic".to_string(),
        model_id: "dynamic".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-dynamic-claim".to_string(),
        turn_kind: "root".to_string(),
    };
    engine
        .ensure_chat_turn_for_native_action(&prebound)
        .unwrap();

    let mut resolved = prebound.clone();
    resolved.provider_id = "local_model".to_string();
    resolved.model_id = "gemma-3-12b".to_string();
    engine.begin_or_claim_chat_turn_response(&resolved).unwrap();

    let stored = engine
        .select_chat_turn_context(&resolved.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.provider_id, resolved.provider_id);
    assert_eq!(stored.model_id, resolved.model_id);
    let claimed: Option<i64> = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT response_claimed_at_ms FROM chat_turns WHERE turn_id = ?1",
            params![resolved.turn_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(claimed.is_some());

    let mut wrong_claimed_route = resolved.clone();
    wrong_claimed_route.provider_id = "different-provider".to_string();
    let wrong_route_error = engine
        .begin_or_claim_chat_turn_response(&wrong_claimed_route)
        .unwrap_err();
    assert!(response_claim::is_chat_turn_response_claim_mismatch(
        &wrong_route_error
    ));
    assert!(!is_chat_turn_response_claim_conflict(&wrong_route_error));

    // The renderer keeps its immutable dynamic placeholder while the
    // backend records the selected concrete route. That original context
    // remains valid for a follow-on native tool, but no concrete route
    // mismatch is accepted.
    assert!(engine
        .ensure_chat_turn_for_native_action(&prebound)
        .is_err());
    engine.finish_chat_turn(&resolved, "completed").unwrap();
    engine
        .ensure_chat_turn_for_native_action(&prebound)
        .unwrap();
    let canonical_execution_origin = engine
        .canonical_agent_execution_origin_context(&prebound)
        .unwrap();
    assert_eq!(canonical_execution_origin.turn_id, resolved.turn_id);
    assert_eq!(canonical_execution_origin.provider_id, resolved.provider_id);
    assert_eq!(canonical_execution_origin.model_id, resolved.model_id);
    engine
        .begin_agent_execution(
            "execution-dynamic-claim",
            "plan-dynamic-claim",
            &canonical_execution_origin,
            r#"{"route":"canonical"}"#,
        )
        .unwrap();
    engine
        .validate_agent_execution_origin(
            "execution-dynamic-claim",
            "plan-dynamic-claim",
            &canonical_execution_origin,
            r#"{"route":"canonical"}"#,
        )
        .unwrap();
    assert!(engine
        .begin_agent_execution(
            "execution-dynamic-claim-duplicate",
            "plan-dynamic-claim",
            &canonical_execution_origin,
            r#"{"route":"canonical"}"#,
        )
        .is_err());
    let mut mismatched_concrete = resolved.clone();
    mismatched_concrete.provider_id = "different-provider".to_string();
    assert!(engine
        .ensure_chat_turn_for_native_action(&mismatched_concrete)
        .is_err());

    let mut rebound = resolved.clone();
    rebound.turn_id = "turn-token-rebind".to_string();
    rebound.root_turn_id = rebound.turn_id.clone();
    let rebound_error = engine
        .begin_or_claim_chat_turn_response(&rebound)
        .unwrap_err();
    assert!(response_claim::is_chat_turn_response_claim_mismatch(
        &rebound_error
    ));
    assert!(!is_chat_turn_response_claim_conflict(&rebound_error));
    assert!(engine
        .select_chat_turn_context(&rebound.turn_id)
        .unwrap()
        .is_none());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn released_dynamic_turn_claim_can_bind_a_new_concrete_route_before_dispatch() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_dynamic_turn_retry_route_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-dynamic-retry-route".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            title: Some("Dynamic retry route".to_string()),
            dynamic_routing_override: Some(true),
            workspace_id: None,
        })
        .unwrap();
    let prebound = ChatTurnPersistenceContext {
        turn_id: "turn-dynamic-retry-route".to_string(),
        generation_token: "generation-dynamic-retry-route".to_string(),
        session_id: session.id,
        agent_id: session.agent_id,
        provider_id: "dynamic".to_string(),
        model_id: "dynamic".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-dynamic-retry-route".to_string(),
        turn_kind: "root".to_string(),
    };
    engine
        .ensure_chat_turn_for_native_action(&prebound)
        .unwrap();

    let mut first_route = prebound.clone();
    first_route.provider_id = "first-cloud-provider".to_string();
    first_route.model_id = "first-cloud-model".to_string();
    engine
        .begin_or_claim_chat_turn_response(&first_route)
        .unwrap();
    assert!(engine
        .release_chat_turn_response_claim(&first_route)
        .unwrap());

    let mut retry_route = prebound.clone();
    retry_route.provider_id = "second-cloud-provider".to_string();
    retry_route.model_id = "second-cloud-model".to_string();
    engine
        .begin_or_claim_chat_turn_response(&retry_route)
        .unwrap();
    let stored = engine
        .select_chat_turn_context(&retry_route.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.provider_id, retry_route.provider_id);
    assert_eq!(stored.model_id, retry_route.model_id);

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn static_session_without_auto_route_rejects_dynamic_rebinding_as_mismatch() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_static_turn_claim_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-static-claim".to_string(),
            provider_id: "configured-cloud".to_string(),
            model_id: "configured-model".to_string(),
            title: Some("Static response claim".to_string()),
            dynamic_routing_override: Some(false),
            workspace_id: None,
        })
        .unwrap();
    let prebound = ChatTurnPersistenceContext {
        turn_id: "turn-static-claim".to_string(),
        generation_token: "generation-static-claim".to_string(),
        session_id: session.id,
        agent_id: session.agent_id,
        provider_id: "dynamic".to_string(),
        model_id: "dynamic".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-static-claim".to_string(),
        turn_kind: "root".to_string(),
    };
    engine
        .ensure_chat_turn_for_native_action(&prebound)
        .unwrap();

    let mut resolved = prebound.clone();
    resolved.provider_id = "local_model".to_string();
    resolved.model_id = "gemma-3-12b".to_string();
    let error = engine
        .begin_or_claim_chat_turn_response(&resolved)
        .unwrap_err();
    assert!(response_claim::is_chat_turn_response_claim_mismatch(&error));
    assert!(!is_chat_turn_response_claim_conflict(&error));

    let stored = engine
        .select_chat_turn_context(&prebound.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.provider_id, "dynamic");
    assert_eq!(stored.model_id, "dynamic");
    let claim: Option<i64> = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT response_claimed_at_ms FROM chat_turns WHERE turn_id = ?1",
            params![prebound.turn_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(claim.is_none());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn completed_claimed_turn_replays_as_reconcilable_conflict() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_completed_claim_replay_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-completed-replay".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-replay".to_string(),
            title: Some("Completed replay".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-completed-replay".to_string(),
        generation_token: "generation-completed-replay".to_string(),
        session_id: session.id,
        agent_id: session.agent_id,
        provider_id: session.provider_id,
        model_id: session.model_id,
        parent_turn_id: None,
        root_turn_id: "turn-completed-replay".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.ensure_chat_turn_for_native_action(&context).unwrap();
    engine.begin_or_claim_chat_turn_response(&context).unwrap();
    engine
        .complete_claimed_chat_turn(CompleteClaimedChatTurnRequest {
            context: context.clone(),
            role: "assistant".to_string(),
            content: "The durable answer already exists.".to_string(),
            message_provider_id: context.provider_id.clone(),
            message_model_id: context.model_id.clone(),
            metadata: json!({"turnId": context.turn_id}),
            session_title: None,
            session_provider_id: context.provider_id.clone(),
            session_model_id: context.model_id.clone(),
            status: "completed".to_string(),
        })
        .unwrap();

    let replay = engine
        .begin_or_claim_chat_turn_response(&context)
        .expect_err("a completed immutable replay must reconcile its terminal receipt");
    assert!(is_chat_turn_response_claim_conflict(&replay));
    assert!(!response_claim::is_chat_turn_response_claim_mismatch(
        &replay
    ));
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn dynamic_child_may_follow_only_an_auto_route_parents_concrete_claim() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_dynamic_child_claim_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-dynamic-child".to_string(),
            provider_id: "configured-provider".to_string(),
            model_id: "configured-model".to_string(),
            title: Some("Dynamic child".to_string()),
            dynamic_routing_override: Some(true),
            workspace_id: None,
        })
        .unwrap();
    let parent = ChatTurnPersistenceContext {
        turn_id: "turn-dynamic-parent".to_string(),
        generation_token: "generation-dynamic-parent".to_string(),
        session_id: session.id.clone(),
        agent_id: session.agent_id.clone(),
        provider_id: "dynamic".to_string(),
        model_id: "dynamic".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-dynamic-parent".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.ensure_chat_turn_for_native_action(&parent).unwrap();
    let mut claimed_parent = parent.clone();
    claimed_parent.provider_id = "local_model".to_string();
    claimed_parent.model_id = "gemma-child".to_string();
    engine
        .begin_or_claim_chat_turn_response(&claimed_parent)
        .unwrap();

    let child = ChatTurnPersistenceContext {
        turn_id: "turn-dynamic-child".to_string(),
        generation_token: "generation-dynamic-child".to_string(),
        session_id: parent.session_id.clone(),
        agent_id: parent.agent_id.clone(),
        provider_id: "dynamic".to_string(),
        model_id: "dynamic".to_string(),
        parent_turn_id: Some(parent.turn_id.clone()),
        root_turn_id: parent.root_turn_id.clone(),
        turn_kind: "steer".to_string(),
    };
    engine.begin_chat_turn(&child).unwrap();

    let mut wrong_concrete_child = child.clone();
    wrong_concrete_child.turn_id = "turn-wrong-concrete-child".to_string();
    wrong_concrete_child.generation_token = "generation-wrong-concrete-child".to_string();
    wrong_concrete_child.provider_id = "different-provider".to_string();
    wrong_concrete_child.model_id = "different-model".to_string();
    assert!(engine.begin_chat_turn(&wrong_concrete_child).is_err());
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn dynamic_child_alias_is_denied_without_the_session_override() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_dynamic_child_without_override_{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-no-dynamic-child".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            title: Some("No dynamic child override".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let parent = ChatTurnPersistenceContext {
        turn_id: "turn-no-override-parent".to_string(),
        generation_token: "generation-no-override-parent".to_string(),
        session_id: session.id,
        agent_id: session.agent_id,
        provider_id: "dynamic".to_string(),
        model_id: "dynamic".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-no-override-parent".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.ensure_chat_turn_for_native_action(&parent).unwrap();
    let mut claimed_parent = parent.clone();
    claimed_parent.provider_id = "local_model".to_string();
    claimed_parent.model_id = "gemma-no-override".to_string();
    engine
        .begin_or_claim_chat_turn_response(&claimed_parent)
        .unwrap();
    let child = ChatTurnPersistenceContext {
        turn_id: "turn-no-override-child".to_string(),
        generation_token: "generation-no-override-child".to_string(),
        session_id: parent.session_id.clone(),
        agent_id: parent.agent_id.clone(),
        provider_id: "dynamic".to_string(),
        model_id: "dynamic".to_string(),
        parent_turn_id: Some(parent.turn_id.clone()),
        root_turn_id: parent.root_turn_id.clone(),
        turn_kind: "steer".to_string(),
    };

    assert!(engine.begin_chat_turn(&child).is_err());
    let _ = std::fs::remove_dir_all(temp_dir);
}
