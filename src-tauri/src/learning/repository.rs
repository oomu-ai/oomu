use super::*;
use crate::{
    db::PersistenceEngine,
    foundation::{clock::unix_time_ms_i64, digest::sha256_hex},
    p0_contracts::{EvidenceClass, ProjectId, TaskRunId},
};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

fn random_id(prefix: &str) -> String {
    let mut bytes = [0u8; 18];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}_{}", hex::encode(bytes))
}

fn evidence_refs(engine: &PersistenceEngine, task_run_id: &str) -> Result<Vec<Value>, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare("SELECT event_json FROM task_events WHERE task_run_id=?1 ORDER BY sequence")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![task_run_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let mut refs = Vec::new();
    for raw in rows {
        let value: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
        let class = value
            .get("evidenceClass")
            .and_then(Value::as_str)
            .unwrap_or("");
        if matches!(
            class,
            "observed_result" | "verified_postcondition" | "signed_artifact"
        ) {
            refs.push(json!({
                "eventId": value.get("eventId").cloned().unwrap_or(Value::Null),
                "eventType": value.get("eventType").cloned().unwrap_or(Value::Null),
                "evidenceClass": class,
            }));
        }
    }
    Ok(refs)
}

pub(crate) fn extract(
    engine: &PersistenceEngine,
    task_run_id: &str,
) -> Result<LearningOfferView, String> {
    TaskRunId::parse(task_run_id)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let task = connection
        .query_row(
            "SELECT task_id,project_id,state,summary FROM task_runs WHERE task_run_id=?1",
            params![task_run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Task was not found.".to_string())?;
    let project_id = task
        .1
        .ok_or_else(|| "Only Project work can become a saved method.".to_string())?;
    ProjectId::parse(&project_id)?;
    if task.2 != "completed" {
        return Err("OOMU can only learn from completed work.".into());
    }
    let summary = task.3.trim();
    if summary.len() < 3 || summary.chars().count() > 240 || forbidden_learning_text(summary) {
        return Err("This Task does not contain a safe, reusable description.".into());
    }
    drop(connection);
    let evidence = evidence_refs(engine, task_run_id)?;
    if evidence.is_empty() {
        return Err("This Task does not yet have verified results to learn from.".into());
    }
    let offer_id = random_id("learning");
    let method = json!({
        "schemaVersion": LEARNING_SCHEMA_VERSION,
        "name": summary,
        "inputs": ["Project context supplied by the Task"],
        "output": "A result that matches the reviewed Task goal",
        "preconditions": ["The Project is available", "Required services are connected"],
        "capabilities": ["project_read"],
        "budgets": {"wallTimeMs": 600000, "toolCalls": 24, "mutations": 0},
        "approvals": ["Any consequential change still needs normal approval"],
        "fallbacks": ["Stop and explain what is missing"],
        "postconditions": ["The result is non-empty", "Source evidence remains linked"],
        "sourceTaskRunIds": [task_run_id],
    });
    let now = unix_time_ms_i64();
    engine.open_connection().map_err(|error| error.to_string())?.execute(
        "INSERT INTO learning_offers (offer_id,project_id,task_id,task_run_id,kind,status,summary,proposed_method_json,source_evidence_json,exposure_summary,conflict_summary,created_at_ms) VALUES (?1,?2,?3,?4,'procedure','proposed',?5,?6,?7,'Uses only the Project access this Task already had.','No conflicting saved method was found.',?8) ON CONFLICT(task_run_id,kind) DO NOTHING",
        params![offer_id,project_id,task.0,task_run_id,summary,method.to_string(),serde_json::to_string(&evidence).map_err(|error|error.to_string())?,now],
    ).map_err(|error| error.to_string())?;
    let mut offers = list_offers(engine, task_run_id)?;
    offers
        .pop()
        .ok_or_else(|| "OOMU could not prepare this learning review.".to_string())
}

pub(crate) fn list_offers(
    engine: &PersistenceEngine,
    task_run_id: &str,
) -> Result<Vec<LearningOfferView>, String> {
    TaskRunId::parse(task_run_id)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement = connection.prepare("SELECT offer_id,project_id,task_run_id,status,summary,source_evidence_json,exposure_summary,conflict_summary,created_at_ms FROM learning_offers WHERE task_run_id=?1 ORDER BY created_at_ms").map_err(|error|error.to_string())?;
    let rows = statement
        .query_map(params![task_run_id], |row| {
            let raw: String = row.get(5)?;
            let evidence: Vec<Value> = serde_json::from_str(&raw).unwrap_or_default();
            Ok(LearningOfferView {
                offer_id: row.get(0)?,
                project_id: row.get(1)?,
                task_run_id: row.get(2)?,
                status: row.get(3)?,
                summary: row.get(4)?,
                source_task_count: 1,
                evidence_count: evidence.len(),
                exposure_summary: row.get(6)?,
                conflict_summary: row.get(7)?,
                created_at_ms: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

pub(crate) fn review(
    engine: &PersistenceEngine,
    request: &ReviewLearningOfferRequest,
) -> Result<Option<SavedMethodView>, String> {
    let action = request.action.as_str();
    if !matches!(
        action,
        "remember_project" | "remember_everywhere" | "no_thanks" | "ask_later"
    ) {
        return Err("Unknown learning choice.".into());
    }
    if action == "remember_everywhere" && !request.use_everywhere_confirmed {
        return Err("Using this in every Project needs a second confirmation.".into());
    }
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let offer = connection.query_row("SELECT project_id,task_run_id,status,summary,proposed_method_json FROM learning_offers WHERE offer_id=?1",params![request.offer_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?))).optional().map_err(|error|error.to_string())?.ok_or_else(||"Learning offer was not found.".to_string())?;
    if offer.2 != "proposed" && offer.2 != "postponed" {
        return Err("This learning offer was already reviewed.".into());
    }
    let now = unix_time_ms_i64();
    if action == "no_thanks" || action == "ask_later" {
        let status = if action == "no_thanks" {
            "rejected"
        } else {
            "postponed"
        };
        connection
            .execute(
                "UPDATE learning_offers SET status=?2,reviewed_at_ms=?3 WHERE offer_id=?1",
                params![request.offer_id, status, now],
            )
            .map_err(|error| error.to_string())?;
        return Ok(None);
    }
    let summary = request.edited_summary.as_deref().unwrap_or(&offer.3).trim();
    if summary.len() < 3 || summary.chars().count() > 500 || forbidden_learning_text(summary) {
        return Err("The saved wording contains information OOMU must not learn.".into());
    }
    let mut method_json: Value =
        serde_json::from_str(&offer.4).map_err(|error| error.to_string())?;
    method_json["name"] = Value::String(summary.to_string());
    let method_id = random_id("method");
    let project_scope = (action == "remember_project").then_some(offer.0.clone());
    let tx = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    tx.execute("UPDATE learning_offers SET status='accepted',summary=?2,reviewed_at_ms=?3 WHERE offer_id=?1",params![request.offer_id,summary,now]).map_err(|error|error.to_string())?;
    tx.execute("INSERT INTO saved_methods (method_id,source_offer_id,project_id,name,summary,current_version,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,?4,?4,1,?5,?5)",params![method_id,request.offer_id,project_scope,summary,now]).map_err(|error|error.to_string())?;
    tx.execute("INSERT INTO saved_method_versions (method_id,version,method_json,source_task_run_id,change_summary,created_at_ms) VALUES (?1,1,?2,?3,'First reviewed version',?4)",params![method_id,method_json.to_string(),offer.1,now]).map_err(|error|error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    crate::tools::task_runtime::record_event(
        engine,
        &offer.1,
        "learning.method_remembered",
        EvidenceClass::ExecutedMutation,
        json!({"methodId":method_id,"projectOnly":project_scope.is_some(),"newAuthority":false,"methodDigest":sha256_hex(method_json.to_string().as_bytes())}),
    )?;
    Ok(Some(get_method(engine, &method_id)?))
}

fn history(
    connection: &rusqlite::Connection,
    method_id: &str,
) -> Result<Vec<MethodVersionView>, String> {
    let mut statement=connection.prepare("SELECT version,change_summary,created_at_ms FROM saved_method_versions WHERE method_id=?1 ORDER BY version DESC").map_err(|error|error.to_string())?;
    let rows = statement
        .query_map(params![method_id], |row| {
            Ok(MethodVersionView {
                version: row.get::<_, i64>(0)? as u64,
                summary: row.get(1)?,
                created_at_ms: row.get(2)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn get_method(engine: &PersistenceEngine, method_id: &str) -> Result<SavedMethodView, String> {
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let row=connection.query_row("SELECT m.method_id,m.project_id,m.name,m.summary,m.current_version,m.enabled,m.use_count,m.successful_use_count,m.intervention_count,m.deleted_at_ms,m.created_at_ms,m.updated_at_ms,v.method_json FROM saved_methods m JOIN saved_method_versions v ON v.method_id=m.method_id AND v.version=m.current_version WHERE m.method_id=?1",params![method_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,i64>(4)?,row.get::<_,i64>(5)?,row.get::<_,i64>(6)?,row.get::<_,i64>(7)?,row.get::<_,i64>(8)?,row.get::<_,Option<i64>>(9)?,row.get::<_,i64>(10)?,row.get::<_,i64>(11)?,row.get::<_,String>(12)?))).optional().map_err(|error|error.to_string())?.ok_or_else(||"Saved method was not found.".to_string())?;
    Ok(SavedMethodView {
        method_id: row.0.clone(),
        project_id: row.1,
        name: row.2,
        summary: row.3,
        current_version: row.4 as u64,
        enabled: row.5 != 0,
        use_count: row.6 as u64,
        successful_use_count: row.7 as u64,
        intervention_count: row.8 as u64,
        deleted_at_ms: row.9,
        created_at_ms: row.10,
        updated_at_ms: row.11,
        method: serde_json::from_str(&row.12).map_err(|error| error.to_string())?,
        history: history(&connection, &row.0)?,
    })
}

pub(crate) fn list_methods(
    engine: &PersistenceEngine,
    project_id: &str,
) -> Result<Vec<SavedMethodView>, String> {
    ProjectId::parse(project_id)?;
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    let mut statement=connection.prepare("SELECT method_id FROM saved_methods WHERE deleted_at_ms IS NULL AND (project_id=?1 OR project_id IS NULL) ORDER BY updated_at_ms DESC").map_err(|error|error.to_string())?;
    let ids = statement
        .query_map(params![project_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    drop(connection);
    ids.into_iter().map(|id| get_method(engine, &id)).collect()
}

pub(crate) fn control(
    engine: &PersistenceEngine,
    request: &MethodControlRequest,
    operation: &str,
) -> Result<SavedMethodView, String> {
    let now = unix_time_ms_i64();
    match operation {
        "enabled" => {
            let enabled = request
                .enabled
                .ok_or_else(|| "Choose whether to turn this method on or off.".to_string())?;
            engine.open_connection().map_err(|e|e.to_string())?.execute("UPDATE saved_methods SET enabled=?2,updated_at_ms=?3 WHERE method_id=?1 AND deleted_at_ms IS NULL",params![request.method_id,enabled as i64,now]).map_err(|e|e.to_string())?;
        }
        "forget" => {
            engine.open_connection().map_err(|e|e.to_string())?.execute("UPDATE saved_methods SET deleted_at_ms=?2,enabled=0,updated_at_ms=?2 WHERE method_id=?1 AND deleted_at_ms IS NULL",params![request.method_id,now]).map_err(|e|e.to_string())?;
        }
        "undo" => {
            engine.open_connection().map_err(|e|e.to_string())?.execute("UPDATE saved_methods SET deleted_at_ms=NULL,enabled=1,updated_at_ms=?2 WHERE method_id=?1 AND deleted_at_ms IS NOT NULL AND deleted_at_ms>=?3",params![request.method_id,now,now-10_000]).map_err(|e|e.to_string())?;
        }
        "go_back" => {
            let version = request
                .version
                .ok_or_else(|| "Choose an earlier version.".to_string())?
                as i64;
            let connection = engine.open_connection().map_err(|e| e.to_string())?;
            let exists: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM saved_method_versions WHERE method_id=?1 AND version=?2",
                    params![request.method_id, version],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if exists != 1 {
                return Err("That earlier version is unavailable.".into());
            }
            connection.execute("UPDATE saved_methods SET current_version=?2,updated_at_ms=?3 WHERE method_id=?1 AND deleted_at_ms IS NULL",params![request.method_id,version,now]).map_err(|e|e.to_string())?;
        }
        "edit" => {
            let summary = request.summary.as_deref().unwrap_or("").trim();
            if summary.len() < 3
                || summary.chars().count() > 500
                || forbidden_learning_text(summary)
            {
                return Err("The saved wording contains information OOMU must not learn.".into());
            }
            let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
            let current=connection.query_row("SELECT m.current_version,v.method_json,o.task_run_id FROM saved_methods m JOIN saved_method_versions v ON v.method_id=m.method_id AND v.version=m.current_version JOIN learning_offers o ON o.offer_id=m.source_offer_id WHERE m.method_id=?1 AND m.deleted_at_ms IS NULL",params![request.method_id],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).optional().map_err(|e|e.to_string())?.ok_or_else(||"Saved method was not found.".to_string())?;
            let mut method: Value = serde_json::from_str(&current.1).map_err(|e| e.to_string())?;
            method["name"] = Value::String(summary.to_string());
            let version = current.0 + 1;
            let tx = connection.transaction().map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO saved_method_versions (method_id,version,method_json,source_task_run_id,change_summary,created_at_ms) VALUES (?1,?2,?3,?4,'Edited by the user',?5)",params![request.method_id,version,method.to_string(),current.2,now]).map_err(|e|e.to_string())?;
            tx.execute("UPDATE saved_methods SET name=?2,summary=?2,current_version=?3,updated_at_ms=?4 WHERE method_id=?1",params![request.method_id,summary,version,now]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        _ => return Err("Unknown saved method action.".into()),
    }
    get_method(engine, &request.method_id)
}

pub(crate) fn export(engine: &PersistenceEngine, method_id: &str) -> Result<Value, String> {
    let method = get_method(engine, method_id)?;
    Ok(
        json!({"schemaVersion":LEARNING_SCHEMA_VERSION,"name":method.name,"projectId":method.project_id,"currentVersion":method.current_version,"method":method.method,"history":method.history}),
    )
}
