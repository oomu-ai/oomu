use super::*;

fn bounded_hourly_schedule(end_at_ms: i64) -> WorkflowScheduleRecord {
    WorkflowScheduleRecord {
        id: "routine_bounded_hourly".to_string(),
        workflow_id: "workflow-bounded-hourly".to_string(),
        workflow_version: Some(1),
        label: "Bounded hourly".to_string(),
        schedule_expression: "every 1 hour".to_string(),
        run_request: crate::routines::control::with_end_at_ms(&json!({}), end_at_ms).unwrap(),
        is_active: true,
        next_run_at_ms: Some(1_000),
        claimed_at_ms: None,
        last_started_at_ms: None,
        last_completed_at_ms: None,
        last_status: None,
        last_error: None,
        last_instance_id: None,
        created_at_ms: 1,
        updated_at_ms: 1,
        routine_timezone: "UTC".to_string(),
        schedule_kind: "recurring".to_string(),
        project_id: None,
        missed_run_policy: "skip".to_string(),
        missed_run_cap: 3,
        active_window_start_minute: None,
        active_window_end_minute: None,
        delivery_target: json!({}),
        authority: json!({}),
    }
}

fn hourly_schedule_at(
    due_at_ms: i64,
    missed_run_policy: &str,
    missed_run_cap: u8,
) -> WorkflowScheduleRecord {
    let mut schedule = bounded_hourly_schedule(i64::MAX);
    schedule.run_request = json!({});
    schedule.next_run_at_ms = Some(due_at_ms);
    schedule.missed_run_policy = missed_run_policy.to_string();
    schedule.missed_run_cap = missed_run_cap;
    schedule
}

#[test]
fn recurring_next_run_is_capped_before_the_reviewed_end_boundary() {
    let after_ms = 1_000;
    let candidate = after_ms + 60 * 60 * 1_000;
    assert_eq!(
        next_run_with_end_boundary(&bounded_hourly_schedule(candidate + 1), after_ms).unwrap(),
        Some(candidate)
    );
    assert_eq!(
        next_run_with_end_boundary(&bounded_hourly_schedule(candidate), after_ms).unwrap(),
        None
    );
}

#[test]
fn late_execution_advances_from_the_claimed_occurrence_not_the_clock() {
    let due = 1_000;
    let now = due + 10 * 60 * 1_000;
    let plans = claimed_occurrences_at(&hourly_schedule_at(due, "run_once", 3), now).unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].schedule.next_run_at_ms, Some(due));
    assert_eq!(plans[0].next_run_at_ms, Some(due + 60 * 60 * 1_000));
    assert_ne!(plans[0].next_run_at_ms, Some(now + 60 * 60 * 1_000),);
}

#[test]
fn run_once_on_wake_discards_extra_missed_slots_without_moving_the_anchor() {
    let due = 1_000;
    let hour = 60 * 60 * 1_000;
    let now = due + 2 * hour + 10 * 60 * 1_000;
    let plans = claimed_occurrences_at(&hourly_schedule_at(due, "run_once", 3), now).unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].schedule.next_run_at_ms, Some(due));
    assert_eq!(plans[0].next_run_at_ms, Some(due + 3 * hour));
}

#[test]
fn run_each_catches_up_unique_occurrences_once_and_enforces_the_cap() {
    let due = 1_000;
    let hour = 60 * 60 * 1_000;
    let now = due + 5 * hour + 1;
    let plans = claimed_occurrences_at(&hourly_schedule_at(due, "run_each", 3), now).unwrap();
    let scheduled = plans
        .iter()
        .map(|plan| plan.schedule.next_run_at_ms.unwrap())
        .collect::<Vec<_>>();
    assert_eq!(scheduled, vec![due, due + hour, due + 2 * hour]);
    assert_eq!(plans.last().unwrap().next_run_at_ms, Some(due + 6 * hour));
    assert_eq!(
        scheduled
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        scheduled.len()
    );
    assert!(!scheduled.contains(&plans.last().unwrap().next_run_at_ms.unwrap()));
}

#[test]
fn skipped_missed_run_uses_the_original_recurrence_grid() {
    let due = 1_000;
    let hour = 60 * 60 * 1_000;
    let now = due + 2 * hour + 10 * 60 * 1_000;
    assert_eq!(
        first_run_after_now_from_occurrence(&hourly_schedule_at(due, "skip", 3), due, now).unwrap(),
        Some(due + 3 * hour)
    );
}

#[test]
fn run_now_returns_to_the_saved_occurrence_without_shifting_it() {
    let now = 1_000;
    let scheduled = now + 42 * 60 * 1_000;
    let end = now + 24 * 60 * 60 * 1_000;
    let mut schedule = hourly_schedule_at(now, "run_each", 3);
    schedule.run_request = crate::routines::control::with_run_now_resume_at_ms(
        &crate::routines::control::with_end_at_ms(&json!({}), end).unwrap(),
        scheduled,
    )
    .unwrap();

    let plans = claimed_occurrences_at(&schedule, now + 30_000).unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].schedule.next_run_at_ms, Some(now));
    assert_eq!(plans[0].next_run_at_ms, Some(scheduled));
}

#[test]
fn reached_end_boundary_deactivates_and_releases_the_routine() {
    let root = std::env::temp_dir().join(format!(
        "oomu-routine-end-boundary-{}",
        crate::p0_contracts::TaskId::new()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO workflow_schedules(id,workflow_id,label,schedule_expression,run_request_json,is_active,next_run_at_ms,claimed_at_ms,last_status,created_at_ms,updated_at_ms) VALUES ('routine_end_boundary','workflow','End boundary','every 1 hour','{}',1,1,1,'Completed',1,1)",
            [],
        )
        .unwrap();

    finish_routine_at_end_boundary(&engine, "routine_end_boundary").unwrap();
    let state: (i64, Option<i64>, Option<i64>, String, String) = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT is_active,next_run_at_ms,claimed_at_ms,last_status,paused_reason FROM workflow_schedules WHERE id='routine_end_boundary'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        (
            0,
            None,
            None,
            "Completed".to_string(),
            "Routine end time reached".to_string()
        )
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn end_boundary_preserves_a_failed_final_run() {
    let root = std::env::temp_dir().join(format!(
        "oomu-routine-failed-end-boundary-{}",
        crate::p0_contracts::TaskId::new()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO workflow_schedules(id,workflow_id,label,schedule_expression,run_request_json,is_active,last_status,last_error,created_at_ms,updated_at_ms) VALUES ('routine_failed_end_boundary','workflow','Failed boundary','every 1 hour','{}',1,'Failed','mail_read_failed',1,1)",
            [],
        )
        .unwrap();

    finish_routine_at_end_boundary(&engine, "routine_failed_end_boundary").unwrap();
    let state: (i64, String, String, String) = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT is_active,last_status,last_error,paused_reason FROM workflow_schedules WHERE id='routine_failed_end_boundary'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        (
            0,
            "Failed".to_string(),
            "mail_read_failed".to_string(),
            "Routine end time reached".to_string(),
        )
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn end_boundary_finalizes_before_a_later_next_run_and_preserves_the_real_failure() {
    let root = std::env::temp_dir().join(format!(
        "oomu-routine-preclaim-end-boundary-{}",
        crate::p0_contracts::TaskId::new()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let end = 10_000;
    let request = crate::routines::control::with_end_at_ms(&json!({}), end).unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO workflow_schedules(id,workflow_id,label,schedule_expression,run_request_json,is_active,next_run_at_ms,last_status,last_error,created_at_ms,updated_at_ms) VALUES ('routine_failed_before_boundary','workflow','Failed before boundary','every 1 hour',?1,1,?2,'Failed','verified_final_failure',1,1)",
            rusqlite::params![request.to_string(), end + 7 * 24 * 60 * 60 * 1_000],
        )
        .unwrap();

    finalize_due_routines_at_end_boundary(&engine, end).unwrap();
    let state: (i64, Option<i64>, String, String, String) = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT is_active,next_run_at_ms,last_status,last_error,paused_reason FROM workflow_schedules WHERE id='routine_failed_before_boundary'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        (
            0,
            None,
            "Failed".to_string(),
            "verified_final_failure".to_string(),
            "Routine end time reached".to_string(),
        )
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn persisting_run_now_result_consumes_only_the_temporary_anchor() {
    let root = std::env::temp_dir().join(format!(
        "oomu-routine-consume-run-now-{}",
        crate::p0_contracts::TaskId::new()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let now = 1_000;
    let scheduled = now + 60 * 60 * 1_000;
    let end = now + 12 * 60 * 60 * 1_000;
    let mut schedule = hourly_schedule_at(now, "run_once", 3);
    schedule.id = "routine_consume_run_now".to_string();
    schedule.run_request = crate::routines::control::with_run_now_resume_at_ms(
        &crate::routines::control::with_end_at_ms(&json!({}), end).unwrap(),
        scheduled,
    )
    .unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO workflow_schedules(id,workflow_id,label,schedule_expression,run_request_json,is_active,next_run_at_ms,claimed_at_ms,last_status,created_at_ms,updated_at_ms,schedule_kind,routine_timezone,missed_run_policy,missed_run_cap) VALUES (?1,?2,?3,?4,?5,1,?6,?6,'Running',1,1,'recurring','UTC','run_once',3)",
            rusqlite::params![schedule.id, schedule.workflow_id, schedule.label, schedule.schedule_expression, schedule.run_request.to_string(), now],
        )
        .unwrap();

    persist_claimed_schedule_result(
        &engine,
        &schedule,
        ExecutionStatus::Failed,
        None,
        Some("real_failure"),
        Some(scheduled),
    )
    .unwrap();
    let saved = engine.load_workflow_schedule(&schedule.id).unwrap();
    assert_eq!(saved.next_run_at_ms, Some(scheduled));
    assert_eq!(saved.last_status.as_deref(), Some("Failed"));
    assert_eq!(saved.last_error.as_deref(), Some("real_failure"));
    assert_eq!(
        crate::routines::control::run_now_resume_at_ms(&saved.run_request).unwrap(),
        None
    );
    assert_eq!(
        crate::routines::control::end_at_ms(&saved.run_request).unwrap(),
        Some(end)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn projection_error_is_persisted_as_failure_before_end_boundary_finalization() {
    let root = std::env::temp_dir().join(format!(
        "oomu-routine-projection-failure-{}",
        crate::p0_contracts::TaskId::new()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let mut schedule = bounded_hourly_schedule(3_000);
    schedule.id = "routine_projection_failure".to_string();
    schedule.next_run_at_ms = Some(1_000);
    engine.open_connection().unwrap().execute(
        "INSERT INTO workflow_schedules(id,workflow_id,label,schedule_expression,run_request_json,is_active,next_run_at_ms,claimed_at_ms,last_status,created_at_ms,updated_at_ms,schedule_kind,routine_timezone) VALUES (?1,?2,?3,?4,?5,1,?6,?6,'Running',1,1,'recurring','UTC')",
        rusqlite::params![schedule.id,schedule.workflow_id,schedule.label,schedule.schedule_expression,schedule.run_request.to_string(),1_000],
    ).unwrap();
    let response = workflow_runtime::RunWorkflowResponse {
        instance: crate::workflow_ir::ExecutionInstance {
            id: "wfi-missing-projection".to_string(),
            workflow_id: schedule.workflow_id.clone(),
            workflow_version: 1,
            status: ExecutionStatus::Completed,
            active_node_id: None,
            input_payload: json!({}),
            output_payload: Some(json!({"status":"completed"})),
            node_payloads: std::collections::HashMap::new(),
            memory: std::collections::HashMap::new(),
            selected_edges: std::collections::HashSet::new(),
            pause_context: None,
            error: None,
            execution_latency_ms: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            created_at_ms: 1,
            started_at_ms: Some(1),
            updated_at_ms: 2,
            completed_at_ms: Some(2),
        },
        execution_order: Vec::new(),
        approval_request: None,
        completion: Some(workflow_runtime::WorkflowCompletion {
            kind: WorkflowCompletionKind::Result,
        }),
    };

    let error =
        project_claimed_schedule_result(&engine, &schedule, &response, None, true).unwrap_err();
    let saved = engine.load_workflow_schedule(&schedule.id).unwrap();
    assert!(!error.is_empty());
    assert!(!saved.is_active);
    assert_eq!(saved.last_status.as_deref(), Some("Failed"));
    assert_eq!(saved.last_error.as_deref(), Some(error.as_str()));
    let paused_reason: String = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT paused_reason FROM workflow_schedules WHERE id=?1",
            rusqlite::params![schedule.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(paused_reason, "Routine end time reached");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_already_recorded_late_occurrence_is_advanced_without_a_second_run() {
    let root = std::env::temp_dir().join(format!(
        "oomu-routine-recorded-occurrence-{}",
        crate::p0_contracts::TaskId::new()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let due = 1_000;
    let next = due + 60 * 60 * 1_000;
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO workflow_schedules(id,workflow_id,label,schedule_expression,run_request_json,is_active,next_run_at_ms,claimed_at_ms,last_status,created_at_ms,updated_at_ms,schedule_kind,routine_timezone) VALUES ('routine_recorded_occurrence','workflow-recorded','Recorded','every 1 hour','{}',1,?1,?1,'Running',1,1,'recurring','UTC')",
            rusqlite::params![due],
        )
        .unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,is_active,created_at_ms,updated_at_ms) VALUES ('workflow-recorded',1,'Recorded','','{}',1,1,1)",
            [],
        )
        .unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO execution_instances(id,workflow_id,workflow_version,status,created_at_ms,updated_at_ms) VALUES ('wfi-recorded-occurrence','workflow-recorded',1,'Running',1,1)",
            [],
        )
        .unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO routine_runs(schedule_id,execution_instance_id,scheduled_for_ms,created_at_ms) VALUES ('routine_recorded_occurrence','wfi-recorded-occurrence',?1,1)",
            rusqlite::params![due],
        )
        .unwrap();
    let schedule = engine
        .load_workflow_schedule("routine_recorded_occurrence")
        .unwrap();
    assert!(advance_if_recorded(
        &engine,
        &ClaimedOccurrencePlan {
            schedule,
            next_run_at_ms: Some(next),
        },
    )
    .unwrap());

    let saved = engine
        .load_workflow_schedule("routine_recorded_occurrence")
        .unwrap();
    assert_eq!(saved.next_run_at_ms, Some(next));
    assert!(saved.claimed_at_ms.is_none());
    let logical_runs: i64 = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM routine_runs WHERE schedule_id='routine_recorded_occurrence' AND scheduled_for_ms=?1",
            rusqlite::params![due],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(logical_runs, 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn scheduler_owner_fails_closed_when_window_visibility_is_not_foreground() {
    assert_eq!(
        scheduler_owner_from_visibility(Some(true)),
        crate::routines::background::ScheduledWorkOwner::ForegroundApplication,
    );
    for visibility in [Some(false), None] {
        assert_eq!(
            scheduler_owner_from_visibility(visibility),
            crate::routines::background::ScheduledWorkOwner::DetachedRuntime,
        );
    }
}

#[test]
fn routine_result_policy_counts_failure_once_and_resets_on_completion() {
    let root = std::env::temp_dir().join(format!("oomu-routine-policy-{}", unix_time_ms()));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    engine
            .open_connection()
            .unwrap()
            .execute(
                "INSERT INTO workflow_schedules (id,workflow_id,workflow_version,label,schedule_expression,run_request_json,is_active,created_at_ms,updated_at_ms,consecutive_failures,failure_threshold) VALUES ('routine_policy','workflow',NULL,'Policy','manual','{}',1,1,1,0,3)",
                [],
            )
            .unwrap();

    record_result_policy(&engine, "routine_policy", true, false).unwrap();
    let failures: i64 = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT consecutive_failures FROM workflow_schedules WHERE id='routine_policy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(failures, 1);

    record_result_policy(&engine, "routine_policy", false, false).unwrap();
    let (failures, is_active, paused_reason): (i64, i64, Option<String>) = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT consecutive_failures,is_active,paused_reason FROM workflow_schedules WHERE id='routine_policy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
    assert_eq!(failures, 0);
    assert_eq!(is_active, 1);
    assert!(paused_reason.is_none());

    record_result_policy(&engine, "routine_policy", false, true).unwrap();
    let (is_active, paused_reason): (i64, Option<String>) = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT is_active,paused_reason FROM workflow_schedules WHERE id='routine_policy'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(is_active, 0);
    assert_eq!(paused_reason.as_deref(), Some("One-time routine completed"));

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn approval_reconciliation_preserves_next_pause_then_completes_one_shot_once() {
    let root = std::env::temp_dir().join(format!(
        "oomu-routine-approval-reconcile-{}",
        unix_time_ms()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let now = unix_time_ms();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,workflow_ir_json,is_active,created_at_ms,updated_at_ms,compiled_at_ms) VALUES ('workflow-reconcile',1,'Reconcile','','{}',NULL,1,?1,?1,?1)",
            rusqlite::params![now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO workflow_schedules(id,workflow_id,workflow_version,label,schedule_expression,run_request_json,is_active,next_run_at_ms,created_at_ms,updated_at_ms,schedule_kind,routine_timezone) VALUES ('routine_reconcile','workflow-reconcile',1,'Reconcile','manual','{}',1,NULL,?1,?1,'one_shot','UTC')",
            rusqlite::params![now],
        )
        .unwrap();
    drop(connection);

    let mut instance = crate::workflow_ir::ExecutionInstance {
        id: "wfi-reconcile".to_string(),
        workflow_id: "workflow-reconcile".to_string(),
        workflow_version: 1,
        status: ExecutionStatus::AwaitingApproval,
        active_node_id: Some("send-mail".to_string()),
        input_payload: json!({}),
        output_payload: None,
        node_payloads: std::collections::HashMap::new(),
        memory: std::collections::HashMap::new(),
        selected_edges: std::collections::HashSet::new(),
        pause_context: None,
        error: None,
        execution_latency_ms: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        created_at_ms: now,
        started_at_ms: Some(now),
        updated_at_ms: now,
        completed_at_ms: None,
    };
    engine.insert_execution_instance(&instance).unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO routine_runs(schedule_id,execution_instance_id,scheduled_for_ms,created_at_ms) VALUES ('routine_reconcile',?1,?2,?3)",
            rusqlite::params![instance.id, now - 1_000, now],
        )
        .unwrap();

    let awaiting = workflow_runtime::RunWorkflowResponse {
        instance: instance.clone(),
        execution_order: vec!["calendar".to_string()],
        approval_request: None,
        completion: None,
    };
    let schedule = reconcile_routine_records_after_permission(&engine, &awaiting)
        .unwrap()
        .unwrap();
    assert!(schedule.is_active);
    assert_eq!(schedule.last_status.as_deref(), Some("AwaitingApproval"));
    let awaiting_task_state: String = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT state FROM task_runs WHERE runtime_kind='workflow' AND runtime_record_id=?1",
            rusqlite::params![instance.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(awaiting_task_state, "awaiting_approval");

    instance.status = ExecutionStatus::Completed;
    instance.active_node_id = None;
    instance.output_payload = Some(json!({"status":"completed"}));
    instance.updated_at_ms = now + 1;
    instance.completed_at_ms = Some(now + 1);
    engine.update_execution_instance(&instance).unwrap();
    let completed = workflow_runtime::RunWorkflowResponse {
        instance: instance.clone(),
        execution_order: vec!["send-mail".to_string(), "output".to_string()],
        approval_request: None,
        completion: Some(workflow_runtime::WorkflowCompletion {
            kind: WorkflowCompletionKind::Result,
        }),
    };
    let schedule = reconcile_routine_records_after_permission(&engine, &completed)
        .unwrap()
        .unwrap();
    assert!(!schedule.is_active);
    assert_eq!(schedule.last_status.as_deref(), Some("Completed"));
    let connection = engine.open_connection().unwrap();
    let (task_run_id, task_state): (String, String) = connection
        .query_row(
            "SELECT task_run_id,state FROM task_runs WHERE runtime_kind='workflow' AND runtime_record_id=?1",
            rusqlite::params![instance.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let linked_task_run_id: String = connection
        .query_row(
            "SELECT task_run_id FROM routine_runs WHERE execution_instance_id=?1",
            rusqlite::params![instance.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(task_state, "completed");
    assert_eq!(linked_task_run_id, task_run_id);
    drop(connection);

    let markdown = root.join("operations_brief_2026-07-21_1200.md");
    let pdf = root.join("operations_brief_2026-07-21_1200.pdf");
    std::fs::write(&markdown, b"verified markdown").unwrap();
    std::fs::write(&pdf, b"verified pdf").unwrap();
    let connection = engine.open_connection().unwrap();
    for (sequence, path) in [&markdown, &pdf].into_iter().enumerate() {
        let bytes = std::fs::read(path).unwrap();
        let event = json!({
            "eventType":"file.created",
            "evidenceClass":"verified_postcondition",
            "payload":{
                "path":path.to_string_lossy(),
                "sha256":crate::foundation::digest::sha256_hex(&bytes),
                "byteLength":bytes.len()
            }
        });
        connection
            .execute(
                "INSERT INTO task_events(task_run_id,sequence,event_json,created_at_ms) VALUES (?1,?2,?3,?4)",
                rusqlite::params![task_run_id, sequence as i64, event.to_string(), now],
            )
            .unwrap();
    }
    drop(connection);
    let copy = SchedulerCopy::english();
    let delivery_body = verified_routine_delivery_body(
        &engine,
        Some(&instance.id),
        ExecutionStatus::Completed,
        "Operations brief",
        "generic completion",
        &copy.delivery_completed_verified,
        &copy.delivery_completed_declined_verified,
        &copy.delivery_failed_verified,
        &[],
    );
    assert_eq!(
        delivery_body,
        "Operations brief completed successfully. Verified files: operations_brief_2026-07-21_1200.md, operations_brief_2026-07-21_1200.pdf."
    );

    let first = reserve_routine_delivery(
        &engine,
        &schedule,
        Some(&instance.id),
        Some(task_run_id.clone()),
        "signal",
        "destination-hash",
        "completed",
    )
    .unwrap();
    let second = reserve_routine_delivery(
        &engine,
        &schedule,
        Some(&instance.id),
        Some(task_run_id),
        "signal",
        "destination-hash",
        "completed",
    )
    .unwrap();
    assert!(matches!(first, RoutineDeliveryReservationState::Send(_)));
    assert!(matches!(second, RoutineDeliveryReservationState::Send(_)));
    let connection = engine.open_connection().unwrap();
    let receipts: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM routine_delivery_receipts WHERE schedule_id='routine_reconcile' AND event_kind='completed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let effects: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM task_effects WHERE effect_kind='routine_channel_delivery'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 1);
    assert_eq!(effects, 1);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn terminal_delivery_destination_drift_fails_before_reservation() {
    let root = std::env::temp_dir().join(format!(
        "oomu-routine-delivery-authority-{}",
        unix_time_ms()
    ));
    let project_root = root.join("project");
    std::fs::create_dir_all(&project_root).unwrap();
    let project_root = std::fs::canonicalize(project_root).unwrap();
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let now = unix_time_ms();
    let project_id = "project_00000000-0000-4000-8000-000000000654";
    let workflow_id = "workflow-delivery-authority";
    let schedule_id = "routine_delivery_authority";
    let instance_id = "wfi-delivery-authority";
    let destination = "C123";
    let drifted_destination = "C999";
    let ir = json!({
        "schemaVersion": crate::workflow_ir::WORKFLOW_IR_SCHEMA_VERSION,
        "workflowId": workflow_id,
        "workflowVersion": 1,
        "name": "Delivery authority",
        "compiler": {"model": crate::workflow_ir::WORKFLOW_COMPILER_MODEL},
        "nodes": [],
        "edges": []
    });
    let authority = json!({
        "schemaVersion": 1,
        "mode": "reviewed_workflow_scope",
        "scheduleId": schedule_id,
        "workflowId": workflow_id,
        "workflowVersion": 1,
        "projectId": project_id,
        "projectRoots": [project_root.to_string_lossy()],
        "nodes": [],
        "terminalDelivery": {
            "platform": "slack",
            "destinationSha256": crate::foundation::digest::sha256_hex(destination.as_bytes())
        }
    });
    let connection = engine.open_connection().unwrap();
    connection.execute(
        "INSERT INTO projects(project_id,name,description,created_at_ms,updated_at_ms) VALUES (?1,'Delivery authority','',?2,?2)",
        rusqlite::params![project_id, now],
    ).unwrap();
    connection.execute(
        "INSERT INTO project_sources(source_id,project_id,source_kind,canonical_path,grant_reference,grant_state,created_at_ms,updated_at_ms) VALUES ('source-delivery-authority',?1,'local_folder',?2,'grant-delivery','active',?3,?3)",
        rusqlite::params![project_id, project_root.to_string_lossy(), now],
    ).unwrap();
    connection.execute(
        "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,workflow_ir_json,compilation_status,is_active,created_at_ms,updated_at_ms,compiled_at_ms,project_id) VALUES (?1,1,'Delivery authority','','{}',?2,'Compiled',1,?3,?3,?3,?4)",
        rusqlite::params![workflow_id, ir.to_string(), now, project_id],
    ).unwrap();
    connection.execute(
        "INSERT INTO workflow_schedules(id,workflow_id,workflow_version,label,schedule_expression,run_request_json,is_active,created_at_ms,updated_at_ms,project_id,schedule_kind,routine_timezone,delivery_target_json,authority_json) VALUES (?1,?2,1,'Delivery authority','manual','{}',1,?3,?3,?4,'one_shot','UTC',?5,?6)",
        rusqlite::params![schedule_id, workflow_id, now, project_id, json!({"platform":"slack","destination":drifted_destination}).to_string(), authority.to_string()],
    ).unwrap();
    connection.execute(
        "INSERT INTO execution_instances(id,workflow_id,workflow_version,status,created_at_ms,updated_at_ms,project_id) VALUES (?1,?2,1,'Completed',?3,?3,?4)",
        rusqlite::params![instance_id, workflow_id, now, project_id],
    ).unwrap();
    connection.execute(
        "INSERT INTO routine_runs(schedule_id,execution_instance_id,scheduled_for_ms,created_at_ms) VALUES (?1,?2,?3,?3)",
        rusqlite::params![schedule_id, instance_id, now],
    ).unwrap();
    drop(connection);

    let schedule = engine.load_workflow_schedule(schedule_id).unwrap();
    let rejected = reserve_authorized_routine_delivery(
        &engine,
        &schedule,
        Some(instance_id),
        None,
        "slack",
        drifted_destination,
        "completed",
    )
    .unwrap_err();
    assert!(rejected.contains("authority no longer matches"));
    let connection = engine.open_connection().unwrap();
    let receipts: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM routine_delivery_receipts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let effects: i64 = connection
        .query_row("SELECT COUNT(*) FROM task_effects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(receipts, 0);
    assert_eq!(effects, 0);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn offline_failure_requeues_and_restored_run_reuses_one_logical_occurrence() {
    let root = std::env::temp_dir().join(format!("oomu-routine-offline-retry-{}", unix_time_ms()));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let now = unix_time_ms();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,workflow_ir_json,is_active,created_at_ms,updated_at_ms,compiled_at_ms) VALUES ('workflow-offline',1,'Offline','','{}',NULL,1,?1,?1,?1)",
            rusqlite::params![now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO workflow_schedules(id,workflow_id,workflow_version,label,schedule_expression,run_request_json,is_active,next_run_at_ms,created_at_ms,updated_at_ms,schedule_kind,routine_timezone,last_status) VALUES ('routine_offline','workflow-offline',1,'Offline','manual','{}',1,NULL,?1,?1,'one_shot','UTC','Running')",
            rusqlite::params![now],
        )
        .unwrap();
    drop(connection);
    let mut instance = crate::workflow_ir::ExecutionInstance {
        id: "wfi-offline".to_string(),
        workflow_id: "workflow-offline".to_string(),
        workflow_version: 1,
        status: ExecutionStatus::Failed,
        active_node_id: None,
        input_payload: json!({}),
        output_payload: None,
        node_payloads: std::collections::HashMap::new(),
        memory: std::collections::HashMap::new(),
        selected_edges: std::collections::HashSet::new(),
        pause_context: None,
        error: Some(json!({
            "code":"workflow_runtime_execution_failed",
            "message":"MCP tool local_search / search_web returned an error result: network_unavailable"
        })),
        execution_latency_ms: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        created_at_ms: now,
        started_at_ms: Some(now),
        updated_at_ms: now,
        completed_at_ms: Some(now),
    };
    engine.insert_execution_instance(&instance).unwrap();
    let task_id = crate::p0_contracts::TaskId::new().to_string();
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO task_runs(task_run_id,task_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,'workflow',?3,'failed','routine',?2,'Offline',?4,?4,'reconciled')",
            rusqlite::params![task_run_id, task_id, instance.id, now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO routine_runs(schedule_id,execution_instance_id,task_run_id,scheduled_for_ms,created_at_ms) VALUES ('routine_offline',?1,?2,?3,?4)",
            rusqlite::params![instance.id, task_run_id, now - 5_000, now],
        )
        .unwrap();
    drop(connection);
    let response = workflow_runtime::RunWorkflowResponse {
        instance: instance.clone(),
        execution_order: vec![],
        approval_request: None,
        completion: None,
    };
    let schedule = engine.load_workflow_schedule("routine_offline").unwrap();
    assert!(requeue_transient_failure(&engine, &schedule, &response).unwrap());
    let requeued = engine.load_workflow_schedule("routine_offline").unwrap();
    assert!(requeued.is_active);
    assert_eq!(requeued.last_status.as_deref(), Some("Pending"));
    assert!(requeued.next_run_at_ms.is_some_and(|retry| retry > now));
    assert_eq!(
        retryable_instance_for_claim(&engine, &requeued).unwrap(),
        Some(instance.id.clone())
    );
    let connection = engine.open_connection().unwrap();
    let (retried_task_run_id, retried_task_id, retried_state): (String, String, String) = connection
        .query_row(
            "SELECT task_run_id,task_id,state FROM task_runs WHERE runtime_kind='workflow' AND runtime_record_id=?1",
            rusqlite::params![instance.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let routine_task_run_id: String = connection
        .query_row(
            "SELECT task_run_id FROM routine_runs WHERE schedule_id='routine_offline'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retried_task_run_id, task_run_id);
    assert_eq!(retried_task_id, task_id);
    assert_eq!(retried_state, "running");
    assert_eq!(routine_task_run_id, task_run_id);
    drop(connection);

    instance.status = ExecutionStatus::Completed;
    instance.error = None;
    instance.output_payload = Some(json!({"status":"completed"}));
    instance.updated_at_ms = now + 1;
    instance.completed_at_ms = Some(now + 1);
    engine.update_execution_instance(&instance).unwrap();
    let completed = workflow_runtime::RunWorkflowResponse {
        instance: instance.clone(),
        execution_order: vec!["output".to_string()],
        approval_request: None,
        completion: Some(workflow_runtime::WorkflowCompletion {
            kind: WorkflowCompletionKind::Result,
        }),
    };
    let terminal = reconcile_routine_records_after_permission(&engine, &completed)
        .unwrap()
        .unwrap();
    assert!(!terminal.is_active);
    let connection = engine.open_connection().unwrap();
    let logical_runs: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM routine_runs WHERE schedule_id='routine_offline'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let deliveries: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM routine_delivery_receipts WHERE schedule_id='routine_offline'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(logical_runs, 1);
    assert_eq!(deliveries, 0);
    let (terminal_task_run_id, terminal_task_id, terminal_state): (String, String, String) = connection
        .query_row(
            "SELECT task_run_id,task_id,state FROM task_runs WHERE runtime_kind='workflow' AND runtime_record_id=?1",
            rusqlite::params![instance.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(terminal_task_run_id, task_run_id);
    assert_eq!(terminal_task_id, task_id);
    assert_eq!(terminal_state, "completed");
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn terminal_delivery_retries_only_before_dispatch_and_never_creates_a_second_receipt() {
    let root =
        std::env::temp_dir().join(format!("oomu-routine-terminal-delivery-{}", unix_time_ms()));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let now = unix_time_ms();
    let task_id = crate::p0_contracts::TaskId::new().to_string();
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO workflow_schedules(id,workflow_id,label,schedule_expression,run_request_json,is_active,created_at_ms,updated_at_ms,schedule_kind,routine_timezone,delivery_target_json) VALUES ('routine_delivery_state','workflow-delivery-state','Delivery state','manual','{}',1,?1,?1,'one_shot','UTC','{\"platform\":\"signal\",\"destination\":\"owner\"}')",
            rusqlite::params![now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,is_active,created_at_ms,updated_at_ms) VALUES ('workflow-delivery-state',1,'Delivery state','','{}',1,?1,?1)",
            rusqlite::params![now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO execution_instances(id,workflow_id,workflow_version,status,created_at_ms,updated_at_ms) VALUES ('wfi-delivery-state','workflow-delivery-state',1,'Completed',?1,?1)",
            rusqlite::params![now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO task_runs(task_run_id,task_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,'workflow','wfi-delivery-state','completed','routine',?2,'Delivery state',?3,?3,'reconciled')",
            rusqlite::params![task_run_id, task_id, now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO routine_runs(schedule_id,execution_instance_id,task_run_id,scheduled_for_ms,created_at_ms) VALUES ('routine_delivery_state','wfi-delivery-state',?1,?2,?2)",
            rusqlite::params![task_run_id, now],
        )
        .unwrap();
    drop(connection);
    let schedule = engine
        .load_workflow_schedule("routine_delivery_state")
        .unwrap();

    let first = reserve_routine_delivery(
        &engine,
        &schedule,
        Some("wfi-delivery-state"),
        Some(task_run_id.clone()),
        "signal",
        "destination-hash",
        "completed",
    )
    .unwrap();
    let RoutineDeliveryReservationState::Send(first) = first else {
        panic!("first terminal delivery should reserve the exact effect");
    };
    mark_routine_delivery_dispatched(&engine, &first).unwrap();
    fail_routine_delivery(&engine, &first, "signal_outbound_channel_unavailable", true).unwrap();
    apply_terminal_delivery_outcome(
        &engine,
        &schedule,
        "wfi-delivery-state",
        &RoutineDeliveryOutcome::RetryableFailure {
            error_code: "signal_outbound_channel_unavailable".to_string(),
        },
        &SchedulerCopy::english(),
    )
    .unwrap();
    let (pending_status, pending_task_state): (String, String) = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT s.last_status,t.state FROM workflow_schedules s JOIN routine_runs r ON r.schedule_id=s.id JOIN task_runs t ON t.task_run_id=r.task_run_id WHERE s.id='routine_delivery_state'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(pending_status, "Pending");
    assert_eq!(pending_task_state, "blocked");
    assert!(
        retryable_terminal_delivery(&engine, unix_time_ms() + 31_000, 30_000)
            .unwrap()
            .is_some()
    );

    let retry = reserve_routine_delivery(
        &engine,
        &schedule,
        Some("wfi-delivery-state"),
        Some(task_run_id.clone()),
        "signal",
        "destination-hash",
        "completed",
    )
    .unwrap();
    let RoutineDeliveryReservationState::Send(retry) = retry else {
        panic!("a proven pre-dispatch failure should reuse its reservation");
    };
    assert_eq!(retry.receipt_id, first.receipt_id);
    mark_routine_delivery_dispatched(&engine, &retry).unwrap();
    finish_routine_delivery(&engine, &retry, "provider-receipt").unwrap();
    apply_terminal_delivery_outcome(
        &engine,
        &schedule,
        "wfi-delivery-state",
        &RoutineDeliveryOutcome::Delivered,
        &SchedulerCopy::english(),
    )
    .unwrap();
    assert!(matches!(
        reserve_routine_delivery(
            &engine,
            &schedule,
            Some("wfi-delivery-state"),
            Some(task_run_id.clone()),
            "signal",
            "destination-hash",
            "completed",
        )
        .unwrap(),
        RoutineDeliveryReservationState::AlreadyDelivered
    ));
    let connection = engine.open_connection().unwrap();
    let (receipts, effect_state, final_status, final_task_state): (i64, String, String, String) = connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM routine_delivery_receipts WHERE schedule_id='routine_delivery_state'),(SELECT state FROM task_effects WHERE task_run_id=?1 AND effect_kind='routine_channel_delivery'),(SELECT last_status FROM workflow_schedules WHERE id='routine_delivery_state'),(SELECT state FROM task_runs WHERE task_run_id=?1)",
            rusqlite::params![task_run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(receipts, 1);
    assert_eq!(effect_state, "verified");
    assert_eq!(final_status, "Completed");
    assert_eq!(final_task_state, "completed");
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn terminal_delivery_retries_only_proven_pre_dispatch_transport_failures() {
    use crate::workflow_scheduler::delivery::routine_delivery_failure_is_safe_to_retry;

    assert!(routine_delivery_failure_is_safe_to_retry(
        "signal_outbound_channel_unavailable"
    ));
    assert!(routine_delivery_failure_is_safe_to_retry(
        "whatsapp_outbound_queue_unavailable: offline"
    ));
    for permanent_or_uncertain in [
        "routine_delivery_owner_mismatch",
        "routine_delivery_destination_not_authorized_owner",
        "routine_delivery_channel_inactive",
        "routine_delivery_platform_unsupported",
        "signal_send_acknowledgement_timeout",
        "keychain_secret_unavailable",
    ] {
        assert!(
            !routine_delivery_failure_is_safe_to_retry(permanent_or_uncertain),
            "{permanent_or_uncertain} must require review"
        );
    }
}

#[test]
fn uncertain_terminal_delivery_waits_for_an_explicit_absence_confirmation() {
    let root =
        std::env::temp_dir().join(format!("oomu-routine-terminal-review-{}", unix_time_ms()));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let now = unix_time_ms();
    let task_id = crate::p0_contracts::TaskId::new().to_string();
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO workflow_schedules(id,workflow_id,label,schedule_expression,run_request_json,is_active,created_at_ms,updated_at_ms,schedule_kind,routine_timezone,delivery_target_json) VALUES ('routine_delivery_review','workflow-delivery-review','Delivery review','manual','{}',1,?1,?1,'one_shot','UTC','{\"platform\":\"signal\",\"destination\":\"owner\"}')",
            rusqlite::params![now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,is_active,created_at_ms,updated_at_ms) VALUES ('workflow-delivery-review',1,'Delivery review','','{}',1,?1,?1)",
            rusqlite::params![now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO execution_instances(id,workflow_id,workflow_version,status,created_at_ms,updated_at_ms) VALUES ('wfi-delivery-review','workflow-delivery-review',1,'Completed',?1,?1)",
            rusqlite::params![now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO task_runs(task_run_id,task_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,'workflow','wfi-delivery-review','completed','routine',?2,'Delivery review',?3,?3,'reconciled')",
            rusqlite::params![task_run_id, task_id, now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO routine_runs(schedule_id,execution_instance_id,task_run_id,scheduled_for_ms,created_at_ms) VALUES ('routine_delivery_review','wfi-delivery-review',?1,?2,?2)",
            rusqlite::params![task_run_id, now],
        )
        .unwrap();
    drop(connection);
    let schedule = engine
        .load_workflow_schedule("routine_delivery_review")
        .unwrap();
    let reservation = reserve_routine_delivery(
        &engine,
        &schedule,
        Some("wfi-delivery-review"),
        Some(task_run_id.clone()),
        "signal",
        "destination-hash",
        "completed",
    )
    .unwrap();
    let RoutineDeliveryReservationState::Send(reservation) = reservation else {
        panic!("terminal delivery should reserve the exact effect");
    };
    mark_routine_delivery_dispatched(&engine, &reservation).unwrap();
    let acknowledgement_error = "signal_send_acknowledgement_timeout";
    assert!(
        !crate::workflow_scheduler::delivery::routine_delivery_failure_is_safe_to_retry(
            acknowledgement_error
        )
    );
    fail_routine_delivery(&engine, &reservation, acknowledgement_error, false).unwrap();
    // signal-cli received the request but did not return a correlated provider
    // receipt. This ambiguous boundary must never auto-resend.
    assert!(retryable_terminal_delivery(&engine, now + 31_000, 30_000)
        .unwrap()
        .is_none());
    assert!(matches!(
        reserve_routine_delivery(
            &engine,
            &schedule,
            Some("wfi-delivery-review"),
            Some(task_run_id),
            "signal",
            "destination-hash",
            "completed",
        )
        .unwrap(),
        RoutineDeliveryReservationState::NeedsReview
    ));
    apply_terminal_delivery_outcome(
        &engine,
        &schedule,
        "wfi-delivery-review",
        &RoutineDeliveryOutcome::NeedsReview {
            error_code: "routine_delivery_confirmation_required".to_string(),
        },
        &SchedulerCopy::english(),
    )
    .unwrap();
    let review_status: String = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT last_status FROM workflow_schedules WHERE id='routine_delivery_review'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(review_status, "AwaitingApproval");
    confirm_terminal_delivery_absent_and_retry(&engine, "routine_delivery_review").unwrap();
    assert!(retryable_terminal_delivery(&engine, unix_time_ms(), 30_000)
        .unwrap()
        .is_some());
    let delivery_state: String = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT CASE WHEN d.state IN ('pending','failed') AND e.state='reserved' THEN 'retrying' ELSE 'unexpected' END FROM routine_delivery_receipts d JOIN task_effects e ON e.task_run_id=d.task_run_id AND e.effect_kind='routine_channel_delivery' WHERE d.schedule_id='routine_delivery_review' ORDER BY d.created_at_ms DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(delivery_state, "retrying");
    std::fs::remove_dir_all(root).unwrap();
}
