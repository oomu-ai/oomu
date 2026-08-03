use super::*;

#[tokio::test]
async fn ordinary_chat_execution_creates_a_real_file_with_project_bound_provenance() {
    let root = std::env::temp_dir().join(format!(
        "oomu-chat-file-execution-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ms_i64()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let engine = crate::db::PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute_batch(
            "CREATE TABLE taskflows (flow_id TEXT PRIMARY KEY, parent_session_id TEXT NOT NULL, directive TEXT NOT NULL, status TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);
             CREATE TABLE taskflow_steps (flow_id TEXT NOT NULL, status TEXT NOT NULL);",
        )
        .unwrap();
    crate::tasks::register_runtime_bridge().unwrap();
    let session = engine
        .ensure_chat_session(crate::db::CreateChatSessionRequest {
            agent_id: "agent-chat-file".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Create a file".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let turn = crate::db::ChatTurnPersistenceContext {
        turn_id: "turn-chat-file".to_string(),
        generation_token: "generation-chat-file".to_string(),
        session_id: session.id,
        agent_id: session.agent_id,
        provider_id: session.provider_id,
        model_id: session.model_id,
        parent_turn_id: None,
        root_turn_id: "turn-chat-file".to_string(),
        turn_kind: "root".to_string(),
    };
    engine.begin_chat_turn(&turn).unwrap();
    engine
        .begin_agent_execution("execution-chat-file", "plan-chat-file", &turn, "{}")
        .unwrap();

    let destination = root.join("Hello World.txt");
    let identity = crate::sovereign_identity::SovereignIdentity::initialize_ephemeral();
    let result = execute_registration(
        TaskToolExecutionContext {
            persistence: &engine,
            identity: &identity,
            app: None,
            execution_id: Some("execution-chat-file"),
            plan_id: None,
            objective: None,
            session_id: None,
            model_route: None,
        },
        json!({"file":{
            "title":"Hello World",
            "content":"Hello World",
            "locale":"en-US",
            "format":"txt",
            "destinationPath":destination
        }}),
    )
    .await
    .unwrap();

    assert!(result.verified);
    assert_eq!(
        std::fs::read_to_string(&destination).unwrap(),
        "Hello World"
    );

    let pdf_destination = root.join("Hello World.pdf");
    let pdf_result = execute_registration(
        TaskToolExecutionContext {
            persistence: &engine,
            identity: &identity,
            app: None,
            execution_id: Some("execution-chat-file"),
            plan_id: None,
            objective: None,
            session_id: None,
            model_route: None,
        },
        json!({"file":{
            "title":"Hello World",
            "content":"Hello World",
            "locale":"en-US",
            "format":"pdf",
            "destinationPath":pdf_destination
        }}),
    )
    .await
    .unwrap();
    assert!(pdf_result.verified);
    let pdf_bytes = std::fs::read(&pdf_destination).unwrap();
    assert!(pdf_bytes.starts_with(b"%PDF-"));
    assert!(pdf_bytes.len() > 100);
    let mlc_root = crate::settings::app_data_root()
        .join("logs")
        .join("mlc")
        .join(format!(
            "chat-pdf-final-verification-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ms_i64()
        ));
    std::fs::create_dir_all(&mlc_root).unwrap();
    let mlc_path = mlc_root.join("success.md");
    let mlc_claims = pdf_result.claims.join("\n- ");
    std::fs::write(
        &mlc_path,
        format!("# Logical Certificate\n\n## Claims\n- {mlc_claims}\n"),
    )
    .unwrap();
    let final_verification = crate::verifier::MlcVerifier::new()
        .verify_with_identity(&mlc_path.to_string_lossy(), &identity)
        .expect("the final MLC verifier accepts the real PDF creation claim");
    assert!(final_verification.verified);

    let connection = engine.open_connection().unwrap();
    let (project_id, task_run_id): (String, String) = connection
        .query_row(
            "SELECT t.project_id,t.task_run_id FROM task_runs t WHERE t.runtime_kind='agent' AND t.runtime_record_id='execution-chat-file'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let event_count = connection
        .prepare("SELECT event_json FROM task_events WHERE task_run_id=?1")
        .unwrap()
        .query_map([task_run_id], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|event| {
            serde_json::from_str::<Value>(event)
                .ok()
                .and_then(|value| {
                    value
                        .get("eventType")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .as_deref()
                == Some("file.created")
        })
        .count();
    assert_eq!(
        project_id,
        crate::projects::repository::INTERNAL_LOCAL_FILES_PROJECT_ID
    );
    assert_eq!(event_count, 2);

    drop(connection);
    drop(engine);
    let _ = std::fs::remove_dir_all(mlc_root);
    let _ = std::fs::remove_dir_all(root);
}
