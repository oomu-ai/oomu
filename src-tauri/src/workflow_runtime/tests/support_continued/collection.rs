use super::*;

#[test]
fn successful_empty_collection_completes_before_legacy_linear_downstream_work() {
    let (result, instance, model_calls, tool_calls, checkpoints) =
        execute_indexed_collection_fixture(
            json!({
                "content": [],
                "structuredContent": {"emails": []},
                "isError": false
            }),
            0,
        );
    let outcome = result.unwrap();

    assert_eq!(outcome.execution_order, vec!["input", "read"]);
    assert_eq!(model_calls, 0);
    assert_eq!(tool_calls, 1);
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert_eq!(instance.output_payload, Some(completed_empty_envelope()));
    assert!(!instance.node_payloads.contains_key("consumer"));
    assert_eq!(instance.prompt_tokens, 0);
    assert_eq!(
        checkpoints.last().map(|checkpoint| checkpoint.status),
        Some(ExecutionStatus::Completed)
    );
    assert_eq!(
        checkpoints
            .last()
            .and_then(|checkpoint| checkpoint.output_payload.as_ref()),
        Some(&completed_empty_envelope())
    );

    let response =
        run_workflow_response(instance, outcome.execution_order, outcome.approval_request);
    assert_eq!(
        response.completion,
        Some(WorkflowCompletion {
            kind: WorkflowCompletionKind::EmptyCollection,
        })
    );
    assert_eq!(
        serde_json::to_value(response).unwrap()["completion"]["kind"],
        json!("empty_collection")
    );
}

#[test]
fn email_responder_runs_end_to_end_through_empty_and_review_paths() {
    let (empty_result, empty_instance, empty_model_calls, empty_tool_calls, _) =
        execute_collection_workflow_fixture(
            email_responder_workflow(),
            json!({
                "content": [],
                "structuredContent": { "emails": [] },
                "isError": false
            }),
        );
    let empty_outcome = empty_result.expect("an empty inbox is a successful workflow run");
    assert_eq!(
        empty_outcome.execution_order,
        vec![
            "input",
            "read-unread-emails",
            "mail-has-messages",
            "empty-output"
        ]
    );
    assert_eq!(empty_instance.status, ExecutionStatus::Completed);
    assert_eq!(
        empty_instance.output_payload,
        Some(completed_empty_envelope())
    );
    assert_eq!(empty_model_calls, 0);
    assert_eq!(empty_tool_calls, 1);

    let (message_result, message_instance, model_calls, tool_calls, _) =
        execute_collection_workflow_fixture(
            email_responder_workflow(),
            json!({
                "content": [{ "type": "text", "text": "one unread message" }],
                "structuredContent": {
                    "emails": [{
                        "sender": "Maya Allan <maya@example.test>",
                        "subject": "Quarterly review",
                        "content": "Please review the summary."
                    }]
                },
                "isError": false
            }),
        );
    let message_outcome =
        message_result.expect("a message reaches the review boundary without failing");
    assert_eq!(
        message_outcome.execution_order,
        vec![
            "input",
            "read-unread-emails",
            "mail-has-messages",
            "draft-reply"
        ]
    );
    assert_eq!(message_instance.status, ExecutionStatus::AwaitingApproval);
    let approval = message_outcome
        .approval_request
        .expect("the draft must pause for review");
    assert_eq!(approval.node_id, "approve-email-reply");
    assert_eq!(model_calls, 1);
    assert_eq!(tool_calls, 1);
    assert!(!message_instance
        .node_payloads
        .contains_key("draft-outgoing-email"));
}

#[test]
fn legacy_whole_collection_input_skips_model_and_downstream_tool() {
    let (result, instance, model_calls, tool_calls, _) = execute_collection_workflow_fixture(
        whole_collection_workflow(),
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
fn schema_less_writer_warnings_do_not_imply_empty_collection_completion() {
    let _auto_approve_mcp = crate::tool_security::AutoApproveMcpTestGuard::enable();
    let (result, instance, model_calls, tool_calls, _) = execute_collection_workflow_fixture(
        writer_warning_envelope_workflow(),
        json!({
            "content": [],
            "structuredContent": {
                "success": true,
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
    assert_eq!(model_calls, 1);
    assert_eq!(tool_calls, 2);
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert!(!is_completed_empty_envelope(
        instance.output_payload.as_ref().unwrap()
    ));
}

#[test]
fn migrated_empty_output_does_not_disable_unmigrated_producer_rescue() {
    let (result, instance, model_calls, _, _) = execute_collection_workflow_fixture(
        mixed_migrated_and_legacy_collection_workflow(),
        json!({
            "content": [],
            "structuredContent": {"emails": []},
            "isError": false
        }),
    );
    let outcome = result.unwrap();

    assert!(outcome.execution_order.iter().any(|node| node == "read"));
    assert!(!outcome
        .execution_order
        .iter()
        .any(|node| node == "consumer"));
    assert!(!outcome
        .execution_order
        .iter()
        .any(|node| node == "downstream-tool"));
    assert_eq!(model_calls, 0);
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert_eq!(instance.output_payload, Some(completed_empty_envelope()));
}

#[test]
fn two_source_legacy_rescue_waits_for_all_relevant_collections() {
    for (mail_empty, reminders_empty) in
        [(true, true), (true, false), (false, true), (false, false)]
    {
        let compiled = two_source_legacy_collection_workflow();
        let request = RunWorkflowRequest {
            workflow_id: compiled.workflow_ir.workflow_id.clone(),
            workflow_version: Some(1),
            preflight_mode: WorkflowPreflightMode::default(),
            inputs: HashMap::from([(
                "input".to_string(),
                InputBinding::Manual { value: json!({}) },
            )]),
            outputs: HashMap::new(),
        };
        let model_calls = Arc::new(AtomicUsize::new(0));
        let model = CountingCollectionModel {
            calls: model_calls.clone(),
        };
        let executions = Arc::new(Mutex::new(Vec::new()));
        let tools = PerNodeCollectionTools {
            results: HashMap::from([
                (
                    "mail-read".to_string(),
                    json!({
                        "content": [],
                        "structuredContent": {
                            "emails": if mail_empty { json!([]) } else { json!([{"id": 1}]) }
                        },
                        "isError": false
                    }),
                ),
                (
                    "reminders-read".to_string(),
                    json!({
                        "content": [],
                        "structuredContent": {
                            "reminders": if reminders_empty { json!([]) } else { json!([{"id": 2}]) }
                        },
                        "isError": false
                    }),
                ),
            ]),
            executions: executions.clone(),
        };
        let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();
        let outcome = execute_workflow(
            &compiled,
            &request,
            &model,
            &tools,
            &std::env::temp_dir(),
            &mut instance,
            &mut |_| Ok(()),
            &mut |_, _, _, _, _| {},
            None,
            None,
        )
        .unwrap();

        if mail_empty && reminders_empty {
            assert_eq!(
                outcome.execution_order,
                vec!["input", "mail-read", "reminders-read"]
            );
            assert_eq!(model_calls.load(AtomicOrdering::SeqCst), 0);
            assert_eq!(
                executions.lock().unwrap().as_slice(),
                ["mail-read", "reminders-read"]
            );
            assert_eq!(instance.output_payload, Some(completed_empty_envelope()));
        } else {
            assert_eq!(
                outcome.execution_order,
                vec![
                    "input",
                    "mail-read",
                    "reminders-read",
                    "consumer",
                    "downstream-tool",
                    "output"
                ]
            );
            assert_eq!(model_calls.load(AtomicOrdering::SeqCst), 1);
            assert_eq!(
                executions.lock().unwrap().as_slice(),
                ["mail-read", "reminders-read", "downstream-tool"]
            );
            assert!(!is_completed_empty_envelope(
                instance.output_payload.as_ref().unwrap()
            ));
        }
        assert_eq!(instance.status, ExecutionStatus::Completed);
    }
}

#[test]
fn unrelated_nonempty_prefix_read_does_not_suppress_consumer_empty_completion() {
    let compiled = unrelated_prefix_collection_workflow();
    let request = RunWorkflowRequest {
        workflow_id: compiled.workflow_ir.workflow_id.clone(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual { value: json!({}) },
        )]),
        outputs: HashMap::new(),
    };
    let model_calls = Arc::new(AtomicUsize::new(0));
    let model = CountingCollectionModel {
        calls: model_calls.clone(),
    };
    let executions = Arc::new(Mutex::new(Vec::new()));
    let tools = PerNodeCollectionTools {
        results: HashMap::from([
            (
                "mail-read".to_string(),
                json!({
                    "content": [],
                    "structuredContent": {"emails": [{"id": 1}]},
                    "isError": false
                }),
            ),
            (
                "reminders-read".to_string(),
                json!({
                    "content": [],
                    "structuredContent": {"reminders": []},
                    "isError": false
                }),
            ),
        ]),
        executions: executions.clone(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    let outcome = execute_workflow(
        &compiled,
        &request,
        &model,
        &tools,
        &std::env::temp_dir(),
        &mut instance,
        &mut |_| Ok(()),
        &mut |_, _, _, _, _| {},
        None,
        None,
    )
    .unwrap();

    assert_eq!(
        outcome.execution_order,
        vec!["input", "mail-read", "reminders-read"]
    );
    assert_eq!(model_calls.load(AtomicOrdering::SeqCst), 0);
    assert_eq!(
        executions.lock().unwrap().as_slice(),
        ["mail-read", "reminders-read"]
    );
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert_eq!(instance.output_payload, Some(completed_empty_envelope()));
}

#[test]
fn nested_empty_contact_field_never_completes_the_whole_workflow() {
    let (result, instance, model_calls, tool_calls, checkpoints) =
        execute_collection_workflow_fixture(
            nested_contact_collection_workflow(),
            json!({
                "content": [],
                "structuredContent": {
                    "contacts": [{
                        "name": "Ada Lovelace",
                        "emails": []
                    }]
                },
                "isError": false
            }),
        );
    let error = result.unwrap_err();

    assert_eq!(error.code, "workflow_runtime_empty_collection_indexed");
    assert!(error.message.contains("contacts.0.emails is empty"));
    assert_eq!(instance.status, ExecutionStatus::Failed);
    assert!(instance.output_payload.is_none());
    assert_eq!(model_calls, 1);
    assert_eq!(tool_calls, 1);
    assert_eq!(
        instance.node_payloads["consumer"].status,
        ExecutionStatus::Completed
    );
    assert_eq!(
        instance.node_payloads["downstream-tool"].status,
        ExecutionStatus::Failed
    );
    assert!(!checkpoints.iter().any(|checkpoint| {
        checkpoint
            .output_payload
            .as_ref()
            .is_some_and(is_completed_empty_envelope)
    }));
    assert!(run_workflow_response(instance, Vec::new(), None)
        .completion
        .is_none());
}

#[test]
fn completed_empty_envelope_and_descriptor_survive_persistence_round_trip() {
    let root = std::env::temp_dir().join(format!(
        "oomu-empty-completion-persistence-{}",
        unix_time_ms()
    ));
    let persistence = PersistenceEngine::initialize_at(root.join("workflow.sqlite")).unwrap();
    let compiled = indexed_collection_workflow(0);
    let workflow = SavedWorkflowRecord {
        id: compiled.workflow_ir.workflow_id.clone(),
        name: compiled.workflow_ir.name.clone(),
        steps: r#"{"nodes":[]}"#.to_string(),
        created_at: 1,
        updated_at: 2,
    };
    let mut ir = compiled.workflow_ir.clone();
    persistence
        .reserve_workflow_blueprint(&workflow, &json!({"nodes": []}), &mut ir)
        .unwrap();
    let instructions = compiled.instructions.values().cloned().collect::<Vec<_>>();
    persistence
        .publish_compiled_workflow(&workflow, &ir, &instructions, true)
        .unwrap();
    let model_calls = Arc::new(AtomicUsize::new(0));
    let model = CountingCollectionModel {
        calls: model_calls.clone(),
    };
    let tool_calls = Arc::new(AtomicUsize::new(0));
    let tools = FixedMcpResultTools {
        result: json!({
            "content": [],
            "structuredContent": {"emails": []},
            "isError": false
        }),
        executions: tool_calls.clone(),
        delay_ms: 0,
    };

    let response = run_persisted_workflow(
        RunWorkflowRequest {
            workflow_id: workflow.id.clone(),
            workflow_version: Some(1),
            preflight_mode: WorkflowPreflightMode::default(),
            inputs: HashMap::from([(
                "input".to_string(),
                InputBinding::Manual { value: json!({}) },
            )]),
            outputs: HashMap::new(),
        },
        &persistence,
        &model,
        &tools,
        &root.join("runs"),
        None,
        None,
    )
    .unwrap();

    assert_eq!(response.instance.status, ExecutionStatus::Completed);
    assert_eq!(response.execution_order, vec!["input", "read"]);
    assert_eq!(
        response.completion,
        Some(WorkflowCompletion {
            kind: WorkflowCompletionKind::EmptyCollection,
        })
    );
    assert_eq!(model_calls.load(AtomicOrdering::SeqCst), 0);
    assert_eq!(tool_calls.load(AtomicOrdering::SeqCst), 1);
    let reloaded = persistence
        .load_execution_instance(&response.instance.id)
        .unwrap();
    assert_eq!(reloaded.status, ExecutionStatus::Completed);
    assert_eq!(reloaded.output_payload, Some(completed_empty_envelope()));
    assert!(reloaded.error.is_none());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn malformed_or_out_of_bounds_collection_references_remain_failures() {
    let cases = [
        (
            json!({
                "content": [],
                "structuredContent": {},
                "isError": false
            }),
            0,
            "does not exist",
        ),
        (
            json!({
                "content": [],
                "structuredContent": {"emails": "not-an-array"},
                "isError": false
            }),
            0,
            "value is a string",
        ),
        (
            json!({
                "content": [],
                "structuredContent": {"emails": [{"subject": "only item"}]},
                "isError": false
            }),
            1,
            "out of bounds",
        ),
    ];

    for (mcp_result, index, expected_message) in cases {
        let (result, instance, model_calls, tool_calls, _) =
            execute_indexed_collection_fixture(mcp_result, index);
        let error = result.unwrap_err();
        assert!(
            error.message.contains(expected_message),
            "{}",
            error.message
        );
        assert_eq!(instance.status, ExecutionStatus::Failed);
        assert!(instance.output_payload.is_none());
        assert_eq!(model_calls, 1);
        assert_eq!(tool_calls, 1);
    }
}

#[test]
fn nonempty_collection_continues_through_the_normal_result_path() {
    let (result, instance, model_calls, tool_calls, _) = execute_indexed_collection_fixture(
        json!({
            "content": [],
            "structuredContent": {
                "emails": [{"subject": "Quarterly review"}]
            },
            "isError": false
        }),
        0,
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
    assert!(
        run_workflow_response(instance, outcome.execution_order, None)
            .completion
            .is_none()
    );
}

#[test]
fn conditional_node_branches_from_model_boolean_judgment() {
    let compiled = conditional_workflow();
    for (status, expected, skipped) in [
        ("green", "then_agent", "else_agent"),
        ("red", "else_agent", "then_agent"),
    ] {
        let request = RunWorkflowRequest {
            workflow_id: "conditional-workflow".to_string(),
            workflow_version: Some(1),
            preflight_mode: WorkflowPreflightMode::default(),
            inputs: HashMap::from([(
                "input".to_string(),
                InputBinding::Manual {
                    value: json!({"status": status}),
                },
            )]),
            outputs: HashMap::new(),
        };
        let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();
        let outcome = execute_workflow(
            &compiled,
            &request,
            &ConditionalFixtureModel,
            &NoExternalTools,
            &std::env::temp_dir(),
            &mut instance,
            &mut |_| Ok(()),
            &mut |_, _, _, _, _| {},
            None,
            None,
        )
        .unwrap();

        assert!(outcome.execution_order.contains(&expected.to_string()));
        assert!(!outcome.execution_order.contains(&skipped.to_string()));
        assert_eq!(instance.status, ExecutionStatus::Completed);
        assert!(instance
            .memory
            .get(&format!("nodes.{expected}.output"))
            .is_some());
        assert!(instance
            .memory
            .get(&format!("nodes.{skipped}.output"))
            .is_none());
    }
}

#[test]
fn loop_node_processes_each_item_with_local_item_binding() {
    let compiled = loop_workflow();
    let request = RunWorkflowRequest {
        workflow_id: "loop-workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!({"files": ["a.md", "b.md", "c.md"]}),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    let outcome = execute_workflow(
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
        outcome.execution_order,
        vec![
            "input".to_string(),
            "summarize".to_string(),
            "summarize".to_string(),
            "summarize".to_string(),
            "loop".to_string(),
            "output".to_string()
        ]
    );
    let summaries = instance.memory["nodes.summarize.output"]
        .as_array()
        .expect("loop aggregates body outputs");
    assert_eq!(summaries.len(), 3);
    assert_eq!(summaries[0]["data"], json!("handled:\"a.md\""));
    assert_eq!(summaries[1]["data"], json!("handled:\"b.md\""));
    assert_eq!(summaries[2]["data"], json!("handled:\"c.md\""));
    assert_eq!(instance.output_payload.as_ref().unwrap(), &json!(summaries));
}

#[test]
fn zero_item_loop_materializes_empty_body_aggregates_and_runs_done_branch() {
    let compiled = loop_workflow();
    let request = RunWorkflowRequest {
        workflow_id: "loop-workflow".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!({"files": []}),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    let outcome = execute_workflow(
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

    assert_eq!(outcome.execution_order, vec!["input", "loop", "output"]);
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert_eq!(instance.memory["nodes.summarize.output"], json!([]));
    assert_eq!(instance.node_payloads["summarize"].output, Some(json!([])));
    assert_eq!(instance.output_payload, Some(json!([])));
    assert!(
        run_workflow_response(instance, outcome.execution_order, None)
            .completion
            .is_none()
    );
}

#[test]
fn sprint_61_parallel_dag_interpolates_bindings_and_clears_active_node() {
    let compiled = sprint_61_parallel_workflow();
    let request = RunWorkflowRequest {
        workflow_id: "sprint-61-parallel".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!({"left": "alpha", "right": "beta"}),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    let outcome = execute_workflow(
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
        outcome.execution_order,
        vec![
            "input".to_string(),
            "agent_a".to_string(),
            "agent_b".to_string(),
            "output".to_string()
        ]
    );
    assert_eq!(instance.status, ExecutionStatus::Completed);
    assert_eq!(instance.active_node_id, None);
    assert_eq!(
        instance.output_payload.as_ref().unwrap(),
        &json!("handled:\"alpha\" + handled:\"beta\"")
    );
    assert_eq!(
        instance.memory["nodes.agent_a.output"]["data"],
        json!("handled:\"alpha\"")
    );
    assert_eq!(
        instance.memory["nodes.agent_b.output"]["data"],
        json!("handled:\"beta\"")
    );
}

#[test]
fn sprint_61_circular_workflow_fails_before_nodes_execute() {
    let mut compiled = compiled_workflow(false);
    compiled.workflow_ir.workflow_id = "sprint-61-cycle".to_string();
    compiled.workflow_ir.nodes.insert(
        2,
        WorkflowNode::Agent(AgentNode {
            id: "agent_b".to_string(),
            label: "Cycle peer".to_string(),
            objective: "Create a cycle peer.".to_string(),
            input_mappings: HashMap::from([(
                "context".to_string(),
                "{{nodes.agent.output}}".to_string(),
            )]),
            output_key: "nodes.agent_b.output".to_string(),
            system_timeout_ms: None,
        }),
    );
    if let WorkflowNode::Output(output) = compiled.workflow_ir.nodes.last_mut().unwrap() {
        output.input_mapping = "{{nodes.agent_b.output}}".to_string();
    }
    compiled.workflow_ir.edges = vec![
        WorkflowEdge {
            id: "e1".to_string(),
            source_node_id: "input".to_string(),
            source_port: "out".to_string(),
            target_node_id: "agent".to_string(),
            target_port: None,
        },
        WorkflowEdge {
            id: "e2".to_string(),
            source_node_id: "agent".to_string(),
            source_port: "out".to_string(),
            target_node_id: "agent_b".to_string(),
            target_port: None,
        },
        WorkflowEdge {
            id: "e3".to_string(),
            source_node_id: "agent_b".to_string(),
            source_port: "out".to_string(),
            target_node_id: "agent".to_string(),
            target_port: None,
        },
        WorkflowEdge {
            id: "e4".to_string(),
            source_node_id: "agent_b".to_string(),
            source_port: "out".to_string(),
            target_node_id: "output".to_string(),
            target_port: None,
        },
    ];
    compiled.instructions.insert(
        "agent_b".to_string(),
        instruction("sprint-61-cycle", "agent_b", "{{nodes.agent.output}}"),
    );
    let request = RunWorkflowRequest {
        workflow_id: "sprint-61-cycle".to_string(),
        workflow_version: Some(1),
        preflight_mode: WorkflowPreflightMode::default(),
        inputs: HashMap::from([(
            "input".to_string(),
            InputBinding::Manual {
                value: json!({"message": "cycle"}),
            },
        )]),
        outputs: HashMap::new(),
    };
    let mut instance = new_instance(&compiled.workflow_ir, &request).unwrap();

    let error = match execute_workflow(
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
    ) {
        Ok(_) => panic!("cycle should fail validation before execution"),
        Err(error) => error,
    };

    assert_eq!(error.code, "workflow_runtime_ir_invalid");
    assert!(error.message.contains("acyclic"));
    assert_eq!(instance.status, ExecutionStatus::Pending);
    assert_eq!(instance.active_node_id, None);
    assert!(instance.node_payloads.is_empty());
    assert!(instance.memory.is_empty());
}

#[test]
fn nested_template_references_resolve_payload_data() {
    let memory = HashMap::from([
        (
            "nodes.summary.output".to_string(),
            json!({
                "mediaType": "text/plain",
                "data": "Approved executive summary",
                "assetPath": null,
                "metadata": {}
            }),
        ),
        (
            "summary.output".to_string(),
            json!({
                "mediaType": "text/plain",
                "data": "Approved shorthand summary",
                "assetPath": null,
                "metadata": {}
            }),
        ),
    ]);

    assert_eq!(
        resolve_template("{{nodes.summary.output.data}}", &memory).unwrap(),
        json!("Approved executive summary")
    );
    assert_eq!(
        resolve_template("{{summary.output.data}}", &memory).unwrap(),
        json!("Approved shorthand summary")
    );
    assert_eq!(
        resolve_template("Report: {{nodes.summary.output.data}}", &memory).unwrap(),
        json!("Report: Approved executive summary")
    );
}

#[test]
fn explicit_empty_output_requires_the_declared_primary_collection() {
    let compiled = nested_contact_collection_workflow();
    let output = OutputNode {
        id: "empty-output".to_string(),
        label: "Nothing found".to_string(),
        input_mapping: "  {{nodes.read.output.data.structuredContent.contacts}}  ".to_string(),
        output_schema: json!({"type": "array"}),
        completion_kind: WorkflowCompletionKind::EmptyCollection,
    };
    let empty = execute_output(
        &output,
        None,
        &HashMap::from([(
            "nodes.read.output".to_string(),
            json!({
                "data": {
                    "structuredContent": {"contacts": []}
                }
            }),
        )]),
        "instance",
        &compiled.workflow_ir,
    )
    .unwrap();
    assert_eq!(empty.output, completed_empty_envelope());
    assert_eq!(
        empty.completion_kind,
        WorkflowCompletionKind::EmptyCollection
    );

    let error = execute_output(
        &output,
        None,
        &HashMap::from([(
            "nodes.read.output".to_string(),
            json!({
                "data": {
                    "structuredContent": {"contacts": [{"id": 1}]}
                }
            }),
        )]),
        "instance",
        &compiled.workflow_ir,
    )
    .unwrap_err();
    assert!(error.message.contains("did not resolve to an empty array"));

    let nested_output = OutputNode {
        input_mapping: "{{nodes.read.output.data.structuredContent.contacts.0.emails}}".to_string(),
        ..output
    };
    let nested_error = execute_output(
        &nested_output,
        None,
        &HashMap::from([(
            "nodes.read.output".to_string(),
            json!({
                "data": {
                    "structuredContent": {
                        "contacts": [{"name": "Ada", "emails": []}]
                    }
                }
            }),
        )]),
        "instance",
        &compiled.workflow_ir,
    )
    .unwrap_err();
    assert!(nested_error
        .message
        .contains("declares data.structuredContent.contacts"));
}

#[test]
fn consolidated_scenario_templates_resolve_to_exact_runtime_types() {
    let memory = HashMap::from([
        (
            "nodes.brief.output".to_string(),
            json!({
                "mediaType": "text/plain",
                "data": "Evidence-bound operations brief",
                "assetPath": null,
                "metadata": {}
            }),
        ),
        (
            "nodes.write-report.output".to_string(),
            json!({
                "mediaType": "application/json",
                "data": {
                    "path": "/approved/ship_test_06/supplier_exception.md",
                    "structuredContent": {
                        "path": "/approved/ship_test_06/supplier_exception.md"
                    }
                },
                "assetPath": null,
                "metadata": {}
            }),
        ),
    ]);
    let create_file = resolve_json_templates(
        &json!({"file":{"content":"{{nodes.brief.output.data}}"}}),
        &memory,
    )
    .expect("Scenario five create_file payload resolves");
    assert_eq!(
        create_file.pointer("/file/content"),
        Some(&json!("Evidence-bound operations brief"))
    );
    assert!(create_file
        .pointer("/file/content")
        .is_some_and(Value::is_string));

    let calendar = resolve_json_templates(
        &json!({"notes":"Report: {{nodes.write-report.output.data.structuredContent.path}}"}),
        &memory,
    )
    .expect("Scenario six Calendar link resolves");
    let email = resolve_json_templates(
        &json!({"body":"Report: {{nodes.write-report.output.data.structuredContent.path}}"}),
        &memory,
    )
    .expect("Scenario six email link resolves");
    assert_eq!(
        calendar["notes"],
        json!("Report: /approved/ship_test_06/supplier_exception.md")
    );
    assert_eq!(email["body"], calendar["notes"]);

    let legacy = resolve_json_templates(
        &json!({"body":"Report: {{nodes.write-report.output.data.path}}"}),
        &memory,
    )
    .expect("legacy direct create_file path remains resolvable");
    assert_eq!(legacy["body"], calendar["notes"]);
}
