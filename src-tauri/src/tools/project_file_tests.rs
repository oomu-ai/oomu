use super::*;
use crate::projects::{CreateProjectRequest, ProjectDataPolicy};

fn register_test_task_runtime() {
    use crate::tools::task_runtime::{AgentRuntimeTaskBinding, TaskRuntimeRegistration};

    fn require_bound(
        engine: &crate::db::PersistenceEngine,
        task_run_id: &str,
        project_id: &str,
    ) -> Result<(), String> {
        let matches: i64 = engine
            .open_connection()
            .map_err(|error| error.to_string())?
            .query_row(
                "SELECT COUNT(*) FROM task_runs WHERE task_run_id=?1 AND project_id=?2",
                params![task_run_id, project_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        (matches == 1)
            .then_some(())
            .ok_or_else(|| "Task is not bound to the requested Project.".to_string())
    }

    fn require_agent_runtime(
        engine: &crate::db::PersistenceEngine,
        execution_id: &str,
    ) -> Result<AgentRuntimeTaskBinding, String> {
        engine
            .open_connection()
            .map_err(|error| error.to_string())?
            .query_row(
                "SELECT task_id,task_run_id,project_id FROM task_runs WHERE runtime_kind='workflow' AND runtime_record_id=?1",
                params![execution_id],
                |row| {
                    Ok(AgentRuntimeTaskBinding {
                        task_id: row.get(0)?,
                        task_run_id: row.get(1)?,
                        project_id: row.get(2)?,
                    })
                },
            )
            .map_err(|error| error.to_string())
    }

    fn record_with_sequence(
        engine: &crate::db::PersistenceEngine,
        task_run_id: &str,
        event_type: &str,
        evidence: crate::p0_contracts::EvidenceClass,
        payload: Value,
    ) -> Result<u64, String> {
        let connection = engine
            .open_connection()
            .map_err(|error| error.to_string())?;
        let sequence: i64 = connection
            .query_row(
                "SELECT COALESCE(MAX(sequence), -1) + 1 FROM task_events WHERE task_run_id=?1",
                params![task_run_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO task_events(task_run_id,sequence,event_json,created_at_ms) VALUES (?1,?2,?3,1)",
                params![
                    task_run_id,
                    sequence,
                    json!({
                        "eventType": event_type,
                        "evidenceClass": evidence,
                        "payload": payload,
                    })
                    .to_string()
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(sequence as u64)
    }

    fn record(
        engine: &crate::db::PersistenceEngine,
        task_run_id: &str,
        event_type: &str,
        evidence: crate::p0_contracts::EvidenceClass,
        payload: Value,
    ) -> Result<(), String> {
        record_with_sequence(engine, task_run_id, event_type, evidence, payload).map(|_| ())
    }

    crate::tools::task_runtime::register(TaskRuntimeRegistration {
        record_event: record,
        record_event_with_sequence: record_with_sequence,
        require_bound_task: require_bound,
        require_agent_runtime_task: require_agent_runtime,
    })
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn registered_adapter_reads_real_absolute_project_file_outside_mcp_sandbox() {
    let root = std::env::temp_dir().join(format!(
        "oomu-registered-project-read-{}",
        crate::p0_contracts::TaskId::new()
    ));
    let project_root = root.join("approved-external-project");
    fs::create_dir_all(&project_root).unwrap();
    let project_root = fs::canonicalize(project_root).unwrap();
    let fixture = project_root.join("reviewed_input.txt");
    let expected = "Verified external Project evidence.\n";
    fs::write(&fixture, expected).unwrap();
    let fixture = fs::canonicalize(fixture).unwrap();
    let engine = crate::db::PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute_batch(
            "CREATE TABLE taskflows (flow_id TEXT PRIMARY KEY, parent_session_id TEXT NOT NULL, directive TEXT NOT NULL, status TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);
             CREATE TABLE taskflow_steps (flow_id TEXT NOT NULL, status TEXT NOT NULL);",
        )
        .unwrap();
    let project = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: "External Project read".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let execution_id = "workflow-external-project-read";
    let task_id = crate::p0_contracts::TaskId::new().to_string();
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO project_sources(source_id,project_id,source_kind,canonical_path,grant_reference,grant_state,created_at_ms,updated_at_ms) VALUES ('source-external-read',?1,'knowledge_directory',?2,?3,'active',1,1)",
            params![
                project.project_id,
                project_root.to_string_lossy(),
                "c".repeat(64)
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO task_runs(task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'workflow',?4,'running','routine',?2,'External Project read',1,1,'reconciled')",
            params![task_run_id, task_id, project.project_id, execution_id],
        )
        .unwrap();
    drop(connection);
    register_test_task_runtime();
    if register_task_tool().is_err() {
        assert!(crate::tools::task_tool_runtime::schema(OPERATION).is_ok());
    }
    let request = crate::tools::task_tool_runtime::validate(
        OPERATION,
        json!({"path":fixture.to_string_lossy(),"maxBytes":4096}),
    )
    .unwrap();
    let request =
        crate::tools::task_tool_runtime::resolve(&engine, Some(execution_id), request, &[])
            .unwrap();
    let identity = crate::sovereign_identity::SovereignIdentity::initialize_ephemeral();
    let response = crate::tools::task_tool_runtime::execute(
        crate::tools::task_tool_runtime::TaskToolExecutionContext {
            persistence: &engine,
            identity: &identity,
            app: None,
            execution_id: Some(execution_id),
            plan_id: None,
            objective: Some("Read the exact external Project fixture"),
            session_id: None,
            model_route: None,
        },
        request,
    )
    .await
    .unwrap();
    assert!(response.verified);
    assert_eq!(response.operation, OPERATION);
    let receipt: Value = serde_json::from_str(&response.message).unwrap();
    assert_eq!(receipt["canonicalPath"], fixture.to_string_lossy().as_ref());
    assert_eq!(receipt["content"], expected);
    assert_eq!(receipt["byteCount"], expected.len());
    assert_eq!(
        receipt["contentSha256"],
        crate::foundation::digest::sha256_hex(expected.as_bytes())
    );
    assert_eq!(receipt["verified"], true);
    let event_count: i64 = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM task_events WHERE task_run_id=?1 AND json_extract(event_json,'$.eventType')='project_file.read'",
            params![task_run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(event_count, 1);

    drop(engine);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn registered_project_reader_rejects_approved_root_symlink_drift() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!(
        "oomu-project-read-root-drift-{}",
        crate::p0_contracts::TaskId::new()
    ));
    let stored_root = root.join("approved-root");
    fs::create_dir_all(&stored_root).unwrap();
    let stored_root = fs::canonicalize(stored_root).unwrap();
    let moved_root = root.join("moved-root");
    let engine = crate::db::PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: "Drifted Project root".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO project_sources(source_id,project_id,source_kind,canonical_path,grant_reference,grant_state,created_at_ms,updated_at_ms) VALUES ('source-drift',?1,'knowledge_directory',?2,?3,'active',1,1)",
            params![
                project.project_id,
                stored_root.to_string_lossy(),
                "e".repeat(64)
            ],
        )
        .unwrap();
    fs::rename(&stored_root, &moved_root).unwrap();
    fs::write(moved_root.join("fixture.txt"), "real bytes").unwrap();
    symlink(&moved_root, &stored_root).unwrap();

    let error = read_project_file(
        &engine,
        &project.project_id,
        stored_root.join("fixture.txt").to_string_lossy().as_ref(),
        4_096,
    )
    .expect_err("stored Project root identity drift must fail closed");
    assert!(error.contains("identity changed"));

    drop(engine);
    let _ = fs::remove_dir_all(root);
}
