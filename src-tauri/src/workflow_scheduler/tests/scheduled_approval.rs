use super::*;

#[test]
fn scheduled_pause_projects_task_before_exposing_exact_approval() {
    let root = std::env::temp_dir().join(format!(
        "oomu-scheduled-approval-projection-{}",
        unix_time_ms()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let now = unix_time_ms();
    engine.open_connection().unwrap().execute(
        "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,workflow_ir_json,is_active,created_at_ms,updated_at_ms,compiled_at_ms) VALUES ('workflow-scheduled-approval',1,'Supplier exception','','{}',NULL,1,?1,?1,?1)",
        rusqlite::params![now],
    ).unwrap();
    let instance = ExecutionInstance {
        id: "wfi-scheduled-approval".to_string(),
        workflow_id: "workflow-scheduled-approval".to_string(),
        workflow_version: 1,
        status: ExecutionStatus::AwaitingApproval,
        active_node_id: Some("approve-calendar".to_string()),
        input_payload: json!({}),
        output_payload: None,
        node_payloads: std::collections::HashMap::new(),
        memory: std::collections::HashMap::new(),
        selected_edges: std::collections::HashSet::new(),
        pause_context: None,
        error: None,
        execution_latency_ms: 1,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        created_at_ms: now,
        started_at_ms: Some(now),
        updated_at_ms: now,
        completed_at_ms: None,
    };
    engine.insert_execution_instance(&instance).unwrap();
    let connection = engine.open_connection().unwrap();
    connection.execute(
        "INSERT INTO workflow_schedules(id,workflow_id,label,schedule_expression,run_request_json,is_active,created_at_ms,updated_at_ms) VALUES ('routine_approval','workflow-scheduled-approval','Supplier exception','manual','{}',1,?1,?1)",
        rusqlite::params![now],
    ).unwrap();
    connection.execute(
        "INSERT INTO task_runs(task_run_id,task_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES ('taskrun_scheduled_approval','task_scheduled_approval','workflow',?1,'running','routine','scheduled-approval','Supplier exception',?2,?2,'reconciled')",
        rusqlite::params![instance.id, now],
    ).unwrap();
    connection.execute(
        "INSERT INTO routine_runs(schedule_id,execution_instance_id,task_run_id,scheduled_for_ms,created_at_ms) VALUES ('routine_approval',?1,'taskrun_scheduled_approval',?2,?2)",
        rusqlite::params![instance.id, now],
    ).unwrap();
    drop(connection);

    let task_run_id = project_workflow_task(&engine, &instance.id, true).unwrap();
    assert_eq!(task_run_id, "taskrun_scheduled_approval");
    let state: String = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT state FROM task_runs WHERE task_run_id=?1",
            rusqlite::params![task_run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "awaiting_approval");

    let approval = workflow_runtime::ApprovalRequest {
        instance_id: instance.id.clone(),
        workflow_id: instance.workflow_id.clone(),
        node_id: "approve-calendar".to_string(),
        message: "Create the exact Calendar event?".to_string(),
        context: json!({"calendarName":"OOMU Test","title":"Supplier Exception Follow-up"}),
        approval_token: "approval-token".to_string(),
        approve_command: json!({"decision":"approve"}),
        reject_command: json!({"decision":"reject"}),
    };
    let response = workflow_runtime::RunWorkflowResponse {
        instance: instance.clone(),
        execution_order: vec!["write-report".to_string()],
        approval_request: Some(approval),
        completion: None,
    };
    let dispatch = scheduled_approval(&response).expect("approval event payload");
    assert_eq!(dispatch.instance_id, instance.id);
    assert_eq!(dispatch.node_id, "approve-calendar");
    assert_eq!(dispatch.approval_token, "approval-token");

    let mut completed = response;
    completed.instance.status = ExecutionStatus::Completed;
    assert!(scheduled_approval(&completed).is_none());
    std::fs::remove_dir_all(root).unwrap();
}
