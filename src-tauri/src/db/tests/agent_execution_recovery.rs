use super::*;

#[tokio::test]
async fn invocation_boundary_transition_is_atomic_and_single_use() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu-action-invocation-boundary-{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO plan_generation_states
             (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms)
             VALUES ('plan-invocation','{\"steps\":[1]}',0,'running','running',1)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
             VALUES ('plan-invocation','fixture','{}',NULL,'prepared_effectful',1)",
            [],
        )
        .unwrap();
    let action_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
             VALUES ('plan-invocation','fixture','{}','blocked','prepared_effectful',1)",
            [],
        )
        .unwrap();
    let blocked_action_id = connection.last_insert_rowid();
    drop(connection);

    assert!(engine
        .commit_completed_agent_action_checkpoint(
            action_id,
            "plan-invocation",
            r#"{"steps":[1]}"#,
            1,
            r#"{"status":"completed","verified":true}"#,
            "Completed.",
        )
        .is_err());
    engine
        .mark_agent_action_invocation_started(
            action_id,
            "prepared_effectful".to_string(),
            "started_effectful".to_string(),
        )
        .await
        .unwrap();
    assert!(engine
        .mark_agent_action_invocation_started(
            action_id,
            "prepared_effectful".to_string(),
            "started_effectful".to_string(),
        )
        .await
        .is_err());
    assert!(engine
        .mark_agent_action_invocation_started(
            blocked_action_id,
            "prepared_effectful".to_string(),
            "started_effectful".to_string(),
        )
        .await
        .is_err());

    let connection = engine.open_connection().unwrap();
    let statuses = [action_id, blocked_action_id].map(|id| {
        connection
            .query_row(
                "SELECT status FROM actions WHERE id=?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
    });
    assert_eq!(statuses, ["started_effectful", "prepared_effectful"]);
    drop(connection);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn sqlite_recovery_evidence_requires_an_exact_typed_unchanged_receipt() {
    crate::tools::task_tool_runtime::register_decision_pack_recovery_test_fixture();
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-action-unchanged-receipt-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let normalized = crate::tools::task_tool_runtime::normalize_agent_error(
        "create_decision_pack",
        r#"{"taskToolError":{"code":"decision_pack_research_evidence_unavailable","message":"Research evidence was unavailable.","context":{"changedState":false}}}"#,
    );
    let valid_receipt = json!({
        "schema": crate::agentic_loop::recovery::ACTION_FAILURE_RECEIPT_SCHEMA,
        "operation": "create_decision_pack",
        "status": "failed",
        "verified": true,
        "changedState": "none",
        "potentiallyEffectful": true,
        "agentError": normalized,
    });
    let cases = [
        ("valid", Some(valid_receipt.clone()), false),
        ("missing", None, true),
        (
            "wrong-schema",
            Some({
                let mut value = valid_receipt.clone();
                value["schema"] = json!("unknown");
                value
            }),
            true,
        ),
        (
            "unverified",
            Some({
                let mut value = valid_receipt.clone();
                value["verified"] = json!(false);
                value
            }),
            true,
        ),
        (
            "effect-mismatch",
            Some({
                let mut value = valid_receipt.clone();
                value["potentiallyEffectful"] = json!(false);
                value
            }),
            true,
        ),
    ];
    for (suffix, receipt, expected_uncertain) in cases {
        let plan_id = format!("plan-{suffix}");
        let connection = engine.open_connection().unwrap();
        connection
            .execute(
                "INSERT INTO plan_generation_states
                 (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms)
                 VALUES (?1,'{}',0,'running','running',1)",
                params![plan_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
                 VALUES (?1,'create_decision_pack','{}',?2,?3,1)",
                params![
                    plan_id,
                    receipt.map(|value| value.to_string()),
                    crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_EFFECTFUL,
                ],
            )
            .unwrap();
        drop(connection);
        assert_eq!(
            engine.has_uncertain_agent_action_effect(&plan_id).unwrap(),
            expected_uncertain,
            "{suffix}"
        );
    }
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn resume_gate_requires_a_safe_invocation_state_and_matching_checkpoint() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-agent-resume-adversarial-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let verified_output = r#"{"status":"completed","verified":true}"#;

    for (suffix, recovery_action, action_status, action_output, checkpoint, expected) in [
        (
            "empty",
            Some("resume_same_execution"),
            None,
            None,
            None,
            true,
        ),
        (
            "prepared",
            Some("resume_same_execution"),
            Some("prepared_effectful"),
            None,
            Some(0),
            true,
        ),
        (
            "read-started",
            Some("resume_same_execution"),
            Some("started_read_only"),
            None,
            Some(0),
            true,
        ),
        (
            "read-unverified",
            Some("resume_same_execution"),
            Some("unverified_read_only"),
            Some("{}"),
            Some(0),
            true,
        ),
        (
            "read-sensor",
            Some("resume_same_execution"),
            Some("sensor_captured_read_only"),
            Some("{}"),
            Some(0),
            true,
        ),
        (
            "effect-started",
            Some("resume_same_execution"),
            Some("started_effectful"),
            None,
            Some(0),
            false,
        ),
        (
            "effect-unverified",
            Some("resume_same_execution"),
            Some("unverified_effectful"),
            Some("{}"),
            Some(0),
            false,
        ),
        (
            "effect-sensor",
            Some("resume_same_execution"),
            Some("sensor_captured_effectful"),
            Some("{}"),
            Some(0),
            false,
        ),
        ("legacy-empty", None, None, None, None, true),
        (
            "legacy-prepared",
            None,
            Some("prepared_effectful"),
            None,
            Some(0),
            false,
        ),
        (
            "completed",
            Some("resume_same_execution"),
            Some("completed"),
            Some(verified_output),
            Some(1),
            true,
        ),
        (
            "legacy-completed",
            None,
            Some("completed"),
            Some(verified_output),
            Some(1),
            true,
        ),
        (
            "invalid-completed",
            Some("resume_same_execution"),
            Some("completed"),
            Some("{}"),
            Some(1),
            false,
        ),
        (
            "stale-checkpoint",
            Some("resume_same_execution"),
            Some("completed"),
            Some(verified_output),
            Some(0),
            false,
        ),
        (
            "prepared-output",
            Some("resume_same_execution"),
            Some("prepared_effectful"),
            Some("{}"),
            Some(0),
            true,
        ),
        (
            "missing-checkpoint",
            Some("resume_same_execution"),
            Some("prepared_effectful"),
            None,
            None,
            false,
        ),
    ] {
        let execution_id = format!("execution-{suffix}");
        let plan_id = format!("plan-{suffix}");
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: format!("agent-{suffix}"),
                provider_id: "local_model".to_string(),
                model_id: "gemma-test".to_string(),
                title: Some(format!("Resume {suffix}")),
                dynamic_routing_override: None,
                workspace_id: Some(engine.workspace_id.clone()),
            })
            .unwrap();
        let context = ChatTurnPersistenceContext {
            turn_id: format!("turn-{suffix}"),
            generation_token: format!("generation-{suffix}"),
            session_id: session.id,
            agent_id: format!("agent-{suffix}"),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            parent_turn_id: None,
            root_turn_id: format!("turn-{suffix}"),
            turn_kind: "root".to_string(),
        };
        let context_json = json!({"case": suffix}).to_string();
        engine.begin_chat_turn(&context).unwrap();
        engine.finish_chat_turn(&context, "completed").unwrap();
        engine
            .begin_agent_execution(&execution_id, &plan_id, &context, &context_json)
            .unwrap();

        let connection = engine.open_connection().unwrap();
        if let Some(checkpoint) = checkpoint {
            connection
                .execute(
                    "INSERT INTO plan_generation_states
                     (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms)
                     VALUES (?1,'{}',?2,'running','running',1)",
                    params![plan_id, checkpoint],
                )
                .unwrap();
        }
        if let Some(action_status) = action_status {
            connection
                .execute(
                    "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
                     VALUES (?1,'fixture','{}',?2,?3,1)",
                    params![plan_id, action_output, action_status],
                )
                .unwrap();
        }
        drop(connection);

        let mut receipt = json!({
            "schema": "oomu.agent_execution_recovery.v1",
            "executionId": execution_id,
            "planId": plan_id,
            "code": "agent_execution_interrupted",
            "boundary": "AgentExecution",
            "recoverable": true,
            "message": "Execution stopped at a durable boundary.",
            "context": {},
            "changedState": if checkpoint.unwrap_or(0) > 0 {
                "checkpoint_saved"
            } else {
                "none"
            },
        });
        if let Some(recovery_action) = recovery_action {
            receipt
                .as_object_mut()
                .unwrap()
                .insert("recoveryAction".to_string(), json!(recovery_action));
        }
        let receipt = receipt.to_string();
        engine
            .finalize_agent_execution(
                &execution_id,
                &plan_id,
                &context,
                &context_json,
                "halted",
                Some(&receipt),
                "error",
                "halted",
                "Execution stopped at a durable boundary.",
                Some(&receipt),
            )
            .unwrap();

        let loaded = engine.load_resumable_agent_execution_request(&execution_id);
        let resumed =
            engine.resume_agent_execution(&execution_id, &plan_id, &context, &context_json);
        if expected {
            assert_eq!(loaded.unwrap(), context_json, "{suffix}");
            assert!(resumed.is_ok(), "{suffix}: {resumed:?}");
        } else {
            assert!(loaded.is_err(), "{suffix}");
            assert!(resumed.is_err(), "{suffix}");
        }
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

type CalendarRecoveryFixture = (
    String,
    ChatTurnPersistenceContext,
    crate::agentic_loop::AgentPlanExecutionRequest,
);

fn calendar_recovery_fixture(engine: &PersistenceEngine, suffix: &str) -> CalendarRecoveryFixture {
    let _ = crate::tools::system_calendar_event::register_task_tool();
    let specialized_operation = matches!(
        suffix,
        "specialized-operation" | "specialized-durable-followup"
    );
    let durable_followup = suffix == "specialized-durable-followup";
    if specialized_operation {
        let _ = crate::tools::release_recovery::register_task_tools();
    }
    let calendar_operation = if specialized_operation {
        "create_release_recovery_calendar_event"
    } else {
        "create_conflict_free_calendar_event"
    };
    let calendar_error_code = if specialized_operation {
        "calendar_action_denied"
    } else {
        "calendar_not_found"
    };
    let requested_calendar_name = if durable_followup {
        "OOMU Test Denial"
    } else {
        "OOMU Test"
    };
    let available_calendar_names = if durable_followup {
        json!(["Family", "OOMU Test"])
    } else {
        json!(["Personal", "Work"])
    };
    let specialized_agenda = specialized_operation.then(|| {
        let fixture_root = std::path::PathBuf::from(engine.db_path())
            .parent()
            .unwrap()
            .to_path_buf();
        let input_path = fixture_root.join("release_milestones.json");
        let output_path = fixture_root.join("oomu-release-recovery-agenda.md");
        let input_bytes = br#"[{"milestoneId":"M1","name":"Ship","targetDate":"2026-07-20","status":"PENDING","owner":"OOMU"}]"#;
        let output_bytes = "# OOMU Release Readiness Recovery Agenda\n\n**Status date:** 2026-07-20  \n**Proposed time:** Tuesday, July 21, 2026 at 1:00 PM–1:30 PM (America/New_York)  \n**Frozen start:** `2026-07-21T13:00:00-04:00`  \n**Frozen end:** `2026-07-21T13:30:00-04:00`  \n**Calendar timezone:** `America/New_York`  \n**Duration:** 30 minutes  \n**Availability:** Tentative\n\n## Milestone facts\n\n| ID | Milestone | Target date | Status | Owner | Recovery assessment |\n|---|---|---|---|---|---|\n\n\n## Decisions needed\n\n\n\n## Agenda — exactly five items\n\n1. One\n2. Two\n3. Three\n4. Four\n5. Five\n";
        std::fs::write(&input_path, input_bytes).unwrap();
        std::fs::write(&output_path, output_bytes.as_bytes()).unwrap();
        let input_path = std::fs::canonicalize(input_path).unwrap();
        let output_path = std::fs::canonicalize(output_path).unwrap();
        (
            input_path,
            crate::foundation::digest::sha256_hex(input_bytes),
            output_path,
            crate::foundation::digest::sha256_hex(output_bytes.as_bytes()),
            output_bytes.len(),
        )
    });
    let execution_id = format!("execution-calendar-{suffix}");
    let plan_id = format!("plan-calendar-{suffix}");
    let agent_id = format!("agent-calendar-{suffix}");
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: agent_id.clone(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some(format!("Calendar recovery {suffix}")),
            dynamic_routing_override: None,
            workspace_id: Some(engine.workspace_id.clone()),
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: format!("turn-calendar-{suffix}"),
        generation_token: format!("generation-calendar-{suffix}"),
        session_id: session.id.clone(),
        agent_id: agent_id.clone(),
        provider_id: "local_model".to_string(),
        model_id: "gemma-test".to_string(),
        parent_turn_id: None,
        root_turn_id: format!("turn-calendar-{suffix}"),
        turn_kind: "root".to_string(),
    };
    let calendar_arguments = if specialized_operation {
        json!({
            "calendarName": requested_calendar_name,
            "title": "Supplier Decision Review",
            "agendaStep": 0,
            "availability": "tentative"
        })
    } else {
        json!({
            "calendarName": requested_calendar_name,
            "title": "Supplier Decision Review",
            "day": "next_weekday",
            "windowStartLocal": "13:00",
            "windowEndLocal": "16:00",
            "durationMinutes": 30,
            "location": "",
            "notes": "Decision pack ready",
            "availability": "tentative"
        })
    };
    let request = crate::agentic_loop::AgentPlanExecutionRequest {
        plan: crate::agentic_loop::ActionPlan {
            id: plan_id.clone(),
            objective: "Prepare the supplier decision pack".to_string(),
            intent: crate::gemma::StructuredIntent {
                objective: "Prepare the supplier decision pack".to_string(),
                category: crate::gemma::IntentCategory::ProjectAnalysis,
                source: crate::gemma::IntentSource::Deterministic,
                degraded_reason: None,
            },
            steps: vec![
                crate::agentic_loop::Step {
                    step: "Create the verified decision pack".to_string(),
                    tool: crate::agentic_loop::Tool::FileRead {
                        path: "/tmp/verified-decision-pack".to_string(),
                    },
                    risk_level: crate::agentic_loop::RiskLevel::Low,
                },
                crate::agentic_loop::Step {
                    step: "Create the conflict-free Calendar event".to_string(),
                    tool: crate::agentic_loop::Tool::RegisteredTaskTool(
                        crate::tools::task_tool_runtime::PlannedTaskToolRequest::new(
                            calendar_operation,
                            calendar_arguments,
                        ),
                    ),
                    risk_level: crate::agentic_loop::RiskLevel::High,
                },
                crate::agentic_loop::Step {
                    step: "Prepare the Mail draft".to_string(),
                    tool: crate::agentic_loop::Tool::FileRead {
                        path: "/tmp/mail-draft".to_string(),
                    },
                    risk_level: crate::agentic_loop::RiskLevel::Low,
                },
            ],
            exit_condition: "All requested outputs are verified".to_string(),
            logical_certificate: crate::shield_gate::LogicalCertificate::unsigned(
                vec!["fixture".to_string()],
                vec!["calendar recovery".to_string()],
                "resume only the pending step".to_string(),
            ),
            trusted_automatic_execution: false,
            model_route: crate::agentic_loop::ModelRouteDecision {
                selected_model: crate::shield_gate::ModelMetadata {
                    name: "Gemma".to_string(),
                    version: "test".to_string(),
                    provider: "local".to_string(),
                    locality: "local".to_string(),
                },
                provider_config_id: None,
                provider_id: Some("local_model".to_string()),
                recommended_model: None,
                requires_principal_authorization: false,
                reason: "fixture".to_string(),
                context_excerpt_count: 0,
                context_sources: Vec::new(),
            },
            parent_artifact_hashes: Vec::new(),
        },
        turn_context: crate::agentic_loop::AgentPlanExecutionTurnContext {
            turn_id: context.turn_id.clone(),
            generation_token: context.generation_token.clone(),
            session_id: context.session_id.clone(),
            agent_id: context.agent_id.clone(),
            project_id: None,
            provider_id: context.provider_id.clone(),
            model_id: context.model_id.clone(),
            parent_turn_id: None,
            root_turn_id: context.root_turn_id.clone(),
            turn_kind: context.turn_kind.clone(),
            reasoning: None,
            context_budget: None,
            primary_route_id: None,
            fallback_route_id: None,
            dynamic_routing_enabled: false,
            automated_web_grounding_enabled: true,
            attachment_grants: Vec::new(),
            created_at_ms: 1,
        },
        principal_approved: true,
        authority_proof_id: None,
    };
    let context_json = serde_json::to_string(&request).unwrap();
    let plan_json = serde_json::to_string(&request.plan).unwrap();
    engine.begin_chat_turn(&context).unwrap();
    engine.finish_chat_turn(&context, "completed").unwrap();
    engine
        .begin_agent_execution(&execution_id, &plan_id, &context, &context_json)
        .unwrap();
    let normalized = crate::tools::task_tool_runtime::normalize_agent_error(
        calendar_operation,
        &json!({
            "taskToolError": {
                "code": calendar_error_code,
                "message": "The exact requested calendar action did not change Calendar.",
                "context": {
                    "requestedCalendarName": requested_calendar_name,
                    "availableCalendarNames": available_calendar_names,
                    "changedState": false
                }
            }
        })
        .to_string(),
    );
    let failure_receipt = json!({
        "schema": crate::agentic_loop::recovery::ACTION_FAILURE_RECEIPT_SCHEMA,
        "operation": calendar_operation,
        "status": "failed",
        "verified": true,
        "changedState": "none",
        "potentiallyEffectful": true,
        "agentError": normalized,
    })
    .to_string();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO plan_generation_states
             (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms)
             VALUES (?1,?2,1,'checkpointed','decision pack complete',1)",
            params![plan_id, plan_json],
        )
        .unwrap();
    let completed_output = if specialized_operation {
        let proposed_time = "Tuesday, July 21, 2026 at 1:00 PM–1:30 PM (America/New_York)";
        let event_notes = format!(
            "OOMU Release Readiness recovery meeting\n\nProposed time: {proposed_time}\n\nAgenda:\n1. One\n2. Two\n3. Three\n4. Four\n5. Five"
        );
        let (input_path, input_sha256, output_path, output_sha256, byte_length) =
            specialized_agenda.as_ref().unwrap();
        let mail_body = format!(
            "OOMU release readiness recovery meeting\n\nProposed time: {proposed_time}\n\n1. One\n2. Two\n3. Three\n4. Four\n5. Five\n\nAgenda file: {}",
            output_path.display()
        );
        let agenda_receipt = json!({
            "status": "completed",
            "verified": true,
            "inputPath": input_path,
            "inputSha256": input_sha256,
            "outputPath": output_path,
            "outputSha256": output_sha256,
            "byteLength": byte_length,
            "asOfDate": "2026-07-20",
            "startDate": "2026-07-21T13:00:00-04:00",
            "endDate": "2026-07-21T13:30:00-04:00",
            "timeZone": "America/New_York",
            "proposedTime": proposed_time,
            "agendaItems": ["One", "Two", "Three", "Four", "Five"],
            "milestoneFacts": [],
            "eventNotes": event_notes.clone(),
            "mailBody": mail_body
        });
        serde_json::to_string(&crate::shield_gate::ExecuteCommandResponse {
            operation: "prepare_release_recovery_agenda".to_string(),
            status: crate::shield_gate::CommandStatus::Completed,
            message: agenda_receipt.to_string(),
            metrics: None,
            claims: vec![format!(
                "notes_sha256={}",
                crate::foundation::digest::sha256_hex(event_notes.as_bytes())
            )],
            verified: true,
            model_used: None,
        })
        .unwrap()
    } else {
        r#"{"status":"completed","verified":true}"#.to_string()
    };
    connection
        .execute(
            "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
             VALUES (?1,'create_decision_pack','{}',?2,'completed',1)",
            params![plan_id, completed_output],
        )
        .unwrap();
    if calendar_error_code != "calendar_action_denied" {
        connection
            .execute(
                "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
                 VALUES (?1,?2,'{}',?3,?4,2)",
                params![
                    plan_id,
                    calendar_operation,
                    failure_receipt,
                    crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_EFFECTFUL,
                ],
            )
            .unwrap();
    }
    drop(connection);
    let denied_arguments_sha256 = (calendar_error_code == "calendar_action_denied").then(|| {
        let (_, _, output_path, output_sha256, byte_length) =
            specialized_agenda.as_ref().unwrap();
        let resolved_arguments = json!({
            "calendarName": requested_calendar_name,
            "title": "Supplier Decision Review",
            "startDate": "2026-07-21T13:00:00-04:00",
            "endDate": "2026-07-21T13:30:00-04:00",
            "location": "",
            "notes": "OOMU Release Readiness recovery meeting\n\nProposed time: Tuesday, July 21, 2026 at 1:00 PM–1:30 PM (America/New_York)\n\nAgenda:\n1. One\n2. Two\n3. Three\n4. Four\n5. Five",
            "availability": "tentative",
            "agendaStep": 0,
            "agendaSha256": output_sha256,
            "outputPath": output_path,
            "outputSha256": output_sha256,
            "byteLength": byte_length
        });
        crate::foundation::digest::sha256_hex(
            serde_json::to_string(&resolved_arguments)
                .unwrap()
                .as_bytes(),
        )
    });
    let mut recovery_context = json!({
        "requestedCalendarName": requested_calendar_name,
        "availableCalendarNames": available_calendar_names
    });
    if let Some(digest) = denied_arguments_sha256 {
        recovery_context["calendarStepArgumentsSha256"] = json!(digest);
    }
    let recovery_receipt = json!({
        "schema": crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA,
        "executionId": execution_id,
        "planId": plan_id,
        "code": calendar_error_code,
        "boundary": "Calendar",
        "recoverable": true,
        "recoveryAction": "resolve_calendar_target",
        "message": "The exact requested calendar was not found.",
        "context": recovery_context,
        "changedState": "checkpoint_saved"
    })
    .to_string();
    engine
        .finalize_agent_execution(
            &execution_id,
            &plan_id,
            &context,
            &context_json,
            "halted",
            Some(&recovery_receipt),
            "error",
            "halted",
            "Calendar target needs a choice.",
            Some(&recovery_receipt),
        )
        .unwrap();
    (execution_id, context, request)
}

#[test]
fn specialized_release_recovery_calendar_step_is_eligible_for_narrow_retargeting() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu-specialized-calendar-recovery-{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let (execution_id, context, _) = calendar_recovery_fixture(&engine, "specialized-operation");
    let (_, step_index, requested, available) = engine
        .load_calendar_recovery_execution_request(&execution_id, &context.session_id)
        .unwrap();
    assert_eq!(step_index, 1);
    assert_eq!(requested, "OOMU Test");
    assert_eq!(available, ["Personal", "Work"]);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn accepted_calendar_followup_durably_retargets_the_denied_scenario_step() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-calendar-durable-followup-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let (execution_id, origin, mut request) =
        calendar_recovery_fixture(&engine, "specialized-durable-followup");

    engine
        .accept_chat_turn(AcceptChatTurnRequest {
            turn_id: "turn-calendar-durable-followup-choice".to_string(),
            generation_token: "generation-calendar-durable-followup-choice".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-calendar-durable-followup-choice".to_string(),
            turn_kind: "root".to_string(),
            session_id: origin.session_id.clone(),
            agent_id: origin.agent_id.clone(),
            provider_id: origin.provider_id.clone(),
            model_id: origin.model_id.clone(),
            message: "Use my OOMU Test calendar instead and continue.".to_string(),
        })
        .unwrap();

    let (stored_context, step_index, requested, available) = engine
        .load_calendar_recovery_execution_request(&execution_id, &origin.session_id)
        .expect("an accepted follow-up must not invalidate the saved Calendar recovery");
    assert_eq!(step_index, 1);
    assert_eq!(requested, "OOMU Test Denial");
    assert_eq!(available, ["Family", "OOMU Test"]);

    let crate::agentic_loop::Tool::RegisteredTaskTool(calendar) =
        &mut request.plan.steps[step_index].tool
    else {
        panic!("calendar step missing")
    };
    calendar.arguments["calendarName"] = json!("OOMU Test");
    let resolved_context = serde_json::to_string(&request).unwrap();
    let resolved_plan = serde_json::to_string(&request.plan).unwrap();
    let resolution = json!({
        "schema": crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA,
        "executionId": execution_id,
        "planId": request.plan.id,
        "code": "calendar_target_resolved",
        "boundary": "CalendarRecovery",
        "recoverable": true,
        "recoveryAction": "resume_same_execution",
        "message": "The calendar target was resolved by the user.",
        "context": {
            "requestedCalendarName": "OOMU Test Denial",
            "selectedCalendarName": "OOMU Test",
            "resolution": "selected_existing"
        },
        "changedState": "checkpoint_saved"
    })
    .to_string();

    engine
        .commit_agent_calendar_recovery_resolution(
            &execution_id,
            &origin.session_id,
            &stored_context,
            &resolved_context,
            &resolved_plan,
            &resolution,
        )
        .expect("the exact accepted correction must commit before execution resumes");

    let messages = engine.select_chat_messages(&origin.session_id).unwrap();
    assert!(messages.iter().any(|message| {
        message.role == "user"
            && message.content == "Use my OOMU Test calendar instead and continue."
    }));
    assert!(messages.iter().any(|message| {
        message.role == "assistant"
            && serde_json::from_str::<Value>(&message.content)
                .ok()
                .and_then(|receipt| receipt.get("code").cloned())
                == Some(json!("calendar_target_resolved"))
    }));
    assert_eq!(
        engine
            .load_resumable_agent_execution_request(&execution_id)
            .unwrap(),
        resolved_context
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn calendar_denial_digest_mismatch_rejects_load_and_retarget() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-calendar-denial-binding-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let (execution_id, context, request) =
        calendar_recovery_fixture(&engine, "specialized-operation");
    let (stored_context, step_index, _, _) = engine
        .load_calendar_recovery_execution_request(&execution_id, &context.session_id)
        .unwrap();
    assert_eq!(step_index, 1);

    let connection = engine.open_connection().unwrap();
    let original: String = connection
        .query_row(
            "SELECT payload_json FROM agent_execution_logs
             WHERE execution_id=?1 AND payload_json IS NOT NULL
             ORDER BY id DESC LIMIT 1",
            params![execution_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut tampered = serde_json::from_str::<Value>(&original).unwrap();
    tampered["context"]["calendarStepArgumentsSha256"] = json!("0".repeat(64));
    let tampered = tampered.to_string();
    connection
        .execute(
            "UPDATE agent_execution_logs SET payload_json=?1
             WHERE execution_id=?2 AND payload_json=?3",
            params![tampered, execution_id, original],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE chat_messages SET content=?1,metadata_json=?1
             WHERE session_id=?2 AND content=?3 AND metadata_json=?3",
            params![tampered, context.session_id, original],
        )
        .unwrap();
    drop(connection);

    assert!(engine
        .load_calendar_recovery_execution_request(&execution_id, &context.session_id)
        .is_err());
    let mut resolved = request;
    let crate::agentic_loop::Tool::RegisteredTaskTool(calendar) =
        &mut resolved.plan.steps[step_index].tool
    else {
        panic!("calendar step missing")
    };
    calendar.arguments["calendarName"] = json!("Personal");
    let resolved_context = serde_json::to_string(&resolved).unwrap();
    let resolved_plan = serde_json::to_string(&resolved.plan).unwrap();
    let resolution = json!({
        "schema": crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA,
        "executionId": execution_id,
        "planId": resolved.plan.id,
        "code": "calendar_target_resolved",
        "boundary": "CalendarRecovery",
        "recoverable": true,
        "recoveryAction": "resume_same_execution",
        "message": "The calendar target was resolved by the user.",
        "context": {
            "requestedCalendarName": "OOMU Test",
            "selectedCalendarName": "Personal",
            "resolution": "selected_existing"
        },
        "changedState": "checkpoint_saved"
    })
    .to_string();
    assert!(engine
        .commit_agent_calendar_recovery_resolution(
            &execution_id,
            &context.session_id,
            &stored_context,
            &resolved_context,
            &resolved_plan,
            &resolution,
        )
        .is_err());
    let connection = engine.open_connection().unwrap();
    let (status, stored_plan): (String, String) = connection
        .query_row(
            "SELECT executions.status,state.plan_json
             FROM agent_executions executions
             JOIN plan_generation_states state ON state.plan_id=executions.plan_id
             WHERE executions.execution_id=?1",
            params![execution_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "halted");
    assert_ne!(stored_plan, resolved_plan);
    drop(connection);
    let _ = std::fs::remove_dir_all(temp_dir);
}

fn replace_calendar_target_receipt_with_permission_receipt(
    engine: &PersistenceEngine,
    execution_id: &str,
    context: &ChatTurnPersistenceContext,
    plan_id: &str,
) {
    let connection = engine.open_connection().unwrap();
    let previous: String = connection
        .query_row(
            "SELECT payload_json FROM agent_execution_logs
             WHERE execution_id=?1 AND payload_json IS NOT NULL
             ORDER BY id DESC LIMIT 1",
            params![execution_id],
            |row| row.get(0),
        )
        .unwrap();
    let permission_receipt = json!({
        "schema": crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA,
        "executionId": execution_id,
        "planId": plan_id,
        "code": "calendar_permission_denied",
        "boundary": "CreateConflictFreeCalendarEvent",
        "recoverable": true,
        "recoveryAction": "resume_same_execution",
        "message": "Calendar Full Access is required.",
        "context": {},
        "changedState": "checkpoint_saved"
    })
    .to_string();
    connection
        .execute(
            "UPDATE agent_execution_logs SET payload_json=?1,message=?2
             WHERE id=(SELECT id FROM agent_execution_logs
                       WHERE execution_id=?3 AND payload_json=?4
                       ORDER BY id DESC LIMIT 1)",
            params![
                permission_receipt,
                "Calendar Full Access is required.",
                execution_id,
                previous,
            ],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE chat_messages SET content=?1,metadata_json=?1
             WHERE session_id=?2 AND content=?3 AND metadata_json=?3",
            params![permission_receipt, context.session_id, previous],
        )
        .unwrap();
}

#[test]
fn calendar_resolution_preserves_checkpoint_and_reexecutes_only_pending_step() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu-calendar-recovery-checkpoint-{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();

    for (suffix, selected) in [("select", "Personal"), ("create", "OOMU Test")] {
        let (execution_id, context, request) = calendar_recovery_fixture(&engine, suffix);
        let (stored_context, step_index, requested, available) = engine
            .load_calendar_recovery_execution_request(&execution_id, &context.session_id)
            .unwrap();
        assert_eq!(step_index, 1);
        assert_eq!(requested, "OOMU Test");
        assert_eq!(available, ["Personal", "Work"]);
        let mut resolved = request.clone();
        let crate::agentic_loop::Tool::RegisteredTaskTool(calendar) =
            &mut resolved.plan.steps[step_index].tool
        else {
            panic!("calendar step missing")
        };
        calendar.arguments["calendarName"] = json!(selected);
        let resolved_context = serde_json::to_string(&resolved).unwrap();
        let resolved_plan = serde_json::to_string(&resolved.plan).unwrap();
        let resolution = json!({
            "schema": crate::agentic_loop::recovery::RECOVERY_RECEIPT_SCHEMA,
            "executionId": execution_id,
            "planId": resolved.plan.id,
            "code": "calendar_target_resolved",
            "boundary": "CalendarRecovery",
            "recoverable": true,
            "recoveryAction": "resume_same_execution",
            "message": "The calendar target was resolved by the user.",
            "context": {
                "requestedCalendarName": "OOMU Test",
                "selectedCalendarName": selected,
                "resolution": if selected == "OOMU Test" { "created_requested" } else { "selected_existing" }
            },
            "changedState": "checkpoint_saved"
        })
        .to_string();
        engine
            .commit_agent_calendar_recovery_resolution(
                &execution_id,
                &context.session_id,
                &stored_context,
                &resolved_context,
                &resolved_plan,
                &resolution,
            )
            .unwrap();
        let connection = engine.open_connection().unwrap();
        let mut statement = connection
            .prepare(
                "SELECT content FROM chat_messages WHERE session_id=?1 AND role='assistant'
                 ORDER BY id ASC",
            )
            .unwrap();
        let durable_cards = statement
            .query_map(params![context.session_id], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(durable_cards.len(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(&durable_cards[0]).unwrap()["recoveryAction"],
            "resolve_calendar_target",
            "the original denial or target failure remains immutable"
        );
        assert_eq!(
            serde_json::from_str::<Value>(&durable_cards[1]).unwrap()["recoveryAction"],
            "resume_same_execution"
        );
        drop(statement);
        drop(connection);
        assert_eq!(
            engine
                .load_resumable_agent_execution_request(&execution_id)
                .unwrap(),
            resolved_context
        );
        let checkpoint = engine
            .load_plan_execution_checkpoint(&resolved.plan.id, &resolved_plan, 3)
            .unwrap()
            .unwrap();
        assert_eq!(checkpoint.next_step_index, 1);
        assert_eq!(checkpoint.completed_actions.len(), 1);
        engine
            .resume_agent_execution(
                &execution_id,
                &resolved.plan.id,
                &context,
                &resolved_context,
            )
            .unwrap();
        let connection = engine.open_connection().unwrap();
        let (cursor, completed, unchanged_failures): (i64, i64, i64) = connection
            .query_row(
                "SELECT state.current_step_index,
                        (SELECT COUNT(*) FROM actions WHERE plan_id=?1 AND status='completed'),
                        (SELECT COUNT(*) FROM actions WHERE plan_id=?1 AND status=?2)
                 FROM plan_generation_states state WHERE state.plan_id=?1",
                params![
                    resolved.plan.id,
                    crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_EFFECTFUL,
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((cursor, completed, unchanged_failures), (1, 1, 1));
    }

    let (execution_id, context, _) = calendar_recovery_fixture(&engine, "cancel");
    engine
        .cancel_agent_calendar_recovery(&execution_id, &context.session_id)
        .unwrap();
    assert!(engine
        .load_calendar_recovery_execution_request(&execution_id, &context.session_id)
        .is_err());
    let connection = engine.open_connection().unwrap();
    let (status, durable_card): (String, String) = connection
        .query_row(
            "SELECT executions.status,
                    (SELECT content FROM chat_messages
                     WHERE session_id=executions.session_id AND role='assistant'
                     ORDER BY id DESC LIMIT 1)
             FROM agent_executions executions WHERE execution_id=?1",
            params![execution_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "cancelled");
    assert_eq!(
        serde_json::from_str::<Value>(&durable_card).unwrap()["recoveryAction"],
        "calendar_recovery_cancelled"
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn remaining_work_cancellation_is_session_owned_idempotent_and_preserves_checkpoints() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu-agent-remaining-work-cancel-{}",
        unix_time_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let artifact = temp_dir.join("completed-artifact.txt");
    std::fs::write(&artifact, "verified output").unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let (execution_id, context, request) = calendar_recovery_fixture(&engine, "stop-remaining");
    replace_calendar_target_receipt_with_permission_receipt(
        &engine,
        &execution_id,
        &context,
        &request.plan.id,
    );

    assert!(engine
        .cancel_agent_execution_remaining_work(&execution_id, "another-session")
        .is_err());
    assert_eq!(
        engine
            .cancel_agent_execution_remaining_work(&execution_id, &context.session_id)
            .unwrap(),
        1
    );
    assert_eq!(
        engine
            .cancel_agent_execution_remaining_work(&execution_id, &context.session_id)
            .unwrap(),
        1,
        "the exact committed cancellation is idempotent"
    );

    let connection = engine.open_connection().unwrap();
    let (status, cursor, action_count, completed_count, cancelled_log_count, durable_card): (
        String,
        i64,
        i64,
        i64,
        i64,
        String,
    ) = connection
        .query_row(
            "SELECT executions.status,state.current_step_index,
                    (SELECT COUNT(*) FROM actions WHERE plan_id=executions.plan_id),
                    (SELECT COUNT(*) FROM actions
                     WHERE plan_id=executions.plan_id AND status='completed'),
                    (SELECT COUNT(*) FROM agent_execution_logs
                     WHERE execution_id=executions.execution_id
                       AND phase='cancelled'
                       AND payload_json IS NOT NULL),
                    (SELECT content FROM chat_messages
                     WHERE session_id=executions.session_id AND role='assistant'
                     ORDER BY id DESC LIMIT 1)
             FROM agent_executions executions
             JOIN plan_generation_states state ON state.plan_id=executions.plan_id
             WHERE executions.execution_id=?1",
            params![execution_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(status, "cancelled");
    assert_eq!(cursor, 1, "the completed checkpoint cursor is preserved");
    assert_eq!((action_count, completed_count), (2, 1));
    assert_eq!(cancelled_log_count, 1, "idempotency adds no duplicate log");
    let durable_card = serde_json::from_str::<Value>(&durable_card).unwrap();
    assert_eq!(
        durable_card["code"],
        "agent_execution_remaining_work_cancelled"
    );
    assert_eq!(durable_card["recoveryAction"], "remaining_work_cancelled");
    assert_eq!(durable_card["context"]["completedStepCount"], 1);
    assert_eq!(durable_card["changedState"], "checkpoint_saved");
    assert!(artifact.is_file(), "completed artifacts are not removed");
    drop(connection);
    let _ = std::fs::remove_dir_all(temp_dir);
}
