use super::*;
use crate::projects::{CreateProjectRequest, ProjectDataPolicy};

#[test]
fn legacy_unbound_agent_task_is_repaired_before_a_native_task_tool_runs() {
    let root = std::env::temp_dir().join(format!(
        "oomu-agent-task-project-repair-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE taskflows (flow_id TEXT PRIMARY KEY, parent_session_id TEXT NOT NULL, directive TEXT NOT NULL, status TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);
             CREATE TABLE taskflow_steps (flow_id TEXT NOT NULL, status TEXT NOT NULL);",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO chat_sessions (id,workspace_id,agent_id,title,provider_id,model_id,created_at_ms,updated_at_ms) VALUES ('session-unbound-task','00000000-0000-4000-8000-000000000001','agent-unbound-task','Create a PDF','local_model','gemma-test',1,1)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO agent_executions (execution_id,plan_id,session_id,agent_id,provider_id,model_id,turn_id,generation_token,root_turn_id,turn_kind,context_json,status,created_at_ms,updated_at_ms) VALUES ('execution-unbound-task','plan-unbound-task','session-unbound-task','agent-unbound-task','local_model','gemma-test','turn-unbound-task','generation-unbound-task','turn-unbound-task','root','{}','running',1,1)",
            [],
        )
        .unwrap();
    drop(connection);

    let task = require_agent_runtime_task(&engine, "execution-unbound-task").unwrap();
    assert_eq!(
        task.project_id.as_deref(),
        Some(crate::projects::repository::INTERNAL_LOCAL_FILES_PROJECT_ID)
    );
    let connection = engine.open_connection().unwrap();
    let (session_project, execution_project): (Option<String>, Option<String>) = connection
        .query_row(
            "SELECT c.project_id,a.project_id FROM chat_sessions c JOIN agent_executions a ON a.session_id=c.id WHERE a.execution_id='execution-unbound-task'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(session_project.is_none());
    assert_eq!(execution_project, task.project_id);

    drop(connection);
    drop(engine);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn scheduled_workflow_task_is_available_to_registered_native_tools() {
    let root = std::env::temp_dir().join(format!(
        "oomu-workflow-task-project-binding-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
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
            name: "Scheduled native tools".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let task_id = crate::p0_contracts::TaskId::new().to_string();
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    let execution_id = "workflow-execution-native-tool";
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO task_runs(task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'workflow',?4,'running','routine',?2,'Scheduled native tool',1234,1234,'reconciled')",
            params![task_run_id, task_id, project.project_id, execution_id],
        )
        .unwrap();

    let task = require_agent_runtime_task(&engine, execution_id).unwrap();
    assert_eq!(task.task_run_id, task_run_id);
    assert_eq!(
        task.project_id.as_deref(),
        Some(project.project_id.as_str())
    );
    assert_eq!(task.created_at_ms, 1234);

    drop(engine);
    let _ = std::fs::remove_dir_all(root);
}
