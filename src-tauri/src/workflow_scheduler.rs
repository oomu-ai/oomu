#[cfg(test)]
use crate::schedule_expression::local_datetime;
use crate::schedule_expression::{
    is_manual_expression, next_run_after, next_run_after_in_timezone,
};
use crate::{
    db::{PersistenceEngine, WorkflowScheduleRecord, WorkflowScheduleUpsert},
    foundation::clock::unix_time_ms_i64 as unix_time_ms,
    gateway::SovereignGatewayService,
    gemma::GemmaService,
    knowledge::KnowledgeStore,
    mcp::client::McpClientRegistry,
    p0_contracts::EvidenceClass,
    settings,
    workflow_ir::{ExecutionInstance, ExecutionStatus, WorkflowCompletionKind, WorkflowNode},
    workflow_runtime,
};
#[cfg(test)]
use chrono::{Datelike, Local};
use chrono::{TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use rand_core::RngCore;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::{sync::OnceLock, time::Duration};
use tauri::Manager;

mod delivery;
mod notification;
mod projection;
mod recurrence;
mod retry;
mod runtime;
pub(crate) use delivery::confirm_terminal_delivery_absent_and_retry;
use delivery::{deliver_routine_notice, retryable_terminal_delivery, RoutineDeliveryOutcome};
#[cfg(test)]
use delivery::{
    fail_routine_delivery, finish_routine_delivery, mark_routine_delivery_dispatched,
    reserve_authorized_routine_delivery, reserve_routine_delivery, routine_notice_copy,
    verified_routine_delivery_body, RoutineDeliveryReservationState,
};
#[cfg(test)]
use notification::background_notice_copy;
use notification::{
    notify_background_event, notify_for_run_status, render_scheduler_copy, SchedulerCopy,
};
use projection::{project_workflow_task, scheduled_approval};
use recurrence::*;
use retry::{requeue_transient_failure, retryable_instance_for_claim};
pub(crate) use runtime::WorkflowSchedulerRuntime;

const WORKER_NAME: &str = "oomu-background-workflow-scheduler";
const POLL_INTERVAL: Duration = Duration::from_secs(60);
const CLAIM_LEASE_MS: i64 = 30 * 60 * 1000;
const MAX_SCHEDULES_PER_POLL: usize = 1;
const DELIVERY_RETRY_BACKOFF_MS: i64 = 30_000;
macro_rules! scheduler_try {
    ($result:expr) => {
        match $result {
            Ok(value) => value,
            Err(error) => return Err(error),
        }
    };
}

#[tauri::command]
pub async fn resolve_workflow_permission(
    request: workflow_runtime::ResolvePermissionRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    gemma: tauri::State<'_, GemmaService>,
    _knowledge: tauri::State<'_, KnowledgeStore>,
    mcp_registry: tauri::State<'_, McpClientRegistry>,
    gateway: tauri::State<'_, SovereignGatewayService>,
    app: tauri::AppHandle,
) -> Result<workflow_runtime::RunWorkflowResponse, workflow_runtime::WorkflowRuntimeError> {
    let persistence = persistence.inner().clone();
    let response = workflow_runtime::resolve_workflow_permission_without_reconciliation(
        request,
        persistence.clone(),
        gemma.inner().clone(),
        mcp_registry.inner().clone(),
        app.clone(),
    )
    .await?;
    reconcile_routine_after_permission(&app, &persistence, gateway.inner(), &response)
        .map_err(workflow_runtime::WorkflowRuntimeError::runtime)?;
    workflow_runtime::dispatch_approval_request(&app, response.approval_request.as_ref());
    Ok(response)
}

pub(crate) fn resolve_scheduled_permission(
    request: workflow_runtime::ResolvePermissionRequest,
    persistence: &PersistenceEngine,
    gemma: GemmaService,
    mcp_registry: McpClientRegistry,
    app: tauri::AppHandle,
) -> Result<workflow_runtime::RunWorkflowResponse, workflow_runtime::WorkflowRuntimeError> {
    let response = workflow_runtime::resolve_scheduled_permission_without_reconciliation(
        request,
        persistence,
        gemma,
        mcp_registry,
        app.clone(),
    )?;
    let gateway = app.state::<SovereignGatewayService>().inner().clone();
    reconcile_routine_after_permission(&app, persistence, &gateway, &response)
        .map_err(workflow_runtime::WorkflowRuntimeError::runtime)?;
    workflow_runtime::dispatch_approval_request(&app, response.approval_request.as_ref());
    Ok(response)
}

#[tauri::command]
pub async fn retry_routine_delivery(
    request: crate::routines::RetryRoutineDeliveryRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<crate::routines::RoutineRecord, String> {
    if !request.confirmed_absent {
        return Err("routine_delivery_confirmation_required".to_string());
    }
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::routines::repository::get(&engine, &request.routine_id)?;
        confirm_terminal_delivery_absent_and_retry(&engine, &request.routine_id)?;
        crate::routines::repository::get(&engine, &request.routine_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub fn spawn_background_worker(
    app: tauri::AppHandle,
    persistence: PersistenceEngine,
    gemma: GemmaService,
    knowledge: KnowledgeStore,
    mcp_registry: McpClientRegistry,
    gateway: SovereignGatewayService,
) -> Result<WorkflowSchedulerRuntime, String> {
    WorkflowSchedulerRuntime::spawn(WORKER_NAME, POLL_INTERVAL, move || {
        let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_scheduler_tick(
                &app,
                &persistence,
                gemma.clone(),
                knowledge.clone(),
                mcp_registry.clone(),
                gateway.clone(),
            )
        }));
        match tick_result {
            Ok(Ok(())) => {
                app.state::<crate::DegradedModeState>()
                    .clear_after_verified_recovery(
                        "workflowScheduler",
                        crate::persistence_health::BackingStoreClass::NotApplicable,
                        "Scheduler thread and persistence-backed tick completed successfully.",
                    );
            }
            Ok(Err(error)) => {
                eprintln!(
                    "WORKFLOW_SCHEDULER_TICK_FAILED {}",
                    crate::redaction::redacted_log_text(&error.to_string())
                );
                app.state::<crate::DegradedModeState>().activate(
                    "workflowScheduler",
                    format!("Workflow scheduler tick failed: {error}"),
                    crate::persistence_health::BackingStoreClass::NotApplicable,
                    true,
                    "Scheduled workflows are paused until a scheduler probe succeeds.",
                );
            }
            Err(payload) => {
                let message = crate::panic_payload_message(payload);
                eprintln!(
                    "WORKFLOW_SCHEDULER_TICK_PANICKED {}",
                    crate::redaction::redacted_log_text(&message)
                );
                app.state::<crate::DegradedModeState>().activate(
                    "workflowScheduler",
                    format!("Workflow scheduler tick panicked: {message}"),
                    crate::persistence_health::BackingStoreClass::NotApplicable,
                    true,
                    "Scheduled workflows are paused until a scheduler probe succeeds.",
                );
            }
        }
        if let Err(error) = persistence.run_sqlite_maintenance_if_due(unix_time_ms()) {
            eprintln!(
                "SQLITE_MAINTENANCE_FAILED {}",
                crate::redaction::redacted_log_text(&error.to_string())
            );
        }
    })
}

pub fn sync_workflow_schedule_from_visual_state(
    persistence: &PersistenceEngine,
    workflow_id: &str,
    workflow_version: u32,
    workflow_name: &str,
    visual_state: &Value,
    activate: bool,
) -> Result<(), String> {
    let schedule_id = default_schedule_id(workflow_id);
    let Some(expression) = schedule_expression_from_visual_state(visual_state) else {
        persistence
            .disable_workflow_schedule(&schedule_id)
            .map_err(|error| error.to_string())?;
        return Ok(());
    };

    if !activate {
        persistence
            .disable_workflow_schedule(&schedule_id)
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    let next_run_at_ms = next_run_after(&expression, unix_time_ms())?;
    persistence
        .upsert_workflow_schedule(WorkflowScheduleUpsert {
            id: schedule_id,
            workflow_id: workflow_id.to_string(),
            workflow_version: Some(workflow_version),
            label: workflow_name.trim().to_string(),
            schedule_expression: expression,
            run_request: json!({}),
            is_active: true,
            next_run_at_ms: Some(next_run_at_ms),
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn run_scheduler_tick(
    app: &tauri::AppHandle,
    persistence: &PersistenceEngine,
    gemma: GemmaService,
    knowledge: KnowledgeStore,
    mcp_registry: McpClientRegistry,
    gateway: SovereignGatewayService,
) -> Result<(), String> {
    let owner = scheduler_owner_from_visibility(
        app.get_webview_window("main")
            .and_then(|window| window.is_visible().ok()),
    );
    if !crate::routines::background::scheduled_work_allowed(persistence, owner) {
        return Ok(());
    }
    if !acquire_scheduler_lease(persistence)? {
        return Ok(());
    }
    retry_pending_terminal_delivery(app, persistence, &gateway)?;
    let now = unix_time_ms();
    finalize_due_routines_at_end_boundary(persistence, now)?;
    let due = persistence
        .claim_due_workflow_schedules(now, MAX_SCHEDULES_PER_POLL, CLAIM_LEASE_MS)
        .map_err(|error| error.to_string())?;

    for schedule in due {
        if should_skip_claimed_routine(persistence, &schedule, now)? {
            continue;
        }
        for occurrence in claimed_occurrences_at(&schedule, now)? {
            if advance_if_recorded(persistence, &occurrence)? {
                continue;
            }
            run_claimed_schedule(
                app,
                persistence,
                gemma.clone(),
                knowledge.clone(),
                mcp_registry.clone(),
                gateway.clone(),
                occurrence.schedule,
                occurrence.next_run_at_ms,
            )?;
        }
    }
    Ok(())
}

fn scheduler_owner_from_visibility(
    visible: Option<bool>,
) -> crate::routines::background::ScheduledWorkOwner {
    match visible {
        Some(true) => crate::routines::background::ScheduledWorkOwner::ForegroundApplication,
        Some(false) | None => crate::routines::background::ScheduledWorkOwner::DetachedRuntime,
    }
}

fn retry_pending_terminal_delivery(
    app: &tauri::AppHandle,
    persistence: &PersistenceEngine,
    gateway: &SovereignGatewayService,
) -> Result<(), String> {
    let Some((schedule, instance_id)) =
        retryable_terminal_delivery(persistence, unix_time_ms(), DELIVERY_RETRY_BACKOFF_MS)?
    else {
        return Ok(());
    };
    let instance = persistence
        .load_execution_instance(&instance_id)
        .map_err(|error| error.to_string())?;
    if instance.status != ExecutionStatus::Completed {
        return Err("Routine delivery retry requires a completed workflow instance.".to_string());
    }
    let copy = SchedulerCopy::load(persistence);
    let completion_kind = completion_kind_for_instance(persistence, &instance);
    let declined_actions = declined_actions_for_instance(persistence, &instance)?;
    let outcome = deliver_routine_notice(
        persistence,
        gateway,
        &schedule,
        Some(&instance_id),
        ExecutionStatus::Completed,
        completion_kind,
        None,
        None,
        &copy,
        &declined_actions,
    );
    apply_terminal_delivery_outcome(persistence, &schedule, &instance_id, &outcome, &copy)?;
    match outcome {
        RoutineDeliveryOutcome::Delivered | RoutineDeliveryOutcome::NotRequired => {
            notify_for_run_status(
                app,
                &schedule,
                ExecutionStatus::Completed,
                completion_kind,
                None,
                &copy,
                &declined_actions,
            );
        }
        RoutineDeliveryOutcome::NeedsReview { .. } => notify_background_event(
            app,
            &copy.delivery_review_title,
            &render_scheduler_copy(
                &copy.delivery_review_body,
                &[("name", &schedule_title(&schedule))],
            ),
        ),
        RoutineDeliveryOutcome::RetryableFailure { .. } => {}
    }
    Ok(())
}

fn should_skip_claimed_routine(
    persistence: &PersistenceEngine,
    schedule: &WorkflowScheduleRecord,
    now: i64,
) -> Result<bool, String> {
    if !schedule.id.starts_with("routine_") {
        return Ok(false);
    }
    if crate::routines::control::end_at_ms(&schedule.run_request)?
        .is_some_and(|end_at_ms| now >= end_at_ms)
    {
        finish_routine_at_end_boundary(persistence, &schedule.id)?;
        return Ok(true);
    }
    if crate::routines::control::run_now_resume_at_ms(&schedule.run_request)?.is_some() {
        return Ok(false);
    }
    let zone: Tz = schedule
        .routine_timezone
        .parse()
        .map_err(|_| "Routine timezone is invalid.".to_string())?;
    if let (Some(start), Some(end)) = (
        schedule.active_window_start_minute,
        schedule.active_window_end_minute,
    ) {
        let local = Utc
            .timestamp_millis_opt(now)
            .single()
            .ok_or_else(|| "Invalid routine time.".to_string())?
            .with_timezone(&zone);
        let minute = (local.hour() * 60 + local.minute()) as u16;
        let inside = if start <= end {
            minute >= start && minute <= end
        } else {
            minute >= start || minute <= end
        };
        if !inside {
            release_claimed_at_next_future_occurrence(
                persistence,
                schedule,
                now,
                "Outside the routine active window",
            )?;
            return Ok(true);
        }
    }
    let lateness = schedule
        .next_run_at_ms
        .map(|due| now.saturating_sub(due))
        .unwrap_or_default();
    if lateness > 90_000 && schedule.missed_run_policy == "skip" {
        release_claimed_at_next_future_occurrence(
            persistence,
            schedule,
            now,
            "Skipped a missed run by policy",
        )?;
        return Ok(true);
    }
    Ok(false)
}

fn acquire_scheduler_lease(engine: &PersistenceEngine) -> Result<bool, String> {
    static OWNER: OnceLock<String> = OnceLock::new();
    let owner = OWNER.get_or_init(|| {
        format!(
            "scheduler:{}:{}",
            std::process::id(),
            crate::p0_contracts::TaskId::new()
        )
    });
    let now = unix_time_ms();
    let kind = engine
        .open_connection()
        .ok()
        .and_then(|connection| {
            connection
                .query_row(
                    "SELECT runtime_state='on_verified' FROM background_service_state WHERE singleton=1",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .optional()
                .ok()
                .flatten()
        })
        .unwrap_or(false)
        .then_some("background_service")
        .unwrap_or("foreground");
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let current:Option<(String,i64,i64)>=transaction.query_row("SELECT owner_id,lease_epoch,expires_at_ms FROM scheduler_owner_lease WHERE singleton=1",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|error|error.to_string())?;
    if current
        .as_ref()
        .is_some_and(|(existing, _, expires)| existing != owner && *expires > now)
    {
        return Ok(false);
    }
    let epoch = current.map(|(_, epoch, _)| epoch + 1).unwrap_or(1);
    transaction.execute("INSERT INTO scheduler_owner_lease (singleton,owner_id,owner_kind,lease_epoch,acquired_at_ms,heartbeat_at_ms,expires_at_ms) VALUES (1,?1,?2,?3,?4,?4,?5) ON CONFLICT(singleton) DO UPDATE SET owner_id=excluded.owner_id,owner_kind=excluded.owner_kind,lease_epoch=excluded.lease_epoch,acquired_at_ms=CASE WHEN scheduler_owner_lease.owner_id=excluded.owner_id THEN scheduler_owner_lease.acquired_at_ms ELSE excluded.acquired_at_ms END,heartbeat_at_ms=excluded.heartbeat_at_ms,expires_at_ms=excluded.expires_at_ms",rusqlite::params![owner,kind,epoch,now,now+90_000]).map_err(|error|error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(true)
}

fn release_claimed_without_run(
    engine: &PersistenceEngine,
    id: &str,
    next: i64,
    reason: &str,
) -> Result<(), String> {
    engine.open_connection().map_err(|error|error.to_string())?.execute("UPDATE workflow_schedules SET claimed_at_ms=NULL,next_run_at_ms=?2,last_status='Pending',last_error=?3,updated_at_ms=?4 WHERE id=?1",rusqlite::params![id,next,reason,unix_time_ms()]).map_err(|error|error.to_string())?;
    Ok(())
}
fn record_result_policy(
    engine: &PersistenceEngine,
    id: &str,
    failed: bool,
    terminal_one_shot: bool,
) -> Result<(), String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    if failed{connection.execute("UPDATE workflow_schedules SET consecutive_failures=consecutive_failures+1,is_active=CASE WHEN consecutive_failures+1>=failure_threshold THEN 0 ELSE is_active END,paused_reason=CASE WHEN consecutive_failures+1>=failure_threshold THEN 'Paused after repeated failures' ELSE paused_reason END,updated_at_ms=?2 WHERE id=?1",rusqlite::params![id,unix_time_ms()])}else{connection.execute("UPDATE workflow_schedules SET consecutive_failures=0,is_active=CASE WHEN ?2 THEN 0 ELSE is_active END,paused_reason=CASE WHEN ?2 THEN 'One-time routine completed' ELSE NULL END,updated_at_ms=?3 WHERE id=?1",rusqlite::params![id,terminal_one_shot,unix_time_ms()])}.map_err(|error|error.to_string())?;
    Ok(())
}

fn register_workflow_task(engine: &PersistenceEngine, instance_id: &str) -> Result<String, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    if let Some(existing)=connection.query_row("SELECT task_run_id FROM task_runs WHERE runtime_kind='workflow' AND runtime_record_id=?1",rusqlite::params![instance_id],|row|row.get::<_,String>(0)).optional().map_err(|error|error.to_string())?{return Ok(existing)}
    let (project,status,workflow,error,created,updated):(Option<String>,String,String,Option<String>,i64,i64)=connection.query_row("SELECT project_id,status,workflow_id,error_json,created_at_ms,updated_at_ms FROM execution_instances WHERE id=?1",rusqlite::params![instance_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).map_err(|error|error.to_string())?;
    let state = match status.to_ascii_lowercase().as_str() {
        "pending" => "queued",
        "running" => "running",
        "awaitingapproval" | "awaiting_approval" => "awaiting_approval",
        "paused" => "blocked",
        "completed" => "completed",
        "cancelled" => "cancelled",
        _ => "failed",
    };
    let task_id = crate::p0_contracts::TaskId::new().to_string();
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    connection.execute("INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,last_error,created_at_ms,updated_at_ms,completed_at_ms,recovery_state) VALUES (?1,?2,?3,'workflow',?4,?5,'routine',?2,?6,?7,?8,?9,CASE WHEN ?5 IN ('completed','failed','cancelled') THEN ?9 ELSE NULL END,'reconciled')",rusqlite::params![task_run_id,task_id,project,instance_id,state,format!("Workflow run {workflow}"),error,created,updated]).map_err(|error|error.to_string())?;
    Ok(task_run_id)
}

fn workflow_node_label(node: &WorkflowNode) -> &str {
    match node {
        WorkflowNode::Input(node) => &node.label,
        WorkflowNode::Agent(node) => &node.label,
        WorkflowNode::Router(node) => &node.label,
        WorkflowNode::Conditional(node) => &node.label,
        WorkflowNode::Loop(node) => &node.label,
        WorkflowNode::Permission(node) => &node.label,
        WorkflowNode::McpTool(node) => &node.label,
        WorkflowNode::SystemAction(node) => &node.label,
        WorkflowNode::Output(node) => &node.label,
    }
}

fn declined_actions_for_instance(
    persistence: &PersistenceEngine,
    instance: &ExecutionInstance,
) -> Result<Vec<String>, String> {
    let compiled = persistence
        .load_compiled_workflow(&instance.workflow_id, Some(instance.workflow_version))
        .map_err(|error| error.to_string())?;
    let mut declined = Vec::new();
    for node in &compiled.workflow_ir.nodes {
        let WorkflowNode::Permission(permission) = node else {
            continue;
        };
        let was_declined = instance
            .node_payloads
            .get(&permission.id)
            .and_then(|payload| payload.output.as_ref())
            .and_then(|output| output.pointer("/data/decision"))
            .and_then(Value::as_str)
            == Some("reject");
        if !was_declined {
            continue;
        }
        let action = compiled
            .workflow_ir
            .edges
            .iter()
            .find(|edge| edge.source_node_id == permission.id && edge.source_port == "approved")
            .and_then(|edge| {
                compiled
                    .workflow_ir
                    .nodes
                    .iter()
                    .find(|candidate| candidate.id() == edge.target_node_id)
            })
            .map(workflow_node_label)
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| permission.label.trim())
            .to_string();
        if !action.is_empty() {
            declined.push(action);
        }
    }
    declined.sort();
    declined.dedup();
    Ok(declined)
}

fn record_declined_action_outcome(
    persistence: &PersistenceEngine,
    instance: &ExecutionInstance,
    task_run_id: &str,
    declined_actions: &[String],
    copy: &SchedulerCopy,
    schedule_title: &str,
) -> Result<(), String> {
    if declined_actions.is_empty() {
        return Ok(());
    }
    let actions = declined_actions.join(", ");
    let summary = render_scheduler_copy(
        &copy.completed_declined_body,
        &[("name", schedule_title), ("actions", &actions)],
    );
    let connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE task_runs SET summary=?2,updated_at_ms=?3 WHERE task_run_id=?1",
            rusqlite::params![task_run_id, summary, unix_time_ms()],
        )
        .map_err(|error| error.to_string())?;
    let recorded = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM task_events WHERE task_run_id=?1 AND json_extract(event_json,'$.eventType')='workflow.completed_with_declined_actions')",
            rusqlite::params![task_run_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| error.to_string())?;
    drop(connection);
    if !recorded {
        crate::tasks::record_domain_event(
            persistence,
            task_run_id,
            "workflow.completed_with_declined_actions",
            EvidenceClass::ObservedResult,
            json!({
                "outcome": "completed_with_declined_actions",
                "executionInstanceId": instance.id,
                "actions": declined_actions,
            }),
        )?;
    }
    Ok(())
}

fn completion_kind_for_instance(
    persistence: &PersistenceEngine,
    instance: &ExecutionInstance,
) -> Option<WorkflowCompletionKind> {
    let compiled = persistence
        .load_compiled_workflow(&instance.workflow_id, Some(instance.workflow_version))
        .ok()?;
    compiled.workflow_ir.nodes.iter().find_map(|node| {
        let WorkflowNode::Output(output) = node else {
            return None;
        };
        instance
            .node_payloads
            .get(&output.id)
            .is_some_and(|payload| payload.status == ExecutionStatus::Completed)
            .then_some(output.completion_kind)
    })
}

fn terminal_delivery_is_gated(schedule: &WorkflowScheduleRecord, status: ExecutionStatus) -> bool {
    schedule.id.starts_with("routine_")
        && schedule.schedule_kind == "one_shot"
        && status == ExecutionStatus::Completed
        && schedule
            .delivery_target
            .get("platform")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        && schedule
            .delivery_target
            .get("destination")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
}

fn apply_terminal_delivery_outcome(
    persistence: &PersistenceEngine,
    schedule: &WorkflowScheduleRecord,
    instance_id: &str,
    outcome: &RoutineDeliveryOutcome,
    copy: &SchedulerCopy,
) -> Result<(), String> {
    match outcome {
        RoutineDeliveryOutcome::NotRequired | RoutineDeliveryOutcome::Delivered => {
            record_result_policy(persistence, &schedule.id, false, true)?;
            persistence.reconcile_remote_workflow_task(instance_id)?;
            persistence
                .open_connection()
                .map_err(|error| error.to_string())?
                .execute(
                    "UPDATE workflow_schedules SET last_status='Completed',last_error=NULL,updated_at_ms=?2 WHERE id=?1",
                    rusqlite::params![schedule.id, unix_time_ms()],
                )
                .map_err(|error| error.to_string())?;
        }
        RoutineDeliveryOutcome::RetryableFailure { .. } => {
            let detail = render_scheduler_copy(
                &copy.delivery_retry_body,
                &[("name", &schedule_title(schedule))],
            );
            let now = unix_time_ms();
            let connection = persistence
                .open_connection()
                .map_err(|error| error.to_string())?;
            connection
                .execute(
                    "UPDATE workflow_schedules SET is_active=1,next_run_at_ms=NULL,claimed_at_ms=NULL,paused_reason='Routine delivery retry pending',last_status='Pending',last_error=?2,updated_at_ms=?3 WHERE id=?1",
                    rusqlite::params![schedule.id, detail, now],
                )
                .map_err(|error| error.to_string())?;
            connection
                .execute(
                    "UPDATE task_runs SET state='blocked',last_error=?2,completed_at_ms=NULL,recovery_state='recoverable',updated_at_ms=?3 WHERE runtime_kind='workflow' AND runtime_record_id=?1",
                    rusqlite::params![instance_id, detail, now],
                )
                .map_err(|error| error.to_string())?;
        }
        RoutineDeliveryOutcome::NeedsReview { .. } => {
            let detail = render_scheduler_copy(
                &copy.delivery_review_body,
                &[("name", &schedule_title(schedule))],
            );
            let now = unix_time_ms();
            let connection = persistence
                .open_connection()
                .map_err(|error| error.to_string())?;
            connection
                .execute(
                    "UPDATE workflow_schedules SET is_active=0,next_run_at_ms=NULL,claimed_at_ms=NULL,paused_reason='Routine delivery needs review',last_status='AwaitingApproval',last_error=?2,updated_at_ms=?3 WHERE id=?1",
                    rusqlite::params![schedule.id, detail, now],
                )
                .map_err(|error| error.to_string())?;
            connection
                .execute(
                    "UPDATE task_runs SET state='blocked',last_error=?2,completed_at_ms=NULL,recovery_state='recoverable',updated_at_ms=?3 WHERE runtime_kind='workflow' AND runtime_record_id=?1",
                    rusqlite::params![instance_id, detail, now],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn create_remote_approval(
    engine: &PersistenceEngine,
    schedule: &WorkflowScheduleRecord,
    approval: &crate::workflow_runtime::ApprovalRequest,
) -> Result<Option<String>, String> {
    let Some(platform) = schedule
        .delivery_target
        .get("platform")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let owner: Option<String> = connection
        .query_row(
            "SELECT owner_id FROM channel_configs WHERE platform=?1 AND is_active=1",
            rusqlite::params![platform],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .flatten();
    let Some(owner) = owner.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let mut bytes = [0_u8; 12];
    rand_core::OsRng.fill_bytes(&mut bytes);
    let code = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes);
    let code_hash = crate::foundation::digest::sha256_hex(code.as_bytes());
    let now = unix_time_ms();
    let secret = json!({"instanceId":approval.instance_id,"approvalToken":approval.approval_token});
    crate::secret_store::set_routine_approval(&code_hash, &secret.to_string())?;
    let task_run_id:Option<String>=connection.query_row("SELECT task_run_id FROM task_runs WHERE runtime_kind='workflow' AND runtime_record_id=?1",rusqlite::params![approval.instance_id],|row|row.get(0)).optional().map_err(|error|error.to_string())?;
    connection.execute("INSERT INTO routine_remote_approvals (decision_code_hash,schedule_id,execution_instance_id,task_run_id,node_id,action_name,arguments_hash,channel_platform,channel_owner_hash,expires_at_ms,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",rusqlite::params![code_hash,schedule.id,approval.instance_id,task_run_id,approval.node_id,approval.message,crate::db::hash_arguments(&approval.context),platform,crate::foundation::digest::sha256_hex(owner.trim().as_bytes()),now+15*60*1_000,now]).map_err(|error|error.to_string())?;
    Ok(Some(code))
}

fn run_claimed_schedule(
    app: &tauri::AppHandle,
    persistence: &PersistenceEngine,
    gemma: GemmaService,
    knowledge: KnowledgeStore,
    mcp_registry: McpClientRegistry,
    gateway: SovereignGatewayService,
    schedule: WorkflowScheduleRecord,
    next_run_at_ms: Option<i64>,
) -> Result<(), String> {
    // - `schedule.next_run_at_ms` identifies the claimed logical occurrence.
    // - `next_run_at_ms` is its already-reviewed successor, never wall-clock drift.
    // - Run Now controls are consumed with the terminal result transaction.
    // - transient retries retain the same execution instance and logical occurrence.
    // - end-boundary cleanup preserves the terminal status written for this run.
    // - delivery state remains independently gated by its receipt contract.
    let ends_after_this_run = schedule.schedule_kind != "one_shot"
        && next_run_at_ms.is_none()
        && scheduler_try!(crate::routines::control::end_at_ms(&schedule.run_request)).is_some();
    let retry_instance = scheduler_try!(retryable_instance_for_claim(persistence, &schedule));
    let workspace_root = settings::app_data_root().join("workflow-runs");
    let run_result = if let Some(instance_id) = retry_instance {
        workflow_runtime::retry_scheduled_workflow(
            &schedule,
            &instance_id,
            persistence,
            gemma,
            mcp_registry,
            app.clone(),
            &workspace_root,
        )
    } else {
        workflow_runtime::run_scheduled_workflow(
            &schedule,
            persistence,
            gemma,
            knowledge,
            mcp_registry,
            app.clone(),
            &workspace_root,
        )
    };
    let copy = SchedulerCopy::load(persistence);
    match run_result {
        Ok(response) => {
            if scheduler_try!(requeue_transient_failure(persistence, &schedule, &response)) {
                return Ok(());
            }
            let status = response.instance.status;
            let completion_kind = response
                .completion
                .as_ref()
                .map(|completion| completion.kind);
            let instance_id = response.instance.id.clone();
            let error_message = response
                .instance
                .error
                .as_ref()
                .and_then(compact_error_message);
            let (terminal_delivery_gated, task_run_id) =
                scheduler_try!(project_claimed_schedule_result(
                    persistence,
                    &schedule,
                    &response,
                    next_run_at_ms,
                    ends_after_this_run,
                ));
            scheduler_try!(persist_claimed_schedule_result(
                persistence,
                &schedule,
                status,
                Some(&instance_id),
                error_message.as_deref(),
                next_run_at_ms,
            )
            .map_err(|error| {
                format!(
                    "Unable to persist result for schedule {} instance {}: {error}",
                    schedule.id, instance_id
                )
            }));
            if schedule.id.starts_with("routine_") && !terminal_delivery_gated {
                scheduler_try!(record_result_policy(
                    persistence,
                    &schedule.id,
                    status == ExecutionStatus::Failed,
                    schedule.schedule_kind == "one_shot" && status == ExecutionStatus::Completed,
                ));
                if ends_after_this_run {
                    finish_routine_at_end_boundary(persistence, &schedule.id)?;
                }
            }
            workflow_runtime::dispatch_approval_request(app, scheduled_approval(&response));
            let declined_actions = if status == ExecutionStatus::Completed {
                declined_actions_for_instance(persistence, &response.instance)?
            } else {
                Vec::new()
            };
            record_declined_action_outcome(
                persistence,
                &response.instance,
                &task_run_id,
                &declined_actions,
                &copy,
                &schedule_title(&schedule),
            )?;
            let updated = persistence
                .update_workflow_last_run(
                    &schedule.workflow_id,
                    response.instance.started_at_ms.unwrap_or_else(unix_time_ms),
                )
                .map_err(|error| {
                    format!(
                        "Unable to persist last-run time for workflow {}: {error}",
                        schedule.workflow_id
                    )
                })?;
            if !updated {
                return Err(format!(
                    "Workflow {} disappeared before its last-run state could be persisted.",
                    schedule.workflow_id
                ));
            }
            let approval_code = response.approval_request.as_ref().and_then(|approval| {
                create_remote_approval(persistence, &schedule, approval)
                    .ok()
                    .flatten()
            });
            let delivery_outcome = deliver_routine_notice(
                persistence,
                &gateway,
                &schedule,
                Some(&instance_id),
                status,
                completion_kind,
                error_message.as_deref(),
                approval_code.as_deref(),
                &copy,
                &declined_actions,
            );
            if terminal_delivery_gated {
                apply_terminal_delivery_outcome(
                    persistence,
                    &schedule,
                    &instance_id,
                    &delivery_outcome,
                    &copy,
                )?;
            }
            match delivery_outcome {
                RoutineDeliveryOutcome::RetryableFailure { .. } if terminal_delivery_gated => {
                    notify_background_event(
                        app,
                        &copy.delivery_retry_title,
                        &render_scheduler_copy(
                            &copy.delivery_retry_body,
                            &[("name", &schedule_title(&schedule))],
                        ),
                    );
                }
                RoutineDeliveryOutcome::NeedsReview { .. } if terminal_delivery_gated => {
                    notify_background_event(
                        app,
                        &copy.delivery_review_title,
                        &render_scheduler_copy(
                            &copy.delivery_review_body,
                            &[("name", &schedule_title(&schedule))],
                        ),
                    );
                }
                _ => notify_for_run_status(
                    app,
                    &schedule,
                    status,
                    completion_kind,
                    error_message.as_deref(),
                    &copy,
                    &declined_actions,
                ),
            }
        }
        Err(error) => {
            let message = error.message;
            persist_claimed_schedule_result(
                persistence,
                &schedule,
                ExecutionStatus::Failed,
                None,
                Some(&message),
                next_run_at_ms,
            )
            .map_err(|write_error| {
                format!(
                    "Unable to persist failure for schedule {}: {write_error}",
                    schedule.id
                )
            })?;
            if schedule.id.starts_with("routine_") {
                record_result_policy(persistence, &schedule.id, true, false)?;
                if ends_after_this_run {
                    finish_routine_at_end_boundary(persistence, &schedule.id)?;
                }
            }
            notify_background_event(
                app,
                &copy.failed_title,
                &render_scheduler_copy(
                    &copy.run_failed_body,
                    &[("name", &schedule_title(&schedule)), ("error", &message)],
                ),
            );
            deliver_routine_notice(
                persistence,
                &gateway,
                &schedule,
                None,
                ExecutionStatus::Failed,
                None,
                Some(&message),
                None,
                &copy,
                &[],
            );
        }
    }
    Ok(())
}

fn verify_scheduled_postcondition(
    persistence: &PersistenceEngine,
    schedule: &WorkflowScheduleRecord,
    task_run_id: &str,
    status: ExecutionStatus,
    next_run_at_ms: Option<i64>,
    instance_id: Option<&str>,
) -> Result<(), String> {
    if status != ExecutionStatus::Completed
        || !crate::routines::background::scheduled_file_postcondition_required(
            persistence,
            task_run_id,
        )?
    {
        return Ok(());
    }
    if crate::routines::background::record_verified_schedule_completion(
        persistence,
        &schedule.id,
        task_run_id,
    )? {
        return Ok(());
    }

    const CODE: &str = "scheduled_postcondition_not_verified";
    persist_claimed_schedule_result(
        persistence,
        schedule,
        ExecutionStatus::Failed,
        instance_id,
        Some(CODE),
        next_run_at_ms,
    )?;
    persistence
        .open_connection()
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE task_runs SET state='blocked',last_error=?2,completed_at_ms=NULL,recovery_state='recoverable',updated_at_ms=?3 WHERE task_run_id=?1",
            rusqlite::params![task_run_id, CODE, unix_time_ms()],
        )
        .map_err(|error| error.to_string())?;
    Err(CODE.to_string())
}

fn project_schedule_result(
    persistence: &PersistenceEngine,
    schedule: &WorkflowScheduleRecord,
    response: &workflow_runtime::RunWorkflowResponse,
    next_run_at_ms: Option<i64>,
) -> Result<(bool, String), String> {
    let instance_id = response.instance.id.as_str();
    let status = response.instance.status;
    let terminal_delivery_gated = terminal_delivery_is_gated(schedule, status);
    let task_run_id = project_workflow_task(persistence, instance_id, false)?;
    verify_scheduled_postcondition(
        persistence,
        schedule,
        &task_run_id,
        status,
        next_run_at_ms,
        Some(instance_id),
    )?;
    if !terminal_delivery_gated {
        persistence.reconcile_remote_workflow_task(instance_id)?;
    }
    Ok((terminal_delivery_gated, task_run_id))
}

fn project_claimed_schedule_result(
    persistence: &PersistenceEngine,
    schedule: &WorkflowScheduleRecord,
    response: &workflow_runtime::RunWorkflowResponse,
    next_run_at_ms: Option<i64>,
    ends_after_this_run: bool,
) -> Result<(bool, String), String> {
    match project_schedule_result(persistence, schedule, response, next_run_at_ms) {
        Ok(projected) => Ok(projected),
        Err(error) => {
            persist_claimed_schedule_result(
                persistence,
                schedule,
                ExecutionStatus::Failed,
                Some(&response.instance.id),
                Some(&error),
                next_run_at_ms,
            )?;
            if schedule.id.starts_with("routine_") {
                record_result_policy(persistence, &schedule.id, true, false)?;
            }
            if ends_after_this_run {
                finish_routine_at_end_boundary(persistence, &schedule.id)?;
            }
            Err(error)
        }
    }
}

fn reconcile_routine_records_after_permission(
    persistence: &PersistenceEngine,
    response: &workflow_runtime::RunWorkflowResponse,
) -> Result<Option<WorkflowScheduleRecord>, String> {
    let connection = persistence
        .open_connection()
        .map_err(|error| error.to_string())?;
    let schedule_id = connection
        .query_row(
            "SELECT schedule_id FROM routine_runs WHERE execution_instance_id=?1",
            rusqlite::params![response.instance.id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    drop(connection);
    let Some(schedule_id) = schedule_id else {
        return Ok(None);
    };
    let schedule = persistence
        .load_workflow_schedule(&schedule_id)
        .map_err(|error| error.to_string())?;
    let status = response.instance.status;
    let error_message = response
        .instance
        .error
        .as_ref()
        .and_then(compact_error_message);
    let (terminal_delivery_gated, _task_run_id) =
        project_schedule_result(persistence, &schedule, response, schedule.next_run_at_ms)?;
    persist_claimed_schedule_result(
        persistence,
        &schedule,
        status,
        Some(&response.instance.id),
        error_message.as_deref(),
        schedule.next_run_at_ms,
    )?;
    if !terminal_delivery_gated {
        record_result_policy(
            persistence,
            &schedule.id,
            status == ExecutionStatus::Failed,
            schedule.schedule_kind == "one_shot" && status == ExecutionStatus::Completed,
        )?;
        if schedule.schedule_kind != "one_shot"
            && schedule.next_run_at_ms.is_none()
            && crate::routines::control::end_at_ms(&schedule.run_request)?.is_some()
        {
            finish_routine_at_end_boundary(persistence, &schedule.id)?;
        }
    }
    persistence
        .load_workflow_schedule(&schedule.id)
        .map(Some)
        .map_err(|error| error.to_string())
}

pub(crate) fn reconcile_routine_after_permission(
    app: &tauri::AppHandle,
    persistence: &PersistenceEngine,
    gateway: &SovereignGatewayService,
    response: &workflow_runtime::RunWorkflowResponse,
) -> Result<(), String> {
    let Some(schedule) = reconcile_routine_records_after_permission(persistence, response)? else {
        return Ok(());
    };
    let status = response.instance.status;
    let completion_kind = response
        .completion
        .as_ref()
        .map(|completion| completion.kind);
    let error_message = response
        .instance
        .error
        .as_ref()
        .and_then(compact_error_message);
    let approval_code = response.approval_request.as_ref().and_then(|approval| {
        create_remote_approval(persistence, &schedule, approval)
            .ok()
            .flatten()
    });
    let copy = SchedulerCopy::load(persistence);
    let task_run_id = register_workflow_task(persistence, &response.instance.id)?;
    let declined_actions = if status == ExecutionStatus::Completed {
        declined_actions_for_instance(persistence, &response.instance)?
    } else {
        Vec::new()
    };
    record_declined_action_outcome(
        persistence,
        &response.instance,
        &task_run_id,
        &declined_actions,
        &copy,
        &schedule_title(&schedule),
    )?;
    let delivery_outcome = deliver_routine_notice(
        persistence,
        gateway,
        &schedule,
        Some(&response.instance.id),
        status,
        completion_kind,
        error_message.as_deref(),
        approval_code.as_deref(),
        &copy,
        &declined_actions,
    );
    let terminal_delivery_gated = terminal_delivery_is_gated(&schedule, status);
    if terminal_delivery_gated {
        apply_terminal_delivery_outcome(
            persistence,
            &schedule,
            &response.instance.id,
            &delivery_outcome,
            &copy,
        )?;
    }
    match delivery_outcome {
        RoutineDeliveryOutcome::RetryableFailure { .. } if terminal_delivery_gated => {
            notify_background_event(
                app,
                &copy.delivery_retry_title,
                &render_scheduler_copy(
                    &copy.delivery_retry_body,
                    &[("name", &schedule_title(&schedule))],
                ),
            );
        }
        RoutineDeliveryOutcome::NeedsReview { .. } if terminal_delivery_gated => {
            notify_background_event(
                app,
                &copy.delivery_review_title,
                &render_scheduler_copy(
                    &copy.delivery_review_body,
                    &[("name", &schedule_title(&schedule))],
                ),
            );
        }
        _ => notify_for_run_status(
            app,
            &schedule,
            status,
            completion_kind,
            error_message.as_deref(),
            &copy,
            &declined_actions,
        ),
    }
    Ok(())
}

fn schedule_expression_from_visual_state(visual_state: &Value) -> Option<String> {
    visual_state
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|node| {
            let kind = node.get("kind").and_then(Value::as_str)?;
            if kind != "schedule" {
                return None;
            }
            let expression = node.get("schedule").and_then(Value::as_str)?.trim();
            if expression.is_empty() || is_manual_expression(&expression.to_ascii_lowercase()) {
                None
            } else {
                Some(expression.to_string())
            }
        })
}

fn default_schedule_id(workflow_id: &str) -> String {
    format!("workflow:{workflow_id}:default")
}

fn schedule_title(schedule: &WorkflowScheduleRecord) -> String {
    if schedule.label.trim().is_empty() {
        schedule.workflow_id.clone()
    } else {
        schedule.label.trim().to_string()
    }
}

fn compact_error_message(value: &Value) -> Option<String> {
    value
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(|message| message.chars().take(240).collect())
}

#[cfg(test)]
mod tests;
