use super::repository;
use crate::db::PersistenceEngine;
use rusqlite::params;
use serde_json::{json, Value};

struct RoutineHistoryRow {
    task_run_id: String,
    task_id: String,
    runtime_record_id: String,
    correlation_id: String,
    state: String,
    summary: String,
    last_error: Option<String>,
    task_created_at_ms: i64,
    task_updated_at_ms: i64,
    execution_instance_id: String,
    scheduled_for_ms: Option<i64>,
    run_created_at_ms: i64,
    schedule_created_at_ms: i64,
    schedule_updated_at_ms: i64,
    schedule_next_run_at_ms: Option<i64>,
    completion_event: Option<String>,
    effects_json: String,
    delivery_receipts_json: String,
    last_error_code: Option<String>,
}

fn generated_json_array(raw: &str, label: &str) -> Result<Value, String> {
    let value = serde_json::from_str::<Value>(raw)
        .map_err(|error| format!("Routine {label} verification data is invalid: {error}"))?;
    if !value.is_array() {
        return Err(format!(
            "Routine {label} verification data is not an array."
        ));
    }
    Ok(value)
}

pub(super) fn get(engine: &PersistenceEngine, id: &str) -> Result<Vec<Value>, String> {
    repository::get(engine, id)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare(
            r#"
            SELECT
                t.task_run_id,
                t.task_id,
                t.runtime_record_id,
                t.correlation_id,
                t.state,
                t.summary,
                t.last_error,
                t.created_at_ms,
                t.updated_at_ms,
                r.execution_instance_id,
                r.scheduled_for_ms,
                r.created_at_ms,
                s.created_at_ms,
                s.updated_at_ms,
                s.next_run_at_ms,
                (SELECT event.event_json
                   FROM task_events event
                  WHERE event.task_run_id=t.task_run_id
                    AND json_extract(event.event_json,'$.eventType')='workflow.completed_with_declined_actions'
                  ORDER BY event.sequence DESC
                  LIMIT 1),
                (SELECT json_group_array(json_object(
                    'idempotencyKey', effect.idempotency_key,
                    'effectKind', effect.effect_kind,
                    'state', effect.state,
                    'resultDigest', effect.result_digest,
                    'updatedAtMs', effect.updated_at_ms
                )) FROM (
                    SELECT idempotency_key,effect_kind,state,result_digest,updated_at_ms
                      FROM task_effects
                     WHERE task_run_id=t.task_run_id
                     ORDER BY effect_kind,idempotency_key
                ) effect),
                (SELECT json_group_array(json_object(
                    'receiptId', receipt.receipt_id,
                    'platform', receipt.platform,
                    'eventKind', receipt.event_kind,
                    'state', receipt.state,
                    'providerReceiptHash', receipt.provider_receipt_hash,
                    'errorCode', receipt.error_code,
                    'createdAtMs', receipt.created_at_ms,
                    'updatedAtMs', receipt.updated_at_ms
                )) FROM (
                    SELECT receipt_id,platform,event_kind,state,provider_receipt_hash,error_code,created_at_ms,updated_at_ms
                      FROM routine_delivery_receipts
                     WHERE task_run_id=t.task_run_id
                     ORDER BY created_at_ms,receipt_id
                ) receipt),
                json_extract(e.error_json,'$.code')
              FROM routine_runs r
              JOIN task_runs t ON t.task_run_id=r.task_run_id
              JOIN workflow_schedules s ON s.id=r.schedule_id
              LEFT JOIN execution_instances e ON e.id=r.execution_instance_id
             WHERE r.schedule_id=?1
             ORDER BY r.created_at_ms DESC
             LIMIT 100
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![id], |row| {
            Ok(RoutineHistoryRow {
                task_run_id: row.get(0)?,
                task_id: row.get(1)?,
                runtime_record_id: row.get(2)?,
                correlation_id: row.get(3)?,
                state: row.get(4)?,
                summary: row.get(5)?,
                last_error: row.get(6)?,
                task_created_at_ms: row.get(7)?,
                task_updated_at_ms: row.get(8)?,
                execution_instance_id: row.get(9)?,
                scheduled_for_ms: row.get(10)?,
                run_created_at_ms: row.get(11)?,
                schedule_created_at_ms: row.get(12)?,
                schedule_updated_at_ms: row.get(13)?,
                schedule_next_run_at_ms: row.get(14)?,
                completion_event: row.get(15)?,
                effects_json: row.get(16)?,
                delivery_receipts_json: row.get(17)?,
                last_error_code: row.get(18)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;

    rows.into_iter()
        .map(|row| {
            let completion = row
                .completion_event
                .as_deref()
                .and_then(|event| serde_json::from_str::<Value>(event).ok());
            Ok(json!({
                "taskRunId": row.task_run_id,
                "taskId": row.task_id,
                "runtimeRecordId": row.runtime_record_id,
                "correlationId": row.correlation_id,
                "state": row.state,
                "summary": row.summary,
                "lastError": row.last_error,
                "lastErrorCode": row.last_error_code,
                "createdAtMs": row.task_created_at_ms,
                "updatedAtMs": row.task_updated_at_ms,
                "executionInstanceId": row.execution_instance_id,
                "scheduledForMs": row.scheduled_for_ms,
                "runCreatedAtMs": row.run_created_at_ms,
                "scheduleCreatedAtMs": row.schedule_created_at_ms,
                "scheduleUpdatedAtMs": row.schedule_updated_at_ms,
                "scheduleNextRunAtMs": row.schedule_next_run_at_ms,
                "outcome": completion.as_ref().and_then(|value| value.pointer("/payload/outcome")),
                "declinedActions": completion.as_ref().and_then(|value| value.pointer("/payload/actions")),
                "effects": generated_json_array(&row.effects_json, "effect")?,
                "deliveryReceipts": generated_json_array(&row.delivery_receipts_json, "delivery")?,
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{CreateProjectRequest, ProjectDataPolicy};
    use std::fs;

    #[test]
    fn returns_copy_safe_identity_for_pending_and_completed_runs() {
        let root = std::env::temp_dir().join(format!(
            "oomu-routine-history-{}",
            crate::p0_contracts::TaskId::new()
        ));
        fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let project = crate::projects::repository::create(
            &engine,
            CreateProjectRequest {
                name: "Verification history".to_string(),
                description: String::new(),
                data_policy: ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let workflow_id = "workflow-verification-history";
        let routine_id = format!("routine_{}", crate::p0_contracts::TaskId::new());
        let execution_id = "execution-routine-verification-record";
        let task_run_id = "taskrun_66666666-6666-4666-8666-666666666666";
        let task_id = "task_66666666-6666-4666-8666-666666666666";
        let destination_hash = "sensitive-destination-hash";
        let connection = engine.open_connection().unwrap();
        connection.execute(
            "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,is_active,created_at_ms,updated_at_ms,encryption_state,project_id) VALUES (?1,1,'Verification','','{}',1,1,1,'test',?2)",
            params![workflow_id,project.project_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO workflow_schedules(id,workflow_id,workflow_version,label,schedule_expression,run_request_json,is_active,next_run_at_ms,created_at_ms,updated_at_ms,encryption_state,project_id) VALUES (?1,?2,1,'Verification','0 * * * *','{}',1,2000,100,200,'test',?3)",
            params![routine_id,workflow_id,project.project_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO execution_instances(id,workflow_id,workflow_version,status,created_at_ms,updated_at_ms,encryption_state,project_id) VALUES (?1,?2,1,'Running',700,800,'test',?3)",
            params![execution_id,workflow_id,project.project_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO task_runs(task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'workflow',?4,'running','routine','correlation-verification','Verification run',900,1000,'reconciled')",
            params![task_run_id,task_id,project.project_id,execution_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO routine_runs(schedule_id,execution_instance_id,task_run_id,scheduled_for_ms,created_at_ms) VALUES (?1,?2,?3,600,700)",
            params![routine_id,execution_id,task_run_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO task_effects(task_run_id,idempotency_key,effect_kind,state,result_digest,updated_at_ms) VALUES (?1,'calendar-effect','system_calendar_event','verified','calendar-digest',950),(?1,'delivery-effect','routine_channel_delivery','reserved',NULL,960)",
            params![task_run_id],
        ).unwrap();
        connection.execute(
            "INSERT INTO routine_delivery_receipts(receipt_id,schedule_id,task_run_id,platform,destination_hash,event_kind,state,created_at_ms,updated_at_ms) VALUES ('delivery-verification',?1,?2,'slack',?3,'completed','pending',970,980)",
            params![routine_id,task_run_id,destination_hash],
        ).unwrap();
        drop(connection);

        let pending = get(&engine, &routine_id).unwrap();
        assert_eq!(pending[0]["state"], "running");
        assert_eq!(pending[0]["runtimeRecordId"], execution_id);
        assert_eq!(
            pending[0]["effects"][0]["idempotencyKey"],
            "delivery-effect"
        );
        assert_eq!(pending[0]["deliveryReceipts"][0]["state"], "pending");
        assert!(!pending[0].to_string().contains(destination_hash));

        let connection = engine.open_connection().unwrap();
        connection.execute(
            "UPDATE task_runs SET state='completed',completed_at_ms=1100,updated_at_ms=1100 WHERE task_run_id=?1",
            params![task_run_id],
        ).unwrap();
        connection.execute(
            "UPDATE task_effects SET state='verified',result_digest='delivery-digest',updated_at_ms=1100 WHERE task_run_id=?1 AND idempotency_key='delivery-effect'",
            params![task_run_id],
        ).unwrap();
        connection.execute(
            "UPDATE routine_delivery_receipts SET state='delivered',provider_receipt_hash='provider-message-hash',updated_at_ms=1100 WHERE receipt_id='delivery-verification'",
            [],
        ).unwrap();
        drop(connection);

        let completed = get(&engine, &routine_id).unwrap();
        assert_eq!(completed[0]["state"], "completed");
        assert_eq!(completed[0]["deliveryReceipts"][0]["state"], "delivered");
        assert_eq!(
            completed[0]["deliveryReceipts"][0]["providerReceiptHash"],
            "provider-message-hash"
        );
        assert!(!completed[0].to_string().contains(destination_hash));

        let connection = engine.open_connection().unwrap();
        connection.execute(
            "UPDATE execution_instances SET status='Failed',error_json=?2,updated_at_ms=1200 WHERE id=?1",
            params![execution_id,json!({"code":"official_page_fetch_failed","message":"The official page returned HTTP 403."}).to_string()],
        ).unwrap();
        connection.execute(
            "UPDATE task_runs SET state='failed',last_error='The official page returned HTTP 403.',updated_at_ms=1200 WHERE task_run_id=?1",
            params![task_run_id],
        ).unwrap();
        drop(connection);
        let failed = get(&engine, &routine_id).unwrap();
        assert_eq!(failed[0]["state"], "failed");
        assert_eq!(failed[0]["lastErrorCode"], "official_page_fetch_failed");
        assert_eq!(
            failed[0]["lastError"],
            "The official page returned HTTP 403."
        );
        let _ = fs::remove_dir_all(root);
    }
}
