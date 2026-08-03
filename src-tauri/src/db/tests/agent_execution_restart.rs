use super::*;

fn fixture_agenda_items() -> Vec<String> {
    ["One", "Two", "Three", "Four", "Five"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn fixture_numbered_agenda() -> String {
    fixture_agenda_items()
        .iter()
        .enumerate()
        .map(|(index, item)| format!("{}. {item}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn fixture_proposed_time() -> &'static str {
    "Tuesday, July 21, 2026 at 1:00 PM–1:30 PM (America/New_York)"
}

fn fixture_agenda_markdown() -> String {
    format!(
        "# OOMU Release Readiness Recovery Agenda\n\n**Status date:** 2026-07-20  \n**Proposed time:** {}  \n**Frozen start:** `2026-07-21T13:00:00-04:00`  \n**Frozen end:** `2026-07-21T13:30:00-04:00`  \n**Calendar timezone:** `America/New_York`  \n**Duration:** 30 minutes  \n**Availability:** Tentative\n\n## Milestone facts\n\n| ID | Milestone | Target date | Status | Owner | Recovery assessment |\n|---|---|---|---|---|---|\n\n\n## Decisions needed\n\n\n\n## Agenda — exactly five items\n\n{}\n",
        fixture_proposed_time(),
        fixture_numbered_agenda()
    )
}

fn checkpoint_outputs(
    input_path: &std::path::Path,
    agenda_path: &std::path::Path,
) -> Vec<crate::shield_gate::ExecuteCommandResponse> {
    let numbered_agenda = fixture_numbered_agenda();
    let notes = format!(
        "OOMU Release Readiness recovery meeting\n\nProposed time: {}\n\nAgenda:\n{}",
        fixture_proposed_time(),
        numbered_agenda
    );
    let input_bytes = std::fs::read(input_path).unwrap();
    let agenda_bytes = std::fs::read(agenda_path).unwrap();
    let input_path = std::fs::canonicalize(input_path).unwrap();
    let agenda_path = std::fs::canonicalize(agenda_path).unwrap();
    let agenda = json!({
        "status":"completed",
        "verified":true,
        "inputPath":input_path,
        "inputSha256":crate::foundation::digest::sha256_hex(&input_bytes),
        "outputPath":agenda_path,
        "outputSha256":crate::foundation::digest::sha256_hex(&agenda_bytes),
        "byteLength":agenda_bytes.len(),
        "asOfDate":"2026-07-20",
        "startDate":"2026-07-21T13:00:00-04:00",
        "endDate":"2026-07-21T13:30:00-04:00",
        "timeZone":"America/New_York",
        "proposedTime":fixture_proposed_time(),
        "agendaItems":fixture_agenda_items(),
        "milestoneFacts":[],
        "eventNotes":notes,
        "mailBody":format!(
            "OOMU release readiness recovery meeting\n\nProposed time: {}\n\n{}\n\nAgenda file: {}",
            fixture_proposed_time(),
            numbered_agenda,
            agenda_path.display()
        )
    });
    vec![
        crate::shield_gate::ExecuteCommandResponse {
            operation: "prepare_release_recovery_agenda".to_string(),
            status: crate::shield_gate::CommandStatus::Completed,
            message: agenda.to_string(),
            metrics: None,
            claims: vec!["agenda verified".to_string()],
            verified: true,
            model_used: None,
        },
        crate::shield_gate::ExecuteCommandResponse {
            operation: "create_release_recovery_calendar_event".to_string(),
            status: crate::shield_gate::CommandStatus::Completed,
            message: json!({
                "verified":true,
                "exists":true,
                "startDate":"2026-07-21T13:00:00-04:00",
                "endDate":"2026-07-21T13:30:00-04:00",
                "notesSha256":crate::foundation::digest::sha256_hex(notes.as_bytes())
            })
            .to_string(),
            metrics: None,
            claims: vec!["calendar event verified".to_string()],
            verified: true,
            model_used: None,
        },
    ]
}

fn verified_unchanged_calendar_denial() -> String {
    let agent_error = crate::tools::task_tool_runtime::normalize_agent_error(
        "create_release_recovery_calendar_event",
        &json!({
            "taskToolError": {
                "code": "calendar_action_denied",
                "message": "Calendar permission was denied before any event was created.",
                "context": {
                    "requestedCalendarName": "OOMU Test",
                    "availableCalendarNames": ["Personal", "Work"],
                    "changedState": false
                }
            }
        })
        .to_string(),
    );
    json!({
        "schema": crate::agentic_loop::recovery::ACTION_FAILURE_RECEIPT_SCHEMA,
        "operation": "create_release_recovery_calendar_event",
        "status": "failed",
        "verified": true,
        "changedState": "none",
        "potentiallyEffectful": true,
        "agentError": agent_error,
    })
    .to_string()
}

fn release_recovery_request(
    plan_id: &str,
    context: &ChatTurnPersistenceContext,
    input_path: &std::path::Path,
    agenda_path: &std::path::Path,
) -> crate::agentic_loop::AgentPlanExecutionRequest {
    let registered = |operation, arguments| {
        crate::agentic_loop::Tool::RegisteredTaskTool(
            crate::tools::task_tool_runtime::PlannedTaskToolRequest::new(operation, arguments),
        )
    };
    crate::agentic_loop::AgentPlanExecutionRequest {
        plan: crate::agentic_loop::ActionPlan {
            id: plan_id.to_string(),
            objective: "Recover the release-readiness meeting".to_string(),
            intent: crate::gemma::StructuredIntent {
                objective: "Recover the release-readiness meeting".to_string(),
                category: crate::gemma::IntentCategory::ProjectAnalysis,
                source: crate::gemma::IntentSource::Deterministic,
                degraded_reason: None,
            },
            steps: vec![
                crate::agentic_loop::Step {
                    step: "Prepare the verified agenda".to_string(),
                    tool: registered(
                        "prepare_release_recovery_agenda",
                        json!({
                            "inputPath":input_path,
                            "outputPath":agenda_path,
                            "day":"next_weekday",
                            "windowStartLocal":"13:00",
                            "windowEndLocal":"16:00",
                            "durationMinutes":30,
                            "agendaItemCount":5,
                            "locale":"en-US"
                        }),
                    ),
                    risk_level: crate::agentic_loop::RiskLevel::High,
                },
                crate::agentic_loop::Step {
                    step: "Create the tentative Calendar event".to_string(),
                    tool: registered(
                        "create_release_recovery_calendar_event",
                        json!({
                            "calendarName":"OOMU Test",
                            "title":"OOMU Release Readiness - Recovery Meeting",
                            "agendaStep":0,
                            "availability":"tentative"
                        }),
                    ),
                    risk_level: crate::agentic_loop::RiskLevel::High,
                },
                crate::agentic_loop::Step {
                    step: "Create the unsent Mail draft".to_string(),
                    tool: registered(
                        "draft_release_recovery_email",
                        json!({
                            "to":"recipient@example.com",
                            "subject":"OOMU Release Readiness - Recovery Meeting",
                            "agendaStep":0,
                            "calendarStep":1
                        }),
                    ),
                    risk_level: crate::agentic_loop::RiskLevel::High,
                },
            ],
            exit_condition: "All three outputs are verified.".to_string(),
            logical_certificate: crate::shield_gate::LogicalCertificate::unsigned(
                vec!["verified receipts".to_string()],
                vec!["resume the pending Mail step".to_string()],
                "Do not replay completed work.".to_string(),
            ),
            trusted_automatic_execution: false,
            model_route: crate::agentic_loop::ModelRouteDecision {
                selected_model: crate::shield_gate::ModelMetadata {
                    name: "Gemma".to_string(),
                    version: "E4B".to_string(),
                    provider: "local".to_string(),
                    locality: "local".to_string(),
                },
                provider_config_id: None,
                provider_id: Some("local_model".to_string()),
                recommended_model: None,
                requires_principal_authorization: false,
                reason: "deterministic recovery fixture".to_string(),
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
            automated_web_grounding_enabled: false,
            attachment_grants: Vec::new(),
            created_at_ms: 1,
        },
        principal_approved: true,
        authority_proof_id: None,
    }
}

fn pending_mail_approval(
    engine: &PersistenceEngine,
    execution_id: &str,
    request: &crate::agentic_loop::AgentPlanExecutionRequest,
    outputs: &[crate::shield_gate::ExecuteCommandResponse],
) -> crate::authority::shield_decision::FrozenShieldRequest {
    let crate::agentic_loop::Tool::RegisteredTaskTool(planned) = &request.plan.steps[2].tool else {
        panic!("release Mail step missing")
    };
    let validated = crate::tools::task_tool_runtime::authorize(
        crate::tools::task_tool_runtime::requested_action(planned),
    )
    .unwrap();
    let resolved =
        crate::tools::task_tool_runtime::resolve(engine, Some(execution_id), validated, outputs)
            .unwrap();
    let action = crate::tools::task_tool_runtime::requested_action_for_validated(&resolved);
    let mut approval = crate::shield_gate::build_shield_approval_request(&action).unwrap();
    approval.session_id = Some(request.turn_context.session_id.clone());
    approval.turn_id = Some(request.turn_context.turn_id.clone());
    approval.generation_token = Some(request.turn_context.generation_token.clone());
    approval.principal = Some(request.turn_context.agent_id.clone());
    crate::authority::shield_decision::freeze_request(&approval).unwrap()
}

#[test]
fn restart_before_mail_persists_exact_checkpoint_and_requires_a_fresh_approval() {
    let _ = crate::tools::release_recovery::register_task_tools();
    assert!(crate::tools::task_tool_runtime::is_registered(
        "draft_release_recovery_email"
    ));
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-restart-before-mail-{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("release_milestones.json");
    let agenda_path = temp_dir.join("oomu_release_recovery_agenda.md");
    std::fs::write(
        &input_path,
        br#"[{"milestoneId":"M1","name":"Ship","targetDate":"2026-07-20","status":"PENDING","owner":"OOMU"}]"#,
    )
    .unwrap();
    std::fs::write(&agenda_path, fixture_agenda_markdown().as_bytes()).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("chat.sqlite")).unwrap();
    let execution_id = "execution-restart-before-mail";
    let plan_id = "plan-restart-before-mail";
    let agent_id = "agent-restart-before-mail";
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: agent_id.to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-test".to_string(),
            title: Some("Restart before Mail".to_string()),
            dynamic_routing_override: None,
            workspace_id: Some(engine.workspace_id.clone()),
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-restart-before-mail".to_string(),
        generation_token: "generation-restart-before-mail".to_string(),
        session_id: session.id,
        agent_id: agent_id.to_string(),
        provider_id: "local_model".to_string(),
        model_id: "gemma-test".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-restart-before-mail".to_string(),
        turn_kind: "root".to_string(),
    };
    let request = release_recovery_request(plan_id, &context, &input_path, &agenda_path);
    let context_json = serde_json::to_string(&request).unwrap();
    let plan_json = serde_json::to_string(&request.plan).unwrap();
    let outputs = checkpoint_outputs(&input_path, &agenda_path);
    engine.begin_chat_turn(&context).unwrap();
    engine.finish_chat_turn(&context, "completed").unwrap();
    engine
        .begin_agent_execution(execution_id, plan_id, &context, &context_json)
        .unwrap();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO plan_generation_states
             (plan_id,plan_json,current_step_index,status,generated_text,timestamp_ms)
             VALUES (?1,?2,2,'checkpointed','Calendar event verified',1)",
            params![plan_id, plan_json],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
             VALUES (?1,?2,'{}',?3,'completed',1)",
            params![
                plan_id,
                outputs[0].operation,
                serde_json::to_string(&outputs[0]).unwrap(),
            ],
        )
        .unwrap();
    let calendar_denial = verified_unchanged_calendar_denial();
    connection
        .execute(
            "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
             VALUES (?1,'create_release_recovery_calendar_event','{}',?2,?3,2)",
            params![
                plan_id,
                calendar_denial,
                crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_EFFECTFUL,
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO actions (plan_id,tool,input,output,status,timestamp_ms)
             VALUES (?1,?2,'{}',?3,'completed',3)",
            params![
                plan_id,
                outputs[1].operation,
                serde_json::to_string(&outputs[1]).unwrap(),
            ],
        )
        .unwrap();
    drop(connection);

    let before = pending_mail_approval(&engine, execution_id, &request, &outputs);
    engine.audit_recovery();
    let connection = engine.open_connection().unwrap();
    let (status, cursor, action_count, phase, payload, chat_payload): (
        String,
        i64,
        i64,
        String,
        String,
        String,
    ) = connection
        .query_row(
            "SELECT executions.status,state.current_step_index,
                    (SELECT COUNT(*) FROM actions WHERE plan_id=executions.plan_id),
                    (SELECT phase FROM agent_execution_logs WHERE execution_id=executions.execution_id AND payload_json IS NOT NULL ORDER BY id DESC LIMIT 1),
                    (SELECT payload_json FROM agent_execution_logs WHERE execution_id=executions.execution_id AND payload_json IS NOT NULL ORDER BY id DESC LIMIT 1),
                    (SELECT content FROM chat_messages WHERE session_id=executions.session_id AND role='assistant' ORDER BY id DESC LIMIT 1)
             FROM agent_executions executions JOIN plan_generation_states state ON state.plan_id=executions.plan_id
             WHERE executions.execution_id=?1",
            params![execution_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
        )
        .unwrap();
    drop(connection);
    assert_eq!((status.as_str(), cursor, action_count), ("halted", 2, 3));
    assert_eq!(phase, "restart_recovery_ready");
    assert_eq!(payload, chat_payload);
    let receipt = serde_json::from_str::<Value>(&payload).unwrap();
    assert_eq!(receipt["recoveryAction"], "resume_same_execution");
    assert_eq!(receipt["changedState"], "checkpoint_saved");
    assert_eq!(receipt["context"]["completedStepCount"], 2);
    assert_eq!(receipt["context"]["nextStepIndex"], 2);
    assert_eq!(
        receipt["context"]["nextOperation"],
        "draft_release_recovery_email"
    );
    assert_eq!(
        receipt["context"]["replaySafeActionEvidence"][0]["status"],
        crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_EFFECTFUL
    );
    assert_eq!(
        receipt["context"]["replaySafeActionEvidence"][0]["receiptSha256"],
        crate::foundation::digest::sha256_hex(calendar_denial.as_bytes())
    );
    assert_eq!(
        receipt["context"]["frozenArgumentSha256"],
        before.argument_sha256
    );
    assert_eq!(receipt["context"]["approvalTokenRetained"], false);
    assert!(!payload.contains(&before.approval_token));
    assert_eq!(
        engine
            .load_resumable_agent_execution_request(execution_id)
            .unwrap(),
        context_json
    );

    let after = pending_mail_approval(&engine, execution_id, &request, &outputs);
    assert_ne!(after.approval_token, before.approval_token);
    assert_ne!(after.request_sha256, before.request_sha256);
    assert_eq!(after.argument_sha256, before.argument_sha256);
    let mut tampered_receipts = Vec::new();
    let mut missing_schema = receipt.clone();
    missing_schema.as_object_mut().unwrap().remove("schema");
    tampered_receipts.push(missing_schema);
    for (pointer, value) in [
        ("/code", json!("different_recovery")),
        ("/boundary", json!("DifferentBoundary")),
        ("/context/frozenArgumentSha256", json!("0".repeat(64))),
        ("/context/completedReceiptSha256s/0", json!("0".repeat(64))),
        (
            "/context/replaySafeActionEvidence/0/receiptSha256",
            json!("0".repeat(64)),
        ),
        (
            "/context/replaySafeActionEvidence/0/status",
            json!("prepared_effectful"),
        ),
        ("/context/approvalTokenRetained", json!(true)),
        ("/context/approvalRequiredOnResume", json!(false)),
    ] {
        let mut tampered = receipt.clone();
        *tampered.pointer_mut(pointer).unwrap() = value;
        tampered_receipts.push(tampered);
    }
    for tampered in tampered_receipts {
        let connection = engine.open_connection().unwrap();
        connection
            .execute(
                "UPDATE agent_execution_logs SET payload_json=?1
                 WHERE execution_id=?2 AND phase='restart_recovery_ready'",
                params![tampered.to_string(), execution_id],
            )
            .unwrap();
        drop(connection);
        assert!(engine
            .load_resumable_agent_execution_request(execution_id)
            .is_err());
        assert!(engine
            .resume_agent_execution(execution_id, plan_id, &context, &context_json)
            .is_err());
    }
    let mut changed_completed_output = serde_json::to_value(&outputs[1]).unwrap();
    *changed_completed_output.pointer_mut("/claims/0").unwrap() =
        json!("calendar event remains verified but the durable receipt changed");
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE agent_execution_logs SET payload_json=?1
             WHERE execution_id=?2 AND phase='restart_recovery_ready'",
            params![payload, execution_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE actions SET output=?1
             WHERE plan_id=?2 AND tool='create_release_recovery_calendar_event'
               AND status='completed'",
            params![changed_completed_output.to_string(), plan_id],
        )
        .unwrap();
    drop(connection);
    assert!(engine
        .load_resumable_agent_execution_request(execution_id)
        .is_err());
    assert!(engine
        .resume_agent_execution(execution_id, plan_id, &context, &context_json)
        .is_err());
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE agent_execution_logs SET payload_json=?1
             WHERE execution_id=?2 AND phase='restart_recovery_ready'",
            params![payload, execution_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE actions SET output=?1
             WHERE plan_id=?2 AND tool='create_release_recovery_calendar_event'
               AND status='completed'",
            params![serde_json::to_string(&outputs[1]).unwrap(), plan_id],
        )
        .unwrap();
    drop(connection);
    let mut changed_denial = serde_json::from_str::<Value>(&calendar_denial).unwrap();
    changed_denial["diagnostic"] = json!("different verified unchanged receipt bytes");
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE actions SET output=?1
             WHERE plan_id=?2 AND status=?3",
            params![
                changed_denial.to_string(),
                plan_id,
                crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_EFFECTFUL,
            ],
        )
        .unwrap();
    drop(connection);
    assert!(engine
        .load_resumable_agent_execution_request(execution_id)
        .is_err());
    assert!(engine
        .resume_agent_execution(execution_id, plan_id, &context, &context_json)
        .is_err());
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE actions SET output=?1,status=?2
             WHERE plan_id=?3 AND status=?4",
            params![
                calendar_denial,
                crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_EFFECTFUL,
                plan_id,
                crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_EFFECTFUL,
            ],
        )
        .unwrap();
    drop(connection);
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE actions SET status=?1
             WHERE plan_id=?2 AND status=?3",
            params![
                crate::agentic_loop::recovery::ACTION_UNVERIFIED_EFFECTFUL,
                plan_id,
                crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_EFFECTFUL,
            ],
        )
        .unwrap();
    drop(connection);
    assert!(engine
        .load_resumable_agent_execution_request(execution_id)
        .is_err());
    assert!(engine
        .resume_agent_execution(execution_id, plan_id, &context, &context_json)
        .is_err());
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE actions SET status=?1
             WHERE plan_id=?2 AND status=?3",
            params![
                crate::agentic_loop::recovery::ACTION_FAILED_UNCHANGED_EFFECTFUL,
                plan_id,
                crate::agentic_loop::recovery::ACTION_UNVERIFIED_EFFECTFUL,
            ],
        )
        .unwrap();
    drop(connection);
    engine
        .resume_agent_execution(execution_id, plan_id, &context, &context_json)
        .unwrap();
    let checkpoint = engine
        .load_plan_execution_checkpoint(plan_id, &plan_json, 3)
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.next_step_index, 2);
    assert_eq!(checkpoint.completed_actions.len(), 2);
    let connection = engine.open_connection().unwrap();
    let (resumed_status, action_count): (String, i64) = connection
        .query_row(
            "SELECT status,(SELECT COUNT(*) FROM actions WHERE plan_id=?2)
             FROM agent_executions WHERE execution_id=?1",
            params![execution_id, plan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((resumed_status.as_str(), action_count), ("running", 3));
    drop(connection);
    let _ = std::fs::remove_dir_all(temp_dir);
}
