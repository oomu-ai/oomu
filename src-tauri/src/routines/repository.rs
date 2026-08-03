use super::{CreateRoutineRequest, RoutineEndBoundary, RoutineRecord};
use crate::{db::PersistenceEngine, foundation::clock::unix_time_ms_i64, p0_contracts::ProjectId};
use rusqlite::{params, OptionalExtension, Row};
use serde_json::Value;

fn validate(request: &CreateRoutineRequest) -> Result<(), String> {
    if !request.confirmed {
        return Err("Confirm the normalized schedule before activation.".to_string());
    }
    ProjectId::parse(&request.project_id)?;
    if request.label.trim().is_empty() || request.label.len() > 120 {
        return Err("Routine name is required and must be 120 characters or fewer.".to_string());
    }
    if !matches!(request.schedule_kind.as_str(), "one_shot" | "recurring") {
        return Err("Routine schedule kind is invalid.".to_string());
    }
    if !matches!(
        request.missed_run_policy.as_str(),
        "skip" | "run_once" | "run_each"
    ) {
        return Err("Routine missed-run policy is invalid.".to_string());
    }
    if !(1..=12).contains(&request.missed_run_cap) {
        return Err("Routine missed-run cap must be between 1 and 12.".to_string());
    }
    let _: chrono_tz::Tz = request
        .timezone
        .parse()
        .map_err(|_| "Routine timezone is invalid.".to_string())?;
    if request.active_window_start_minute.is_some() != request.active_window_end_minute.is_some() {
        return Err("An active window requires both a start and end.".to_string());
    }
    if !request.task_template.is_object() {
        return Err("Routine task template must be a JSON object.".to_string());
    }
    if request.end_boundary.is_some() && request.schedule_kind != "recurring" {
        return Err("A Routine end boundary requires a recurring schedule.".to_string());
    }
    Ok(())
}

pub(super) fn create(
    engine: &PersistenceEngine,
    request: CreateRoutineRequest,
) -> Result<RoutineRecord, String> {
    validate(&request)?;
    engine.require_durable_store("create a routine")?;
    let id = format!("routine_{}", crate::p0_contracts::TaskId::new());
    let authority = super::authority::derive_for_create(engine, &id, &request)?;
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let now = unix_time_ms_i64();
    let scheduled_next = crate::schedule_expression::next_run_after_in_timezone(
        &request.schedule_expression,
        &request.timezone,
        now,
    )?;
    let end_at_ms = match request.end_boundary {
        Some(RoutineEndBoundary::Midnight) => {
            Some(super::control::next_midnight_ms(&request.timezone, now)?)
        }
        None => None,
    };
    let run_request = match end_at_ms {
        Some(end_at_ms) => super::control::with_end_at_ms(&request.task_template, end_at_ms)?,
        None => request.task_template.clone(),
    };
    let next = if request.run_once_after_create {
        now
    } else if end_at_ms.is_some_and(|end| scheduled_next >= end) {
        return Err("The recurring schedule has no run before its end boundary.".to_string());
    } else {
        scheduled_next
    };
    connection.execute("INSERT INTO workflow_schedules (id,workflow_id,workflow_version,label,schedule_expression,run_request_json,is_active,next_run_at_ms,created_at_ms,updated_at_ms,encryption_state,project_id,routine_timezone,schedule_kind,active_window_start_minute,active_window_end_minute,missed_run_policy,missed_run_cap,model_route_json,delivery_target_json,authority_json) VALUES (?1,?2,?3,?4,?5,?6,1,?7,?8,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",params![id,request.workflow_id,request.workflow_version,request.label.trim(),request.schedule_expression,run_request.to_string(),next,now,crate::db::get_current_encryption_state(),request.project_id,request.timezone,request.schedule_kind,request.active_window_start_minute,request.active_window_end_minute,request.missed_run_policy,request.missed_run_cap,request.model_route.to_string(),request.delivery_target.to_string(),authority.to_string()]).map_err(|e|e.to_string())?;
    get(engine, &id)
}

const SELECT:&str="SELECT id,label,project_id,workflow_id,workflow_version,schedule_expression,schedule_kind,routine_timezone,is_active,next_run_at_ms,missed_run_policy,consecutive_failures,failure_threshold,paused_reason,delivery_target_json,last_status,last_error,(SELECT CASE WHEN d.state='delivered' THEN 'delivered' WHEN d.state IN ('pending','failed') AND e.state='reserved' THEN 'retrying' WHEN d.state IN ('pending','failed') AND e.state='executed' THEN 'needs_review' ELSE NULL END FROM routine_delivery_receipts d LEFT JOIN task_effects e ON e.task_run_id=d.task_run_id AND e.effect_kind='routine_channel_delivery' AND e.idempotency_key LIKE 'routine-delivery:' || workflow_schedules.id || ':completed%:%' WHERE d.schedule_id=workflow_schedules.id AND d.event_kind='completed' ORDER BY d.created_at_ms DESC LIMIT 1) AS delivery_state,(SELECT d.error_code FROM routine_delivery_receipts d WHERE d.schedule_id=workflow_schedules.id AND d.event_kind='completed' ORDER BY d.created_at_ms DESC LIMIT 1) AS delivery_error_code FROM workflow_schedules WHERE id LIKE 'routine_%'";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RoutineDeletionReceipt {
    pub pending_remote_approval_code_hashes: Vec<String>,
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<RoutineRecord> {
    let expression: String = row.get(5)?;
    let timezone: String = row.get(7)?;
    let next_run: Option<i64> = row.get(9)?;
    let mut next_runs = Vec::new();
    if let Some(mut cursor) = next_run {
        for index in 0..5 {
            next_runs.push(cursor);
            if index == 4 || row.get::<_, String>(6)? == "one_shot" {
                break;
            }
            match crate::schedule_expression::next_run_after_in_timezone(
                &expression,
                &timezone,
                cursor,
            ) {
                Ok(value) => cursor = value,
                Err(_) => break,
            }
        }
    }
    let delivery: String = row.get(14)?;
    Ok(RoutineRecord {
        routine_id: row.get(0)?,
        label: row.get(1)?,
        project_id: row.get(2)?,
        workflow_id: row.get(3)?,
        workflow_version: row.get(4)?,
        schedule_expression: expression,
        schedule_kind: row.get(6)?,
        timezone,
        is_active: row.get::<_, i64>(8)? != 0,
        next_run_at_ms: next_run,
        next_runs_ms: next_runs,
        missed_run_policy: row.get(10)?,
        consecutive_failures: row.get::<_, i64>(11)? as u32,
        failure_threshold: row.get::<_, i64>(12)? as u32,
        paused_reason: row.get(13)?,
        delivery_target: serde_json::from_str(&delivery).unwrap_or(Value::Null),
        last_status: row.get(15)?,
        last_error: row.get(16)?,
        delivery_state: row.get(17)?,
        delivery_error_code: row.get(18)?,
    })
}

pub(super) fn list(engine: &PersistenceEngine) -> Result<Vec<RoutineRecord>, String> {
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let mut statement = connection
        .prepare(&format!("{SELECT} ORDER BY updated_at_ms DESC"))
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], from_row)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub(crate) fn get(engine: &PersistenceEngine, id: &str) -> Result<RoutineRecord, String> {
    if !id.starts_with("routine_task_") {
        return Err("Invalid routine identifier.".to_string());
    }
    engine
        .open_connection()
        .map_err(|e| e.to_string())?
        .query_row(&format!("{SELECT} AND id=?1"), params![id], from_row)
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Routine was not found.".to_string())
}

pub(super) fn set_active(
    engine: &PersistenceEngine,
    id: &str,
    active: bool,
) -> Result<RoutineRecord, String> {
    let current = get(engine, id)?;
    if matches!(
        current.delivery_state.as_deref(),
        Some("retrying" | "needs_review")
    ) {
        return Err("routine_delivery_in_progress".to_string());
    }
    let next = if active {
        Some(crate::schedule_expression::next_run_after_in_timezone(
            &current.schedule_expression,
            &current.timezone,
            unix_time_ms_i64(),
        )?)
    } else {
        None
    };
    engine.open_connection().map_err(|e|e.to_string())?.execute("UPDATE workflow_schedules SET is_active=?2,next_run_at_ms=?3,claimed_at_ms=NULL,paused_reason=CASE WHEN ?2=1 THEN NULL ELSE 'Paused by user' END,updated_at_ms=?4 WHERE id=?1",params![id,active,next,unix_time_ms_i64()]).map_err(|e|e.to_string())?;
    get(engine, id)
}

pub(crate) fn delete_with_receipt(
    engine: &PersistenceEngine,
    id: &str,
) -> Result<RoutineDeletionReceipt, String> {
    if !id.starts_with("routine_task_") {
        return Err("Invalid routine identifier.".to_string());
    }
    engine.require_durable_store("delete a routine")?;
    let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    let state: Option<(Option<i64>, Option<String>)> = transaction
        .query_row(
            "SELECT claimed_at_ms,last_status FROM workflow_schedules WHERE id=?1 AND id LIKE 'routine_%'",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((claimed_at_ms, last_status)) = state else {
        return Err("Routine was not found.".to_string());
    };
    let linked_run_is_running: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM routine_runs r LEFT JOIN task_runs t ON t.task_run_id=r.task_run_id LEFT JOIN execution_instances e ON e.id=r.execution_instance_id WHERE r.schedule_id=?1 AND (t.state IN ('queued','planning','awaiting_approval','running','blocked') OR e.status IN ('Pending','Running','AwaitingApproval')))",
            params![id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if claimed_at_ms.is_some() || last_status.as_deref() == Some("Running") || linked_run_is_running
    {
        return Err("routine_delete_in_progress".to_string());
    }

    let pending_remote_approval_code_hashes = {
        let mut statement = transaction
            .prepare(
                "SELECT decision_code_hash FROM routine_remote_approvals WHERE schedule_id=?1 AND decided_at_ms IS NULL ORDER BY created_at_ms,decision_code_hash",
            )
            .map_err(|error| error.to_string())?;
        let hashes = statement
            .query_map(params![id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        hashes
    };

    for statement in [
        "DELETE FROM routine_remote_approvals WHERE schedule_id=?1",
        "DELETE FROM routine_delivery_receipts WHERE schedule_id=?1",
        "DELETE FROM routine_runs WHERE schedule_id=?1",
        "DELETE FROM routine_authority_grants WHERE schedule_id=?1",
    ] {
        transaction
            .execute(statement, params![id])
            .map_err(|error| error.to_string())?;
    }
    let deleted = transaction
        .execute("DELETE FROM workflow_schedules WHERE id=?1", params![id])
        .map_err(|error| error.to_string())?;
    if deleted != 1 {
        return Err("Routine deletion lost its schedule boundary.".to_string());
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(RoutineDeletionReceipt {
        pending_remote_approval_code_hashes,
    })
}

pub(super) fn delete(engine: &PersistenceEngine, id: &str) -> Result<(), String> {
    let receipt = delete_with_receipt(engine, id)?;
    let cleanup_count = receipt.pending_remote_approval_code_hashes.len();
    let mut cleanup_failures = 0usize;
    for code_hash in receipt.pending_remote_approval_code_hashes {
        if crate::secret_store::delete_routine_approval(&code_hash).is_err() {
            cleanup_failures += 1;
        }
    }
    if cleanup_count > 0 {
        eprintln!(
            "ROUTINE_REMOTE_APPROVALS_REVOKED cleanup_count={cleanup_count} cleanup_failures={cleanup_failures}"
        );
    }
    Ok(())
}

pub(super) fn duplicate(engine: &PersistenceEngine, id: &str) -> Result<RoutineRecord, String> {
    let source = get(engine, id)?;
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let new_id = format!("routine_{}", crate::p0_contracts::TaskId::new());
    let now = unix_time_ms_i64();
    let authority = super::authority::rebind_for_duplicate(engine, &source.routine_id, &new_id)?;
    connection.execute("INSERT INTO workflow_schedules (id,workflow_id,workflow_version,label,schedule_expression,run_request_json,is_active,next_run_at_ms,created_at_ms,updated_at_ms,encryption_state,project_id,routine_timezone,schedule_kind,active_window_start_minute,active_window_end_minute,missed_run_policy,missed_run_cap,model_route_json,delivery_target_json,authority_json,failure_threshold) SELECT ?1,workflow_id,workflow_version,label||' Copy',schedule_expression,run_request_json,0,NULL,?2,?2,encryption_state,project_id,routine_timezone,schedule_kind,active_window_start_minute,active_window_end_minute,missed_run_policy,missed_run_cap,model_route_json,delivery_target_json,?3,failure_threshold FROM workflow_schedules WHERE id=?4",params![new_id,now,authority.to_string(),source.routine_id]).map_err(|e|e.to_string())?;
    get(engine, &new_id)
}

pub(super) fn update(
    engine: &PersistenceEngine,
    request: super::UpdateRoutineRequest,
) -> Result<RoutineRecord, String> {
    let current = get(engine, &request.routine_id)?;
    if request.label.trim().is_empty() || request.label.len() > 120 {
        return Err("Routine name is required and must be 120 characters or fewer.".to_string());
    }
    let _: chrono_tz::Tz = request
        .timezone
        .parse()
        .map_err(|_| "Routine timezone is invalid.".to_string())?;
    if !matches!(
        request.missed_run_policy.as_str(),
        "skip" | "run_once" | "run_each"
    ) {
        return Err("Routine missed-run policy is invalid.".to_string());
    }
    if !(1..=12).contains(&request.missed_run_cap) {
        return Err("Routine missed-run cap must be between 1 and 12.".to_string());
    }
    let authority: Value = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT authority_json FROM workflow_schedules WHERE id=?1",
            params![request.routine_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| error.to_string())
        .and_then(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))?;
    if !super::authority::delivery_matches_manifest(&authority, &request.delivery_target) {
        return Err("Routine delivery changes require a new authority review.".to_string());
    }
    let next = crate::schedule_expression::next_run_after_in_timezone(
        &request.schedule_expression,
        &request.timezone,
        unix_time_ms_i64(),
    )?;
    engine.open_connection().map_err(|e|e.to_string())?.execute("UPDATE workflow_schedules SET label=?2,schedule_expression=?3,routine_timezone=?4,missed_run_policy=?5,missed_run_cap=?6,delivery_target_json=?7,next_run_at_ms=CASE WHEN is_active=1 THEN ?8 ELSE NULL END,claimed_at_ms=NULL,updated_at_ms=?9 WHERE id=?1",params![request.routine_id,request.label.trim(),request.schedule_expression,request.timezone,request.missed_run_policy,request.missed_run_cap,request.delivery_target.to_string(),next,unix_time_ms_i64()]).map_err(|e|e.to_string())?;
    get(engine, &current.routine_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{CreateProjectRequest, ProjectDataPolicy};
    use std::{fs, path::PathBuf};

    struct RoutineFixture {
        root: PathBuf,
        engine: PersistenceEngine,
        project_id: String,
        workflow_id: String,
        routine_id: String,
    }

    fn fixture(label: &str) -> RoutineFixture {
        let root = std::env::temp_dir().join(format!(
            "oomu-routine-delete-{label}-{}",
            crate::p0_contracts::TaskId::new()
        ));
        fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let project = crate::projects::repository::create(
            &engine,
            CreateProjectRequest {
                name: format!("Routine {label}"),
                description: String::new(),
                data_policy: ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let workflow_id = format!("workflow-{label}-{}", crate::p0_contracts::TaskId::new());
        let routine_id = format!("routine_{}", crate::p0_contracts::TaskId::new());
        let connection = engine.open_connection().unwrap();
        connection.execute(
            "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,is_active,created_at_ms,updated_at_ms,encryption_state,project_id) VALUES (?1,1,?2,'','{}',1,1,1,'test',?3)",
            params![workflow_id, format!("Workflow {label}"), project.project_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO workflow_schedules(id,workflow_id,workflow_version,label,schedule_expression,run_request_json,is_active,next_run_at_ms,created_at_ms,updated_at_ms,encryption_state,project_id) VALUES (?1,?2,1,?3,'0 * * * *','{}',0,NULL,1,1,'test',?4)",
            params![routine_id, workflow_id, format!("Routine {label}"), project.project_id],
        ).unwrap();
        drop(connection);
        RoutineFixture {
            root,
            engine,
            project_id: project.project_id,
            workflow_id,
            routine_id,
        }
    }

    fn count(engine: &PersistenceEngine, table: &str, field: &str, value: &str) -> i64 {
        engine
            .open_connection()
            .unwrap()
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {field}=?1"),
                params![value],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn routine_record_projects_durable_terminal_delivery_state() {
        let fixture = fixture("delivery-state");
        let execution_id = "execution-routine-delivery-state";
        let task_run_id = "taskrun_55555555-5555-4555-8555-555555555555";
        let effect_key = format!(
            "routine-delivery:{}:completed:{}",
            fixture.routine_id, execution_id
        );
        let connection = fixture.engine.open_connection().unwrap();
        connection.execute(
            "INSERT INTO execution_instances(id,workflow_id,workflow_version,status,created_at_ms,updated_at_ms,encryption_state,project_id) VALUES (?1,?2,1,'Completed',1,1,'test',?3)",
            params![execution_id,fixture.workflow_id,fixture.project_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO task_runs(task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,'task_55555555-5555-4555-8555-555555555555',?2,'workflow',?3,'blocked','routine','routine-delivery-state','Delivery needs attention',1,1,'recoverable')",
            params![task_run_id,fixture.project_id,execution_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO routine_runs(schedule_id,execution_instance_id,task_run_id,scheduled_for_ms,created_at_ms) VALUES (?1,?2,?3,1,1)",
            params![fixture.routine_id,execution_id,task_run_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO task_effects(task_run_id,idempotency_key,effect_kind,state,updated_at_ms) VALUES (?1,?2,'routine_channel_delivery','executed',1)",
            params![task_run_id,effect_key],
        ).unwrap();
        connection.execute(
            "INSERT INTO routine_delivery_receipts(receipt_id,schedule_id,task_run_id,platform,destination_hash,event_kind,state,error_code,created_at_ms,updated_at_ms) VALUES ('receipt-delivery-state',?1,?2,'slack','destination','completed','failed','network_timeout',1,1)",
            params![fixture.routine_id,task_run_id],
        ).unwrap();
        drop(connection);

        let review = get(&fixture.engine, &fixture.routine_id).unwrap();
        assert_eq!(review.delivery_state.as_deref(), Some("needs_review"));
        assert_eq!(
            review.delivery_error_code.as_deref(),
            Some("network_timeout")
        );
        fixture.engine.open_connection().unwrap().execute(
            "UPDATE task_effects SET state='reserved' WHERE task_run_id=?1 AND idempotency_key=?2",
            params![task_run_id,effect_key],
        ).unwrap();
        assert_eq!(
            get(&fixture.engine, &fixture.routine_id)
                .unwrap()
                .delivery_state
                .as_deref(),
            Some("retrying")
        );
        let _ = fs::remove_dir_all(fixture.root);
    }

    #[test]
    fn delete_removes_only_schedule_owned_rows_and_returns_pending_approval_hashes() {
        let fixture = fixture("owned-rows");
        let execution_id = "execution-routine-delete";
        let task_run_id = "taskrun_33333333-3333-4333-8333-333333333333";
        let pending_hash = "a".repeat(64);
        let decided_hash = "b".repeat(64);
        let connection = fixture.engine.open_connection().unwrap();
        connection.execute(
            "INSERT INTO execution_instances(id,workflow_id,workflow_version,status,created_at_ms,updated_at_ms,encryption_state,project_id) VALUES (?1,?2,1,'Completed',1,1,'test',?3)",
            params![execution_id,fixture.workflow_id,fixture.project_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO task_runs(task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,'task_33333333-3333-4333-8333-333333333333',?2,'workflow',?3,'completed','routine','routine-delete','Completed result',1,1,'reconciled')",
            params![task_run_id,fixture.project_id,execution_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO task_events(task_run_id,sequence,event_json,created_at_ms) VALUES (?1,0,'{}',1)",
            params![task_run_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO routine_runs(schedule_id,execution_instance_id,task_run_id,scheduled_for_ms,created_at_ms) VALUES (?1,?2,?3,1,1)",
            params![fixture.routine_id,execution_id,task_run_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO routine_delivery_receipts(receipt_id,schedule_id,task_run_id,platform,destination_hash,event_kind,state,created_at_ms,updated_at_ms) VALUES ('receipt-delete',?1,?2,'slack','destination','completed','delivered',1,1)",
            params![fixture.routine_id,task_run_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO routine_authority_grants(grant_id,schedule_id,project_id,action_name,arguments_hash,expires_at_ms,created_at_ms) VALUES ('grant-delete',?1,?2,'send','arguments',999999,1)",
            params![fixture.routine_id,fixture.project_id],
        ).unwrap();
        for (hash, decided) in [(&pending_hash, false), (&decided_hash, true)] {
            connection.execute(
                "INSERT INTO routine_remote_approvals(decision_code_hash,schedule_id,execution_instance_id,task_run_id,node_id,action_name,arguments_hash,channel_platform,channel_owner_hash,expires_at_ms,decided_at_ms,decision,created_at_ms) VALUES (?1,?2,?3,?4,'node','approve','arguments','slack','owner',999999,?5,?6,1)",
                params![hash,fixture.routine_id,execution_id,task_run_id,decided.then_some(2_i64),decided.then_some("approve")],
            ).unwrap();
        }
        drop(connection);
        crate::secret_store::set_routine_approval(&pending_hash, r#"{"pending":true}"#).unwrap();

        delete(&fixture.engine, &fixture.routine_id).unwrap();
        assert_eq!(
            crate::secret_store::get_routine_approval(&pending_hash).unwrap(),
            None
        );
        assert_eq!(
            count(
                &fixture.engine,
                "workflow_schedules",
                "id",
                &fixture.routine_id
            ),
            0
        );
        for table in [
            "routine_remote_approvals",
            "routine_delivery_receipts",
            "routine_runs",
            "routine_authority_grants",
        ] {
            assert_eq!(
                count(&fixture.engine, table, "schedule_id", &fixture.routine_id),
                0,
                "{table} retained schedule-owned data"
            );
        }
        assert_eq!(
            count(
                &fixture.engine,
                "workflow_blueprints",
                "workflow_id",
                &fixture.workflow_id
            ),
            1
        );
        assert_eq!(
            count(&fixture.engine, "task_runs", "task_run_id", task_run_id),
            1
        );
        assert_eq!(
            count(&fixture.engine, "task_events", "task_run_id", task_run_id),
            1
        );
        assert_eq!(
            count(&fixture.engine, "execution_instances", "id", execution_id),
            1
        );
        let _ = fs::remove_dir_all(fixture.root);
    }

    #[test]
    fn delete_refuses_claimed_or_in_flight_routines_without_mutation() {
        for (label, update) in [
            ("claimed", "claimed_at_ms=10"),
            ("running", "last_status='Running'"),
        ] {
            let fixture = fixture(label);
            let connection = fixture.engine.open_connection().unwrap();
            connection
                .execute(
                    &format!("UPDATE workflow_schedules SET {update} WHERE id=?1"),
                    params![fixture.routine_id],
                )
                .unwrap();
            connection.execute(
                "INSERT INTO routine_authority_grants(grant_id,schedule_id,project_id,action_name,arguments_hash,expires_at_ms,created_at_ms) VALUES (?1,?2,?3,'send','arguments',999999,1)",
                params![format!("grant-{label}"),fixture.routine_id,fixture.project_id],
            ).unwrap();
            drop(connection);

            let error = delete_with_receipt(&fixture.engine, &fixture.routine_id).unwrap_err();
            assert_eq!(error, "routine_delete_in_progress");
            assert_eq!(
                count(
                    &fixture.engine,
                    "workflow_schedules",
                    "id",
                    &fixture.routine_id
                ),
                1
            );
            assert_eq!(
                count(
                    &fixture.engine,
                    "routine_authority_grants",
                    "schedule_id",
                    &fixture.routine_id
                ),
                1
            );
            let _ = fs::remove_dir_all(fixture.root);
        }

        let fixture = fixture("blocked");
        let execution_id = "execution-routine-blocked";
        let task_run_id = "taskrun_44444444-4444-4444-8444-444444444444";
        let connection = fixture.engine.open_connection().unwrap();
        connection.execute(
            "INSERT INTO execution_instances(id,workflow_id,workflow_version,status,created_at_ms,updated_at_ms,encryption_state,project_id) VALUES (?1,?2,1,'AwaitingApproval',1,1,'test',?3)",
            params![execution_id,fixture.workflow_id,fixture.project_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO task_runs(task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,'task_44444444-4444-4444-8444-444444444444',?2,'workflow',?3,'blocked','routine','routine-blocked','Awaiting approval',1,1,'recoverable')",
            params![task_run_id,fixture.project_id,execution_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO routine_runs(schedule_id,execution_instance_id,task_run_id,scheduled_for_ms,created_at_ms) VALUES (?1,?2,?3,1,1)",
            params![fixture.routine_id,execution_id,task_run_id],
        ).unwrap();
        drop(connection);

        let error = delete_with_receipt(&fixture.engine, &fixture.routine_id).unwrap_err();
        assert_eq!(error, "routine_delete_in_progress");
        assert_eq!(
            count(
                &fixture.engine,
                "workflow_schedules",
                "id",
                &fixture.routine_id
            ),
            1
        );
        assert_eq!(
            count(
                &fixture.engine,
                "routine_runs",
                "schedule_id",
                &fixture.routine_id
            ),
            1
        );
        let _ = fs::remove_dir_all(fixture.root);
    }
}
