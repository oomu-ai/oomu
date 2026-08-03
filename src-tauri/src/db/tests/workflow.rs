use super::*;

#[test]
fn agent_execution_logs_append_and_replay_by_cursor() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_execution_log_test_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    let first = engine
        .insert_agent_execution_log(
            "exec-1",
            "plan-1",
            Some("session-1"),
            Some("agent-1"),
            "info",
            "running",
            "Reading workspace",
            None,
        )
        .unwrap();
    let second = engine
        .insert_agent_execution_log(
            "exec-1",
            "plan-1",
            Some("session-1"),
            Some("agent-1"),
            "info",
            "completed",
            "Execution complete",
            Some("{\"verified\":true}"),
        )
        .unwrap();

    let all = engine
        .select_agent_execution_logs_after("exec-1", 0, 10)
        .unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].message, "Reading workspace");
    assert!(!first.is_terminal());
    assert!(second.is_terminal());

    let restart_recovery = engine
        .insert_agent_execution_log(
            "exec-restart",
            "plan-restart",
            Some("session-1"),
            Some("agent-1"),
            "warn",
            "restart_recovery_ready",
            "Approval must be restored after restart",
            None,
        )
        .unwrap();
    assert!(restart_recovery.is_terminal());

    let replay = engine
        .select_agent_execution_logs_after("exec-1", first.id, 10)
        .unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].id, second.id);
    assert!(engine
        .select_agent_execution_logs_after("missing-exec", 0, 10)
        .unwrap()
        .is_empty());
}

#[test]
fn workflow_schedule_claim_and_result_lifecycle_is_transactional() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_workflow_schedule_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine {
        db_path: Arc::new(RwLock::new(temp_dir.join("workflow.sqlite"))),
        write_lock: Arc::new(Mutex::new(())),
        workspace_id: default_workspace_id(),
        storage_class: Arc::new(RwLock::new(BackingStoreClass::Persistent)),
    };
    engine.run_migrations().unwrap();

    let workflow = SavedWorkflowRecord {
        id: "wf-scheduled".to_string(),
        name: "Scheduled".to_string(),
        steps: r#"{"nodes":[]}"#.to_string(),
        created_at: 10,
        updated_at: 20,
    };
    let mut workflow_ir: WorkflowIr = serde_json::from_value(json!({
        "schemaVersion": "1.0.0",
        "workflowId": "wf-scheduled",
        "workflowVersion": 1,
        "name": "Scheduled",
        "description": "",
        "compiler": { "model": "gemma-4-e2b-qat" },
        "nodes": [
            {
                "kind": "input",
                "id": "input",
                "label": "Input",
                "outputKey": "workflow.input",
                "inputSchema": { "type": "object" }
            },
            {
                "kind": "output",
                "id": "output",
                "label": "Output",
                "inputMapping": "{{workflow.input}}",
                "outputSchema": { "type": "object" }
            }
        ],
        "edges": [
            {
                "id": "e1",
                "sourceNodeId": "input",
                "sourcePort": "out",
                "targetNodeId": "output"
            }
        ]
    }))
    .unwrap();
    let version = engine
        .reserve_workflow_blueprint(&workflow, &json!({"nodes": []}), &mut workflow_ir)
        .unwrap();
    engine
        .publish_compiled_workflow(&workflow, &workflow_ir, &[], true)
        .unwrap();

    let due_at_ms = unix_time_ms();
    let schedule = engine
        .upsert_workflow_schedule(WorkflowScheduleUpsert {
            id: "sched-claim".to_string(),
            workflow_id: "wf-scheduled".to_string(),
            workflow_version: Some(version),
            label: "Scheduled".to_string(),
            schedule_expression: "every 2 minutes".to_string(),
            run_request: json!({}),
            is_active: true,
            next_run_at_ms: Some(due_at_ms),
        })
        .unwrap();
    assert_eq!(schedule.next_run_at_ms, Some(due_at_ms));

    // A wall-clock correction must not make the mutable timestamp older than
    // the schedule's creation timestamp or prevent an otherwise due claim.
    let claim_at_ms = schedule.created_at_ms.saturating_sub(1);
    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE workflow_schedules SET next_run_at_ms=?2 WHERE id=?1",
            params![schedule.id, claim_at_ms],
        )
        .unwrap();

    let claimed = engine
        .claim_due_workflow_schedules(claim_at_ms, 1, 60_000)
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].last_status.as_deref(), Some("Running"));
    assert_eq!(
        engine
            .claim_due_workflow_schedules(claim_at_ms, 1, 60_000)
            .unwrap()
            .len(),
        0
    );

    engine
        .mark_workflow_schedule_run_result(
            "sched-claim",
            ExecutionStatus::AwaitingApproval,
            Some("wfi-waiting"),
            None,
            None,
        )
        .unwrap();
    let connection = engine.open_connection().unwrap();
    let awaiting = select_workflow_schedule_by_id(&connection, "sched-claim").unwrap();
    assert_eq!(awaiting.last_status.as_deref(), Some("AwaitingApproval"));
    assert_eq!(awaiting.last_instance_id.as_deref(), Some("wfi-waiting"));
    assert_eq!(awaiting.last_completed_at_ms, None);
    drop(connection);

    engine
        .mark_workflow_schedule_run_result(
            "sched-claim",
            ExecutionStatus::Completed,
            Some("wfi-test"),
            None,
            Some(claim_at_ms + 120_000),
        )
        .unwrap();
    let connection = engine.open_connection().unwrap();
    let completed = select_workflow_schedule_by_id(&connection, "sched-claim").unwrap();
    assert!(completed.last_completed_at_ms.is_some());
    drop(connection);
    let claimed_again = engine
        .claim_due_workflow_schedules(claim_at_ms + 120_000, 1, 60_000)
        .unwrap();
    assert_eq!(claimed_again.len(), 1);
    assert_eq!(
        claimed_again[0].last_instance_id.as_deref(),
        Some("wfi-test")
    );

    drop(claimed_again);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn workflow_compilation_lifecycle_publishes_atomically() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_workflow_compile_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine {
        db_path: Arc::new(RwLock::new(temp_dir.join("workflow.sqlite"))),
        write_lock: Arc::new(Mutex::new(())),
        workspace_id: default_workspace_id(),
        storage_class: Arc::new(RwLock::new(BackingStoreClass::Persistent)),
    };
    engine.run_migrations().unwrap();

    let workflow = SavedWorkflowRecord {
        id: "wf-compile".to_string(),
        name: "Compile".to_string(),
        steps: r#"{"nodes":[]}"#.to_string(),
        created_at: 10,
        updated_at: 20,
    };
    let mut workflow_ir: WorkflowIr = serde_json::from_value(json!({
        "schemaVersion": "1.0.0",
        "workflowId": "wf-compile",
        "workflowVersion": 1,
        "name": "Compile",
        "description": "",
        "compiler": { "model": "gemma-4-e2b-qat" },
        "nodes": [
            {
                "kind": "input",
                "id": "input",
                "label": "Input",
                "outputKey": "workflow.input",
                "inputSchema": { "type": "object" }
            },
            {
                "kind": "agent",
                "id": "agent",
                "label": "Agent",
                "objective": "Compile",
                "inputMappings": { "context": "{{workflow.input}}" },
                "outputKey": "nodes.agent.output"
            },
            {
                "kind": "output",
                "id": "output",
                "label": "Output",
                "inputMapping": "{{nodes.agent.output}}",
                "outputSchema": { "type": "object" }
            }
        ],
        "edges": [
            {
                "id": "e1",
                "sourceNodeId": "input",
                "sourcePort": "out",
                "targetNodeId": "agent"
            },
            {
                "id": "e2",
                "sourceNodeId": "agent",
                "sourcePort": "out",
                "targetNodeId": "output"
            }
        ]
    }))
    .unwrap();

    let version = engine
        .reserve_workflow_blueprint(&workflow, &json!({"nodes": []}), &mut workflow_ir)
        .unwrap();
    assert_eq!(version, 1);
    let instruction = CompiledInstruction {
        id: "instruction".to_string(),
        workflow_id: workflow.id.clone(),
        workflow_version: version,
        node_id: "agent".to_string(),
        node_kind: WorkflowNodeKind::Agent,
        system_prompt: "Compile deterministically.".to_string(),
        input_variable_mappings: HashMap::from([(
            "context".to_string(),
            "{{workflow.input}}".to_string(),
        )]),
        evaluation_protocol: json!({
            "successCriteria": ["Output exists."],
            "failureAction": "fail",
            "maxRetries": 0
        }),
        compiler_model: "gemma-4-e2b-qat".to_string(),
        compiler_version: "1.0.0".to_string(),
        created_at_ms: 20,
    };
    engine
        .publish_compiled_workflow(&workflow, &workflow_ir, &[instruction], true)
        .unwrap();

    let loaded = engine
        .load_compiled_workflow("wf-compile", Some(1))
        .unwrap();
    assert_eq!(loaded.workflow_ir.workflow_version, 1);
    assert_eq!(loaded.instructions.len(), 1);
    assert_eq!(
        loaded.instructions["agent"].system_prompt,
        "Compile deterministically."
    );

    let mut instance = ExecutionInstance {
        id: "run-compile".to_string(),
        workflow_id: "wf-compile".to_string(),
        workflow_version: 1,
        status: ExecutionStatus::Pending,
        active_node_id: None,
        input_payload: json!({"input": "hello"}),
        output_payload: None,
        node_payloads: HashMap::new(),
        memory: HashMap::new(),
        selected_edges: Default::default(),
        pause_context: None,
        error: None,
        execution_latency_ms: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        created_at_ms: 21,
        started_at_ms: None,
        updated_at_ms: 21,
        completed_at_ms: None,
    };
    engine.insert_execution_instance(&instance).unwrap();
    instance.status = ExecutionStatus::Completed;
    instance.output_payload = Some(json!({"result": "done"}));
    instance.prompt_tokens = 8;
    instance.completion_tokens = 3;
    instance.total_tokens = 11;
    instance.started_at_ms = Some(22);
    instance.updated_at_ms = 24;
    instance.completed_at_ms = Some(24);
    instance.execution_latency_ms = 2;
    engine.update_execution_instance(&instance).unwrap();

    let connection = engine.open_connection().unwrap();
    let state: (String, i64, i64) = connection
        .query_row(
            "
                SELECT compilation_status, is_active,
                    (SELECT COUNT(*) FROM compiled_instructions
                     WHERE workflow_id = 'wf-compile' AND workflow_version = 1)
                FROM workflow_blueprints
                WHERE workflow_id = 'wf-compile' AND version = 1
                ",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, ("Compiled".to_string(), 1, 1));
    let run_state: (String, i64, i64) = connection
        .query_row(
            "
                SELECT status, total_tokens, execution_latency_ms
                FROM execution_instances
                WHERE id = 'run-compile'
                ",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(run_state, ("Completed".to_string(), 11, 2));
    assert_eq!(engine.select_workflows().unwrap().len(), 1);

    drop(connection);
    let mut workflow_v2 = workflow.clone();
    workflow_v2.updated_at = 30;
    let version = engine
        .reserve_workflow_blueprint(&workflow_v2, &json!({"nodes": []}), &mut workflow_ir)
        .unwrap();
    assert_eq!(version, 2);
    let instruction = CompiledInstruction {
        id: "instruction-v2".to_string(),
        workflow_id: workflow_v2.id.clone(),
        workflow_version: version,
        node_id: "agent".to_string(),
        node_kind: WorkflowNodeKind::Agent,
        system_prompt: "Compile version two deterministically.".to_string(),
        input_variable_mappings: HashMap::from([(
            "context".to_string(),
            "{{workflow.input}}".to_string(),
        )]),
        evaluation_protocol: json!({
            "successCriteria": ["Output exists."],
            "failureAction": "fail",
            "maxRetries": 0
        }),
        compiler_model: "gemma-4-e2b-qat".to_string(),
        compiler_version: "1.0.0".to_string(),
        created_at_ms: 30,
    };
    engine
        .publish_compiled_workflow(&workflow_v2, &workflow_ir, &[instruction], false)
        .unwrap();
    let connection = engine.open_connection().unwrap();
    let active_count: i64 = connection
        .query_row(
            "
                SELECT COUNT(*)
                FROM workflow_blueprints
                WHERE workflow_id = 'wf-compile' AND is_active = 1
                ",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active_count, 0);

    drop(connection);
    assert!(engine.delete_workflow_by_id("wf-compile").unwrap());
    let connection = engine.open_connection().unwrap();
    let remaining: (i64, i64, i64, i64) = connection
        .query_row(
            "
                SELECT
                    (SELECT COUNT(*) FROM workflows WHERE id = 'wf-compile'),
                    (SELECT COUNT(*) FROM workflow_blueprints WHERE workflow_id = 'wf-compile'),
                    (SELECT COUNT(*) FROM compiled_instructions WHERE workflow_id = 'wf-compile'),
                    (SELECT COUNT(*) FROM execution_instances WHERE workflow_id = 'wf-compile')
                ",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(remaining, (0, 0, 0, 0));

    drop(connection);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn scheduled_knowledge_sync_helper_creates_compiled_workflow_and_schedule() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_knowledge_sync_{}", unix_time_ms()));
    let vault_dir = temp_dir.join("vault");
    std::fs::create_dir_all(&vault_dir).unwrap();
    let engine = PersistenceEngine {
        db_path: Arc::new(RwLock::new(temp_dir.join("workflow.sqlite"))),
        write_lock: Arc::new(Mutex::new(())),
        workspace_id: default_workspace_id(),
        storage_class: Arc::new(RwLock::new(BackingStoreClass::Persistent)),
    };
    engine.run_migrations().unwrap();

    let next_run_at_ms = unix_time_ms() + 120_000;
    let record = engine
        .create_scheduled_knowledge_sync_workflow(
            vault_dir.to_str().unwrap(),
            "every 2 hours",
            next_run_at_ms,
        )
        .unwrap();

    assert_eq!(record.workflow_version, 1);
    assert_eq!(record.schedule_expression, "every 2 hours");
    assert_eq!(record.next_run_at_ms, next_run_at_ms);
    assert!(record.workflow_id.starts_with("knowledge-sync-"));

    let compiled = engine
        .load_compiled_workflow(&record.workflow_id, Some(record.workflow_version))
        .unwrap();
    assert_eq!(compiled.instructions.len(), 0);
    assert_eq!(compiled.workflow_ir.nodes.len(), 3);

    let claimed = engine
        .claim_due_workflow_schedules(next_run_at_ms, 1, 60_000)
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, record.schedule_id);
    assert_eq!(claimed[0].workflow_id, record.workflow_id);
    assert_eq!(claimed[0].workflow_version, Some(record.workflow_version));

    let _ = std::fs::remove_dir_all(temp_dir);
}
