use super::*;
use crate::tasks::effect_verification::resolve as resolve_effect_verification;
use std::sync::{Arc, Barrier};

struct EffectRecoveryFixture {
    engine: PersistenceEngine,
    root: std::path::PathBuf,
    task_run_id: String,
    task_id: String,
    instance_id: String,
    node_id: String,
    effect_key: String,
    effect_kind: String,
    verification_sequence: u64,
    node_payloads: String,
}

impl EffectRecoveryFixture {
    fn new(label: &str) -> Self {
        let task_run_id = TaskRunId::new().to_string();
        let task_id = TaskId::new().to_string();
        let root = std::env::temp_dir().join(format!(
            "oomu-effect-resolution-{label}-{}",
            task_run_id.trim_start_matches("taskrun_")
        ));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let project = crate::projects::repository::create(
            &engine,
            crate::projects::CreateProjectRequest {
                name: format!("Effect resolution {label}"),
                description: String::new(),
                data_policy: crate::projects::ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let workflow_id = format!("workflow-effect-{label}");
        let schedule_id = format!("routine-effect-{label}");
        let instance_id = format!("instance-effect-{label}");
        let node_id = "native-effect".to_string();
        let effect_kind = "create_system_calendar_event".to_string();
        let effect_key = format!("workflow-task:{node_id}:{effect_kind}:digest");
        let node_payloads = json!({
            "native-effect": {
                "status": "Failed",
                "error": { "code": "workflow_effect_verification_required" }
            }
        })
        .to_string();
        let connection = engine.open_connection().unwrap();
        connection.execute_batch("CREATE TABLE taskflows (flow_id TEXT PRIMARY KEY, parent_session_id TEXT NOT NULL, directive TEXT NOT NULL, status TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL); CREATE TABLE taskflow_steps (flow_id TEXT NOT NULL, status TEXT NOT NULL);").unwrap();
        connection.execute("INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,workflow_ir_json,compilation_status,is_active,created_at_ms,updated_at_ms,compiled_at_ms,project_id) VALUES (?1,1,'Effect recovery','','{}',NULL,'Compiled',1,1,1,1,?2)", params![workflow_id, project.project_id]).unwrap();
        connection.execute("INSERT INTO workflow_schedules(id,workflow_id,workflow_version,label,schedule_expression,run_request_json,is_active,next_run_at_ms,last_status,last_error,last_instance_id,created_at_ms,updated_at_ms,project_id,schedule_kind,routine_timezone) VALUES (?1,?2,1,'Effect recovery','manual','{}',0,NULL,'Failed','verification required',?3,1,1,?4,'one_shot','UTC')", params![schedule_id, workflow_id, instance_id, project.project_id]).unwrap();
        connection.execute("INSERT INTO execution_instances(id,workflow_id,workflow_version,status,node_payloads_json,error_json,created_at_ms,updated_at_ms,completed_at_ms,project_id) VALUES (?1,?2,1,'Failed',?3,?4,1,1,1,?5)", params![instance_id, workflow_id, node_payloads, json!({"code":"workflow_effect_verification_required"}).to_string(), project.project_id]).unwrap();
        connection.execute("INSERT INTO task_runs(task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,last_error,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'workflow',?4,'blocked','routine',?2,'Protected Calendar action','verification required',1,1,'recoverable')", params![task_run_id, task_id, project.project_id, instance_id]).unwrap();
        connection.execute("INSERT INTO routine_runs(schedule_id,execution_instance_id,task_run_id,scheduled_for_ms,created_at_ms) VALUES (?1,?2,?3,1,1)", params![schedule_id, instance_id, task_run_id]).unwrap();
        connection.execute("INSERT INTO task_effects(task_run_id,idempotency_key,effect_kind,state,updated_at_ms) VALUES (?1,?2,?3,'executed',1)", params![task_run_id, effect_key, effect_kind]).unwrap();
        drop(connection);
        let task = get(&engine, &task_run_id).unwrap();
        let verification_sequence = append_event_with_sequence(
                &engine.open_connection().unwrap(),
                &task,
                "workflow.effect.verification_required",
                EvidenceClass::ObservedResult,
                json!({
                    "nodeId": node_id.clone(),
                    "idempotencyKey": effect_key.clone(),
                    "effectKind": effect_kind.clone(),
                    "effectSummary": {"surface":"calendar","calendarName":"OOMU Test","title":"Supplier Decision Review"},
                    "reasonCode": "workflow_effect_execution_ambiguous",
                    "nextAction": "verify_only"
                }),
            )
            .unwrap()
            .unwrap();
        Self {
            engine,
            root,
            task_run_id,
            task_id,
            instance_id,
            node_id,
            effect_key,
            effect_kind,
            verification_sequence,
            node_payloads,
        }
    }

    fn request(
        &self,
        decision: TaskEffectVerificationDecision,
    ) -> ResolveTaskEffectVerificationRequest {
        ResolveTaskEffectVerificationRequest {
            task_run_id: self.task_run_id.clone(),
            task_id: self.task_id.clone(),
            runtime_record_id: self.instance_id.clone(),
            verification_sequence: Some(self.verification_sequence),
            node_id: Some(self.node_id.clone()),
            idempotency_key: Some(self.effect_key.clone()),
            effect_kind: Some(self.effect_kind.clone()),
            decision,
        }
    }

    fn stop_without_details(&self) -> ResolveTaskEffectVerificationRequest {
        ResolveTaskEffectVerificationRequest {
            task_run_id: self.task_run_id.clone(),
            task_id: self.task_id.clone(),
            runtime_record_id: self.instance_id.clone(),
            verification_sequence: None,
            node_id: None,
            idempotency_key: None,
            effect_kind: None,
            decision: TaskEffectVerificationDecision::StopWithoutRepeating,
        }
    }

    fn finish(self) {
        drop(self.engine);
        let _ = std::fs::remove_dir_all(self.root);
    }
}

#[test]
fn control_matrix_rejects_terminal_controls() {
    assert!(controls(TaskState::Completed, "taskflow", false, false).is_empty());
    assert!(controls(TaskState::Cancelled, "agent", false, false).is_empty());
}

#[test]
fn user_observed_non_occurrence_requeues_only_the_exact_scheduled_node_once() {
    let fixture = EffectRecoveryFixture::new("retry-once");
    let before = get(&fixture.engine, &fixture.task_run_id).unwrap();
    assert!(before.effect_verification_required);
    assert!(before.valid_controls.is_empty());
    reconcile_all(&fixture.engine).unwrap();
    let reconciled = get(&fixture.engine, &fixture.task_run_id).unwrap();
    assert_eq!(reconciled.state, TaskState::Blocked);
    assert_eq!(reconciled.recovery_state, "recoverable");
    assert!(reconciled.valid_controls.is_empty());

    let mut cross_task = fixture.request(TaskEffectVerificationDecision::DidNotHappen);
    cross_task.task_id = TaskId::new().to_string();
    assert!(resolve_effect_verification(&fixture.engine, cross_task)
        .unwrap_err()
        .contains("no longer matches this Task"));

    let after = resolve_effect_verification(
        &fixture.engine,
        fixture.request(TaskEffectVerificationDecision::DidNotHappen),
    )
    .unwrap();
    assert_eq!(after.state, TaskState::Queued);
    assert!(!after.effect_verification_required);
    let connection = fixture.engine.open_connection().unwrap();
    let checkpoint: (String, String, Option<String>, i64, String, i64) = connection
            .query_row(
                "SELECT e.status,e.node_payloads_json,e.error_json,(SELECT COUNT(*) FROM task_effects WHERE task_run_id=?2),s.last_status,(SELECT COUNT(*) FROM task_recovery_audit WHERE task_run_id=?2 AND decision='did_not_happen') FROM execution_instances e JOIN workflow_schedules s ON s.last_instance_id=e.id WHERE e.id=?1",
                params![fixture.instance_id, fixture.task_run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .unwrap();
    assert_eq!(checkpoint.0, "Pending");
    assert_eq!(checkpoint.1, fixture.node_payloads);
    assert!(checkpoint.2.is_none());
    assert_eq!(checkpoint.3, 0);
    assert_eq!(checkpoint.4, "Pending");
    assert_eq!(checkpoint.5, 1);
    drop(connection);

    let stale = resolve_effect_verification(
        &fixture.engine,
        fixture.request(TaskEffectVerificationDecision::DidNotHappen),
    )
    .unwrap_err();
    assert!(stale.contains("no longer matches this Task"));
    fixture.finish();
}

#[test]
fn user_observed_occurrence_stops_truthfully_without_enabling_replay() {
    let fixture = EffectRecoveryFixture::new("stop-without-replay");
    let after = resolve_effect_verification(
        &fixture.engine,
        fixture.request(TaskEffectVerificationDecision::Happened),
    )
    .unwrap();
    assert_eq!(after.state, TaskState::Failed);
    assert!(!after.effect_verification_required);
    assert_eq!(after.valid_controls, vec!["acknowledge_failure"]);
    assert!(retry(&fixture.engine, &fixture.task_run_id)
        .unwrap_err()
        .contains("verified postcondition"));
    let connection = fixture.engine.open_connection().unwrap();
    let audit: (String, String, String, i64, String) = connection
            .query_row(
                "SELECT a.resolved_state,a.decision,a.next_action,(SELECT COUNT(*) FROM task_effects WHERE task_run_id=?1 AND state='executed'),json_extract(ev.event_json,'$.evidenceClass') FROM task_recovery_audit a JOIN task_events ev ON ev.task_run_id=a.task_run_id AND json_extract(ev.event_json,'$.eventType')='workflow.effect.verification_resolved' WHERE a.task_run_id=?1",
                params![fixture.task_run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
    assert_eq!(audit.0, "stopped_unverified");
    assert_eq!(audit.1, "happened");
    assert_eq!(audit.2, "none");
    assert_eq!(audit.3, 1);
    assert_eq!(audit.4, "executed_mutation");
    drop(connection);
    fixture.finish();
}

#[test]
fn unknown_outcome_stops_without_releasing_or_retrying_the_effect() {
    let fixture = EffectRecoveryFixture::new("unknown-outcome-stop");
    let mut stale = fixture.request(TaskEffectVerificationDecision::StopWithoutRepeating);
    stale.verification_sequence = Some(fixture.verification_sequence + 1);
    assert!(resolve_effect_verification(&fixture.engine, stale)
        .unwrap_err()
        .contains("stale"));
    assert_eq!(
        get(&fixture.engine, &fixture.task_run_id).unwrap().state,
        TaskState::Blocked
    );

    let after = resolve_effect_verification(
        &fixture.engine,
        fixture.request(TaskEffectVerificationDecision::StopWithoutRepeating),
    )
    .unwrap();
    assert_eq!(after.state, TaskState::Cancelled);
    assert_eq!(after.recovery_state, "reconciled");
    assert!(!after.effect_verification_required);
    assert!(after.valid_controls.is_empty());

    let connection = fixture.engine.open_connection().unwrap();
    let record: (String, String, String, String, i64, String, i64, i64) = connection
        .query_row(
            "SELECT a.resolved_state,a.decision,a.next_action,e.status,\
             (SELECT COUNT(*) FROM task_effects WHERE task_run_id=?1 AND state='executed'),\
             json_extract(ev.event_json,'$.payload.outcome'),\
             json_extract(ev.event_json,'$.payload.postconditionVerified'),\
             (SELECT COUNT(*) FROM task_events WHERE task_run_id=?1 AND json_extract(event_json,'$.evidenceClass')='verified_postcondition') \
             FROM task_recovery_audit a \
             JOIN execution_instances e ON e.id=?2 \
             JOIN task_events ev ON ev.task_run_id=a.task_run_id \
                 AND json_extract(ev.event_json,'$.eventType')='workflow.effect.verification_resolved' \
             WHERE a.task_run_id=?1",
            params![fixture.task_run_id, fixture.instance_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(record.0, "stopped_outcome_unknown");
    assert_eq!(record.1, "stop_without_repeating");
    assert_eq!(record.2, "none");
    assert_eq!(record.3, "Failed");
    assert_eq!(record.4, 1);
    assert_eq!(record.5, "outcome_unknown");
    assert_eq!(record.6, 0);
    assert_eq!(record.7, 0);
    drop(connection);

    assert!(resolve_effect_verification(
        &fixture.engine,
        fixture.request(TaskEffectVerificationDecision::StopWithoutRepeating),
    )
    .unwrap_err()
    .contains("no longer matches this Task"));
    fixture.finish();
}

#[test]
fn details_unavailable_stop_derives_one_exact_event_server_side() {
    let fixture = EffectRecoveryFixture::new("details-unavailable-stop");
    let mut cross_task = fixture.stop_without_details();
    cross_task.task_id = TaskId::new().to_string();
    assert!(resolve_effect_verification(&fixture.engine, cross_task)
        .unwrap_err()
        .contains("no longer matches this Task"));

    let after =
        resolve_effect_verification(&fixture.engine, fixture.stop_without_details()).unwrap();
    assert_eq!(after.state, TaskState::Cancelled);
    assert!(!after.effect_verification_required);
    let connection = fixture.engine.open_connection().unwrap();
    let exact: (i64, String, String, String) = connection
        .query_row(
            "SELECT json_extract(event_json,'$.payload.verificationSequence'),\
             json_extract(event_json,'$.payload.nodeId'),\
             json_extract(event_json,'$.payload.idempotencyKey'),\
             json_extract(event_json,'$.payload.effectKind') \
             FROM task_events WHERE task_run_id=?1 \
             AND json_extract(event_json,'$.eventType')='workflow.effect.verification_resolved'",
            params![fixture.task_run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(exact.0, fixture.verification_sequence as i64);
    assert_eq!(exact.1, fixture.node_id);
    assert_eq!(exact.2, fixture.effect_key);
    assert_eq!(exact.3, fixture.effect_kind);
    drop(connection);
    fixture.finish();
}

#[test]
fn details_unavailable_stop_rejects_ambiguous_unresolved_events() {
    let fixture = EffectRecoveryFixture::new("ambiguous-details-stop");
    let task = get(&fixture.engine, &fixture.task_run_id).unwrap();
    append_event_with_sequence(
        &fixture.engine.open_connection().unwrap(),
        &task,
        "workflow.effect.verification_required",
        EvidenceClass::ObservedResult,
        json!({
            "nodeId": "another-node",
            "idempotencyKey": "another-effect",
            "effectKind": "draft_system_email",
            "nextAction": "verify_only"
        }),
    )
    .unwrap();

    let error =
        resolve_effect_verification(&fixture.engine, fixture.stop_without_details()).unwrap_err();
    assert!(error.contains("More than one protected action"));
    let unchanged = get(&fixture.engine, &fixture.task_run_id).unwrap();
    assert_eq!(unchanged.state, TaskState::Blocked);
    assert!(unchanged.effect_verification_required);
    let effect_count: i64 = fixture
        .engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM task_effects WHERE task_run_id=?1 AND state='executed'",
            params![fixture.task_run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(effect_count, 1);
    fixture.finish();
}

#[test]
fn registry_reconciles_and_controls_the_real_taskflow_row() {
    let root = std::env::temp_dir().join(format!(
        "oomu-task-registry-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();
    connection.execute_batch("CREATE TABLE taskflows (flow_id TEXT PRIMARY KEY, parent_session_id TEXT NOT NULL, directive TEXT NOT NULL, status TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL); CREATE TABLE taskflow_steps (flow_id TEXT NOT NULL, status TEXT NOT NULL);").unwrap();
    connection.execute("INSERT INTO taskflows (flow_id, parent_session_id, directive, status, created_at_ms, updated_at_ms) VALUES ('flow-1', 'legacy-session', 'Inspect the repository', 'active', 1, 2)", []).unwrap();
    drop(connection);
    let report = reconcile_all(&engine).unwrap();
    assert_eq!(report.reconciled, 1);
    let task = list(
        &engine,
        TaskFilter {
            project_id: None,
            state: Some(TaskState::Running),
            origin: None,
            runtime_kind: Some("taskflow".to_string()),
            from_ms: None,
            to_ms: None,
        },
    )
    .unwrap()
    .pop()
    .unwrap();
    let cancelled = cancel(&engine, &task.task_run_id).unwrap();
    assert_eq!(cancelled.state, TaskState::Cancelled);
    let request = TaskEffectRequest {
        task_run_id: task.task_run_id,
        idempotency_key: "effect-one".to_string(),
        effect_kind: "write".to_string(),
        result_digest: None,
    };
    assert!(reserve_effect(&engine, request.clone()).unwrap());
    assert!(!reserve_effect(&engine, request).unwrap());
    let connection = engine.open_connection().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT status FROM taskflows WHERE flow_id='flow-1'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "cancelled"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn concurrent_event_appends_return_unique_committed_sequences() {
    let root = std::env::temp_dir().join(format!(
        "oomu-task-event-sequence-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project = crate::projects::repository::create(
        &engine,
        crate::projects::CreateProjectRequest {
            name: "Concurrent evidence".into(),
            description: String::new(),
            data_policy: crate::projects::ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let task_id = TaskId::new().to_string();
    let task_run_id = TaskRunId::new().to_string();
    engine.open_connection().unwrap().execute("INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'agent','event-race','running','agent',?2,'Concurrent event test',1,1,'reconciled')", params![task_run_id,task_id,project.project_id]).unwrap();
    let writer_count = 8;
    let barrier = Arc::new(Barrier::new(writer_count));
    let handles = (0..writer_count)
        .map(|writer| {
            let engine = engine.clone();
            let task_run_id = task_run_id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                record_domain_event_with_sequence(
                    &engine,
                    &task_run_id,
                    "connector.tool.completed",
                    EvidenceClass::ObservedResult,
                    json!({"writer":writer}),
                )
                .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let mut sequences = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    sequences.sort_unstable();
    assert_eq!(sequences, (0..writer_count as u64).collect::<Vec<_>>());
    let stored: Vec<(i64, String)> = {
        let connection = engine.open_connection().unwrap();
        let mut statement = connection
                .prepare("SELECT sequence,event_json FROM task_events WHERE task_run_id=?1 ORDER BY sequence")
                .unwrap();
        let rows = statement
            .query_map(params![task_run_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows
    };
    assert_eq!(stored.len(), writer_count);
    for (sequence, encoded) in stored {
        let event: P0EventEnvelope = serde_json::from_str(&encoded).unwrap();
        assert_eq!(event.sequence, sequence as u64);
    }
    let _ = std::fs::remove_dir_all(root);
}
