use super::*;

#[test]
fn workflow_output_reveal_is_confined_to_app_workspace() {
    let suffix = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!(
        "oomu-reveal-workflow-output-{}-{suffix}",
        unix_time_ms()
    ));
    let root = base.join("workspace");
    let outside = base.join("outside.txt");
    fs::create_dir_all(root.join("run-a")).unwrap();
    fs::write(root.join("run-a/result.txt"), "result").unwrap();
    fs::write(&outside, "outside").unwrap();

    let inside = resolve_workflow_reveal_path(&root, "run-a/result.txt").unwrap();
    assert!(inside.starts_with(fs::canonicalize(&root).unwrap()));
    let outside_error = resolve_workflow_reveal_path(&root, outside.to_str().unwrap()).unwrap_err();
    assert_eq!(outside_error.code, "workflow_runtime_permission_rejected");
    assert!(!outside_error.message.contains(outside.to_str().unwrap()));
    assert!(resolve_workflow_reveal_path(&root, "../outside.txt").is_err());
    let _ = fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn workflow_output_reveal_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let suffix = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!(
        "oomu-reveal-workflow-symlink-{}-{suffix}",
        unix_time_ms()
    ));
    let root = base.join("workspace");
    let outside = base.join("outside.txt");
    fs::create_dir_all(&root).unwrap();
    fs::write(&outside, "outside").unwrap();
    symlink(&outside, root.join("result.txt")).unwrap();

    assert!(resolve_workflow_reveal_path(&root, "result.txt").is_err());
    let _ = fs::remove_dir_all(base);
}

#[test]
fn production_workflow_knowledge_sync_rejects_path_authority() {
    let error = KnowledgeRuntimeTools
        .execute_sync_knowledge_vault(json!({
            "path": "/",
            "workspaceRoot": "/"
        }))
        .unwrap_err();

    assert_eq!(error.code, "workflow_runtime_execution_failed");
    assert!(error.message.contains("native picker grant"));
}

#[test]
fn volatile_persistence_blocks_workflow_before_instance_side_effect() {
    let root =
        std::env::temp_dir().join(format!("oomu-volatile-workflow-runtime-{}", unix_time_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let persistence = PersistenceEngine::initialize_volatile_at(root.join("state.sqlite")).unwrap();

    let error = require_durable_workflow_actuation(&persistence, "compiled workflow actuation")
        .unwrap_err();
    assert_eq!(error.code, "workflow_volatile_persistence_blocked");
    let instance_count: i64 = persistence
        .open_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM execution_instances", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(instance_count, 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn executes_in_topological_order_and_tracks_tokens() {
    let compiled = compiled_workflow(true);
    let request = RunWorkflowRequest {
        workflow_id: "workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!({"message": "hello"}),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();
    let order = execute_workflow(
        &compiled,
        &request,
        &StubModel,
        &NoExternalTools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        order.execution_order,
        vec!["input", "agent", "router", "output"]
    );
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert_eq!(instance.total_tokens, 10);
    assert!(instance.output_payload.is_some());
}

#[test]
fn calendar_assistant_outlives_legacy_deadline_and_completes_end_to_end() {
    let _auto_approve_mcp = crate::tool_security::AutoApproveMcpTestGuard::enable();
    let (result, instance, model_calls, tool_calls, _) =
        execute_collection_workflow_fixture_with_delay(
            calendar_assistant_workflow(),
            json!({
                "content": [{ "type": "text", "text": "one upcoming event" }],
                "structuredContent": {
                    "events": [{
                        "calendar": "Work",
                        "name": "Quarterly review",
                        "startTime": "2026-07-15T13:00:00-04:00",
                        "endTime": "2026-07-15T14:00:00-04:00",
                        "location": "Conference Room"
                    }]
                },
                "isError": false
            }),
            25,
        );

    let outcome = result.expect(
        "the runtime must upgrade the saved one-millisecond Calendar deadline before running",
    );
    assert_eq!(
        outcome.execution_order,
        vec![
            "input",
            "calendar-assistant-read",
            "calendar-assistant-has-events",
            "meeting-audit",
            "calendar-notification",
            "output"
        ]
    );
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert_eq!(model_calls, 1);
    assert_eq!(tool_calls, 2);
    assert!(instance.output_payload.is_some());
}

#[test]
fn legacy_whole_envelope_agent_input_skips_model_and_downstream_tool() {
    let (result, instance, model_calls, tool_calls, _) = execute_collection_workflow_fixture(
        whole_envelope_collection_workflow(),
        json!({
            "content": [],
            "structuredContent": {"emails": []},
            "isError": false
        }),
    );
    let outcome = result.unwrap();

    assert_eq!(outcome.execution_order, vec!["input", "read"]);
    assert_eq!(model_calls, 0);
    assert_eq!(tool_calls, 1);
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert_eq!(instance.output_payload, Some(completed_empty_envelope()));
    assert!(!instance.node_payloads.contains_key("consumer"));
    assert!(!instance.node_payloads.contains_key("downstream-tool"));
}

#[test]
fn custom_server_cannot_spoof_authoritative_read_tool_name() {
    let _auto_approve_mcp = crate::tool_security::AutoApproveMcpTestGuard::enable();
    let (result, instance, model_calls, tool_calls, _) = execute_collection_workflow_fixture(
        custom_server_read_spoof_workflow(),
        json!({
            "content": [],
            "structuredContent": {"emails": []},
            "isError": false
        }),
    );
    let outcome = result.unwrap();

    assert_eq!(
        outcome.execution_order,
        vec!["input", "read", "consumer", "downstream-tool", "output"]
    );
    assert_eq!(model_calls, 1);
    assert_eq!(tool_calls, 2);
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert!(!is_completed_empty_envelope(
        instance.output_payload.as_ref().unwrap()
    ));
}

#[test]
fn downstream_offline_server_is_lazy_and_only_fails_when_selected() {
    let (empty_result, empty_instance, empty_model_calls, empty_checks, empty_executions) =
        execute_selective_readiness_fixture(json!({
            "content": [],
            "structuredContent": {"emails": []},
            "isError": false
        }));
    let empty_outcome = empty_result.unwrap();

    assert_eq!(empty_outcome.execution_order, vec!["input", "read"]);
    assert_eq!(empty_instance.status, ExecutionStatus::Completed);
    assert_eq!(empty_model_calls, 0);
    assert_eq!(empty_checks, vec!["local_filesystem"]);
    assert_eq!(empty_executions, vec!["local_filesystem"]);
    assert!(!empty_instance.node_payloads.contains_key("downstream-tool"));

    let (selected_result, selected_instance, selected_model_calls, checks, executions) =
        execute_selective_readiness_fixture(json!({
            "content": [],
            "structuredContent": {
                "emails": [{"subject": "Quarterly review"}]
            },
            "isError": false
        }));
    let error = selected_result.unwrap_err();

    assert_eq!(error.code, "workflow_runtime_mcp_preflight_failed");
    assert_eq!(selected_instance.status, ExecutionStatus::Failed);
    assert_eq!(selected_model_calls, 1);
    assert_eq!(checks, vec!["local_filesystem", "offline_writer"]);
    assert_eq!(executions, vec!["local_filesystem"]);
    assert_eq!(
        selected_instance.node_payloads["downstream-tool"].status,
        ExecutionStatus::Failed
    );
    assert!(selected_instance.output_payload.is_none());
}

#[test]
fn schema_less_legacy_rescue_rejects_ambiguous_array_fields() {
    let (result, instance, model_calls, tool_calls, checkpoints) =
        execute_collection_workflow_fixture(
            whole_envelope_collection_workflow(),
            json!({
                "content": [],
                "structuredContent": {
                    "emails": [],
                    "warnings": []
                },
                "isError": false
            }),
        );
    let outcome = result.unwrap();

    assert_eq!(
        outcome.execution_order,
        vec!["input", "read", "consumer", "downstream-tool", "output"]
    );
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert_eq!(model_calls, 1);
    assert_eq!(tool_calls, 2);
    assert!(!is_completed_empty_envelope(
        instance.output_payload.as_ref().unwrap()
    ));
    assert!(!checkpoints.iter().any(|checkpoint| {
        checkpoint
            .output_payload
            .as_ref()
            .is_some_and(is_completed_empty_envelope)
    }));
}

#[test]
fn failed_run_report_projects_every_checkpointed_step_in_workflow_order() {
    let compiled = compiled_workflow(false);
    let request = RunWorkflowRequest {
        workflow_id: compiled.workflow_ir.workflow_id.clone(),
        workflow_version: Some(compiled.workflow_ir.workflow_version),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!({"objective":"test"}),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();
    instance.node_payloads.insert(
        "input".to_string(),
        NodeExecutionPayload {
            status: ExecutionStatus::Completed,
            input: None,
            output: Some(json!({"data":{"objective":"test"}})),
            error: None,
            latency_ms: 1,
            prompt_tokens: 0,
            completion_tokens: 0,
        },
    );
    instance.node_payloads.insert(
        "agent".to_string(),
        NodeExecutionPayload {
            status: ExecutionStatus::Failed,
            input: None,
            output: None,
            error: Some(json!({"message":"The evidence report omitted M1."})),
            latency_ms: 2,
            prompt_tokens: 0,
            completion_tokens: 0,
        },
    );

    assert_eq!(
        recorded_execution_order(&compiled.workflow_ir, &instance),
        vec!["input", "agent"]
    );
}
