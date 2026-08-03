use super::*;
use crate::{
    db::PersistenceEngine,
    foundation::clock::unix_time_ms_i64,
    p0_contracts::{EvidenceClass, ProjectId, TaskRunId},
};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, OptionalExtension};
use serde_json::json;

fn plan_id() -> String {
    let mut bytes = [0u8; 18];
    OsRng.fill_bytes(&mut bytes);
    format!("delegation_{}", hex::encode(bytes))
}

pub(crate) fn create(
    engine: &PersistenceEngine,
    request: &CreateDelegationPlanRequest,
) -> Result<DelegationPlanView, String> {
    policy::validate(request)?;
    ProjectId::parse(&request.project_id)?;
    TaskRunId::parse(&request.task_run_id)?;
    crate::tools::task_runtime::require_bound_task(
        engine,
        &request.task_run_id,
        &request.project_id,
    )?;
    let id = plan_id();
    let now = unix_time_ms_i64();
    let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO delegation_plans (plan_id,project_id,task_run_id,parent_session_id,parent_model_route,state,required_child_count,aggregate_budget_json,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,?4,?5,'planned',?6,?7,?8,?8)",params![id,request.project_id,request.task_run_id,request.parent_session_id,request.parent_model_route,request.children.len() as i64,serde_json::to_string(&request.aggregate_budget).map_err(|e|e.to_string())?,now]).map_err(|e|e.to_string())?;
    for (ordinal, child) in request.children.iter().enumerate() {
        let child_id = child_id();
        tx.execute("INSERT INTO delegation_child_runs (child_run_id,plan_id,ordinal,goal,source_scope_json,tool_scope_json,model_route,budget_json,state,progress_summary,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'planned','Waiting for parent to start the plan.',?9,?9)",params![child_id,id,ordinal as i64,child.goal,serde_json::to_string(&child.sources).map_err(|e|e.to_string())?,serde_json::to_string(&child.allowed_read_tools).map_err(|e|e.to_string())?,child.model_route,serde_json::to_string(&child.budget).map_err(|e|e.to_string())?,now]).map_err(|e|e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    crate::tools::task_runtime::record_event(
        engine,
        &request.task_run_id,
        "delegation.plan_proposed",
        EvidenceClass::ModelAssertion,
        json!({"planId":id,"childCount":request.children.len(),"readOnly":true,"maxConcurrency":MAX_PARALLEL_CHILDREN}),
    )?;
    get(engine, &id)
}

pub(crate) fn get(engine: &PersistenceEngine, id: &str) -> Result<DelegationPlanView, String> {
    if !id.starts_with("delegation_") {
        return Err("Invalid delegation plan identifier.".into());
    }
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let mut plan=connection.query_row("SELECT plan_id,project_id,task_run_id,parent_model_route,state,aggregate_budget_json,synthesis_json,created_at_ms,updated_at_ms FROM delegation_plans WHERE plan_id=?1",params![id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,Option<String>>(6)?,row.get::<_,i64>(7)?,row.get::<_,i64>(8)?))).optional().map_err(|e|e.to_string())?.ok_or_else(||"Delegation plan was not found.".to_string())?;
    let children = list_children(&connection, id)?;
    Ok(DelegationPlanView {
        plan_id: plan.0,
        project_id: plan.1,
        task_run_id: plan.2,
        parent_model_route: plan.3,
        state: plan.4,
        aggregate_budget: serde_json::from_str(&plan.5).map_err(|e| e.to_string())?,
        synthesis: plan
            .6
            .take()
            .map(|v| serde_json::from_str(&v).map_err(|e| e.to_string()))
            .transpose()?,
        children,
        created_at_ms: plan.7,
        updated_at_ms: plan.8,
    })
}

pub(crate) fn list_for_task(
    engine: &PersistenceEngine,
    task_run_id: &str,
) -> Result<Vec<DelegationPlanView>, String> {
    TaskRunId::parse(task_run_id)?;
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let mut statement = connection
        .prepare(
            "SELECT plan_id FROM delegation_plans WHERE task_run_id=?1 ORDER BY updated_at_ms DESC",
        )
        .map_err(|e| e.to_string())?;
    let ids = statement
        .query_map(params![task_run_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    drop(connection);
    ids.into_iter().map(|id| get(engine, &id)).collect()
}

fn list_children(
    connection: &rusqlite::Connection,
    plan_id: &str,
) -> Result<Vec<ChildRunView>, String> {
    let mut statement=connection.prepare("SELECT child_run_id,goal,source_scope_json,tool_scope_json,model_route,budget_json,state,progress_summary,result_json,error_code,attempt FROM delegation_child_runs WHERE plan_id=?1 ORDER BY ordinal").map_err(|e|e.to_string())?;
    let rows = statement
        .query_map(params![plan_id], |row| {
            let raw_sources: String = row.get(2)?;
            let sources: Vec<DelegatedSource> =
                serde_json::from_str(&raw_sources).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
            let raw_tools: String = row.get(3)?;
            let raw_budget: String = row.get(5)?;
            let result: Option<String> = row.get(8)?;
            Ok(ChildRunView {
                child_run_id: row.get(0)?,
                goal: row.get(1)?,
                source_scope: sources.iter().map(|v| v.kind().to_string()).collect(),
                allowed_read_tools: serde_json::from_str(&raw_tools).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                model_route: row.get(4)?,
                budget: serde_json::from_str(&raw_budget).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                state: row.get(6)?,
                progress_summary: row.get(7)?,
                result: result
                    .map(|v| {
                        serde_json::from_str(&v).map_err(|e| {
                            rusqlite::Error::FromSqlConversionFailure(
                                8,
                                rusqlite::types::Type::Text,
                                Box::new(e),
                            )
                        })
                    })
                    .transpose()?,
                error_code: row.get(9)?,
                attempt: row.get::<_, i64>(10)? as u8,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub(crate) fn proposal(
    engine: &PersistenceEngine,
    plan_id: &str,
    child_run_id: &str,
) -> Result<ChildProposal, String> {
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    connection.query_row("SELECT goal,source_scope_json,tool_scope_json,model_route,budget_json FROM delegation_child_runs WHERE plan_id=?1 AND child_run_id=?2",params![plan_id,child_run_id],|row|{let sources:String=row.get(1)?;let tools:String=row.get(2)?;let budget:String=row.get(4)?;Ok(ChildProposal{goal:row.get(0)?,expected_output_schema:"findings_sources_uncertainties_v1".into(),sources:serde_json::from_str(&sources).map_err(|e|rusqlite::Error::FromSqlConversionFailure(1,rusqlite::types::Type::Text,Box::new(e)))?,allowed_read_tools:serde_json::from_str(&tools).map_err(|e|rusqlite::Error::FromSqlConversionFailure(2,rusqlite::types::Type::Text,Box::new(e)))?,model_route:row.get(3)?,budget:serde_json::from_str(&budget).map_err(|e|rusqlite::Error::FromSqlConversionFailure(4,rusqlite::types::Type::Text,Box::new(e)))?})}).optional().map_err(|e|e.to_string())?.ok_or_else(||"Delegated child was not found.".into())
}

pub(crate) fn set_plan_state(
    engine: &PersistenceEngine,
    plan_id: &str,
    state: &str,
    synthesis: Option<&DelegationSynthesis>,
) -> Result<(), String> {
    let now = unix_time_ms_i64();
    let completed = matches!(state, "completed" | "partial" | "failed" | "cancelled");
    let changed=engine.open_connection().map_err(|e|e.to_string())?.execute("UPDATE delegation_plans SET state=?2,synthesis_json=?3,updated_at_ms=?4,completed_at_ms=CASE WHEN ?5 THEN ?4 ELSE NULL END WHERE plan_id=?1",params![plan_id,state,synthesis.map(serde_json::to_string).transpose().map_err(|e|e.to_string())?,now,completed]).map_err(|e|e.to_string())?;
    if changed == 0 {
        Err("Delegation plan was not found.".into())
    } else {
        Ok(())
    }
}

pub(crate) fn set_child_running(
    engine: &PersistenceEngine,
    plan_id: &str,
    child_run_id: &str,
    retry: bool,
) -> Result<(), String> {
    let now = unix_time_ms_i64();
    let changed=engine.open_connection().map_err(|e|e.to_string())?.execute("UPDATE delegation_child_runs SET state='running',progress_summary='Reading only the explicitly delegated sources.',result_json=NULL,error_code=NULL,started_at_ms=?3,completed_at_ms=NULL,updated_at_ms=?3,attempt=attempt+?4 WHERE plan_id=?1 AND child_run_id=?2 AND state IN ('planned','failed','incomplete','cancelled')",params![plan_id,child_run_id,now,retry as i64]).map_err(|e|e.to_string())?;
    if changed == 0 {
        Err("Child is not at a safe start or retry boundary.".into())
    } else {
        Ok(())
    }
}

pub(crate) fn finish_child(
    engine: &PersistenceEngine,
    plan_id: &str,
    child_run_id: &str,
    result: Result<&ChildResult, &str>,
) -> Result<(), String> {
    let now = unix_time_ms_i64();
    let current: Option<String> = engine
        .open_connection()
        .map_err(|e| e.to_string())?
        .query_row(
            "SELECT state FROM delegation_child_runs WHERE plan_id=?1 AND child_run_id=?2",
            params![plan_id, child_run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if current.as_deref() == Some("paused") {
        return Ok(());
    }
    let (state, summary, encoded, error) = match result {
        Ok(value) => (
            if value.complete {
                "completed"
            } else {
                "incomplete"
            },
            if value.complete {
                "Structured findings are ready for parent synthesis."
            } else {
                "Partial evidence is retained and labeled incomplete."
            },
            Some(serde_json::to_string(value).map_err(|e| e.to_string())?),
            None,
        ),
        Err(code) => (
            if code == "cancelled" {
                "cancelled"
            } else {
                "failed"
            },
            if code == "cancelled" {
                "Cancelled; partial work is not treated as complete."
            } else {
                "Child failed without mutation authority."
            },
            None,
            Some(code),
        ),
    };
    engine.open_connection().map_err(|e|e.to_string())?.execute("UPDATE delegation_child_runs SET state=?3,progress_summary=?4,result_json=?5,error_code=?6,completed_at_ms=?7,updated_at_ms=?7 WHERE plan_id=?1 AND child_run_id=?2",params![plan_id,child_run_id,state,summary,encoded,error,now]).map_err(|e|e.to_string())?;
    Ok(())
}

pub(crate) fn pause(engine: &PersistenceEngine, plan_id: &str) -> Result<(), String> {
    let now = unix_time_ms_i64();
    let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    let changed=tx.execute("UPDATE delegation_plans SET state='paused',updated_at_ms=?2 WHERE plan_id=?1 AND state IN ('planned','running')",params![plan_id,now]).map_err(|e|e.to_string())?;
    if changed == 0 {
        return Err("Helpers can only pause while waiting or working.".into());
    }
    tx.execute("UPDATE delegation_child_runs SET state='paused',progress_summary='Paused at a safe boundary.',updated_at_ms=?2 WHERE plan_id=?1 AND state IN ('planned','running')",params![plan_id,now]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn resume(engine: &PersistenceEngine, plan_id: &str) -> Result<(), String> {
    let now = unix_time_ms_i64();
    let mut connection = engine.open_connection().map_err(|e| e.to_string())?;
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    let changed=tx.execute("UPDATE delegation_plans SET state='planned',updated_at_ms=?2 WHERE plan_id=?1 AND state='paused'",params![plan_id,now]).map_err(|e|e.to_string())?;
    if changed == 0 {
        return Err("Helpers are not paused.".into());
    }
    tx.execute("UPDATE delegation_child_runs SET state='planned',progress_summary='Ready to continue.',updated_at_ms=?2 WHERE plan_id=?1 AND state='paused'",params![plan_id,now]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn create_research_suggestions(
    engine: &PersistenceEngine,
    plan: &DelegationPlanView,
) -> Result<(), String> {
    let now = unix_time_ms_i64();
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    for child in &plan.children {
        if let Some(result) = &child.result {
            for (index, finding) in result.findings.iter().enumerate() {
                let key = crate::foundation::digest::sha256_hex(
                    format!("{}:{}:{}", plan.plan_id, child.child_run_id, index).as_bytes(),
                );
                let id = format!("suggestion_{}", &key[..36]);
                let payload = serde_json::json!({"finding":finding,"mutationAuthority":false});
                connection.execute("INSERT OR IGNORE INTO work_graph_suggestions (suggestion_id,plan_id,child_run_id,task_run_id,kind,base_revision,idempotency_key,summary,payload_json,state,created_at_ms) VALUES (?1,?2,?3,?4,'research_note',?5,?6,?7,?8,'awaiting_review',?9)",params![id,plan.plan_id,child.child_run_id,plan.task_run_id,plan.updated_at_ms.to_string(),key,finding.statement,payload.to_string(),now]).map_err(|e|e.to_string())?;
            }
        }
    }
    Ok(())
}

pub(crate) fn list_suggestions(
    engine: &PersistenceEngine,
    plan_id: &str,
) -> Result<Vec<WorkSuggestionView>, String> {
    let connection = engine.open_connection().map_err(|e| e.to_string())?;
    let mut statement=connection.prepare("SELECT suggestion_id,child_run_id,kind,summary,state,rejection_reason FROM work_graph_suggestions WHERE plan_id=?1 ORDER BY created_at_ms").map_err(|e|e.to_string())?;
    let rows = statement
        .query_map(params![plan_id], |row| {
            Ok(WorkSuggestionView {
                suggestion_id: row.get(0)?,
                child_run_id: row.get(1)?,
                kind: row.get(2)?,
                summary: row.get(3)?,
                state: row.get(4)?,
                rejection_reason: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub(crate) fn review_suggestion(
    engine: &PersistenceEngine,
    request: &SuggestionReviewRequest,
) -> Result<Vec<WorkSuggestionView>, String> {
    let reason = request.rejection_reason.as_deref().unwrap_or("").trim();
    if !request.accept && reason.is_empty() {
        return Err("Explain why OOMU should leave this suggestion out.".into());
    }
    let now = unix_time_ms_i64();
    let state = if request.accept {
        "accepted"
    } else {
        "rejected"
    };
    let changed=engine.open_connection().map_err(|e|e.to_string())?.execute("UPDATE work_graph_suggestions SET state=?3,rejection_reason=?4,reviewed_at_ms=?5 WHERE plan_id=?1 AND suggestion_id=?2 AND state='awaiting_review'",params![request.plan_id,request.suggestion_id,state,(!request.accept).then_some(reason),now]).map_err(|e|e.to_string())?;
    if changed == 0 {
        return Err("This suggestion is unavailable or already reviewed.".into());
    }
    list_suggestions(engine, &request.plan_id)
}

pub(crate) fn cancel_not_started(
    engine: &PersistenceEngine,
    plan_id: &str,
    child_run_id: Option<&str>,
) -> Result<(), String> {
    let now = unix_time_ms_i64();
    let sql = if child_run_id.is_some() {
        "UPDATE delegation_child_runs SET state='cancelled',progress_summary='Cancelled before completion.',error_code='cancelled',completed_at_ms=?3,updated_at_ms=?3 WHERE plan_id=?1 AND child_run_id=?2 AND state IN ('planned','running')"
    } else {
        "UPDATE delegation_child_runs SET state='cancelled',progress_summary='Cancelled before completion.',error_code='cancelled',completed_at_ms=?3,updated_at_ms=?3 WHERE plan_id=?1 AND state IN ('planned','running')"
    };
    engine
        .open_connection()
        .map_err(|e| e.to_string())?
        .execute(sql, params![plan_id, child_run_id, now])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        p0_contracts::{TaskId, TaskRunId},
        projects::{CreateProjectRequest, ProjectDataPolicy},
    };

    fn require_bound_fixture(
        engine: &PersistenceEngine,
        task_run_id: &str,
        project_id: &str,
    ) -> Result<(), String> {
        let count: i64 = engine
            .open_connection()
            .map_err(|error| error.to_string())?
            .query_row(
                "SELECT COUNT(*) FROM task_runs WHERE task_run_id=?1 AND project_id=?2",
                params![task_run_id, project_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        (count == 1)
            .then_some(())
            .ok_or_else(|| "task_binding_fixture_mismatch".to_string())
    }

    fn record_event_fixture(
        engine: &PersistenceEngine,
        task_run_id: &str,
        _event_type: &str,
        _evidence: EvidenceClass,
        _payload: serde_json::Value,
    ) -> Result<(), String> {
        let count: i64 = engine
            .open_connection()
            .map_err(|error| error.to_string())?
            .query_row(
                "SELECT COUNT(*) FROM task_runs WHERE task_run_id=?1",
                params![task_run_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        (count == 1)
            .then_some(())
            .ok_or_else(|| "task_event_fixture_missing".to_string())
    }

    fn require_agent_runtime_fixture(
        _engine: &PersistenceEngine,
        _execution_id: &str,
    ) -> Result<crate::tools::task_runtime::AgentRuntimeTaskBinding, String> {
        Err("agent_runtime_fixture_not_available".to_string())
    }

    fn record_event_with_sequence_fixture(
        engine: &PersistenceEngine,
        task_run_id: &str,
        event_type: &str,
        evidence: EvidenceClass,
        payload: serde_json::Value,
    ) -> Result<u64, String> {
        record_event_fixture(engine, task_run_id, event_type, evidence, payload)?;
        Ok(1)
    }

    #[test]
    fn durable_plan_keeps_three_children_under_one_parent_task() {
        crate::tools::task_runtime::register(crate::tools::task_runtime::TaskRuntimeRegistration {
            record_event: record_event_fixture,
            record_event_with_sequence: record_event_with_sequence_fixture,
            require_bound_task: require_bound_fixture,
            require_agent_runtime_task: require_agent_runtime_fixture,
        })
        .unwrap();
        let root =
            std::env::temp_dir().join(format!("oomu-delegation-store-{}", unix_time_ms_i64()));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let project = crate::projects::repository::create(
            &engine,
            CreateProjectRequest {
                name: "Research".into(),
                description: String::new(),
                data_policy: ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let task_id = TaskId::new().to_string();
        let task_run_id = TaskRunId::new().to_string();
        let now = unix_time_ms_i64();
        engine.open_connection().unwrap().execute("INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'taskflow','delegation-test','running','test',?2,'Research',?4,?4,'not_required')",params![task_run_id,task_id,project.project_id,now]).unwrap();
        let child = ChildProposal {
            goal: "Inspect independent source".into(),
            expected_output_schema: "findings_sources_uncertainties_v1".into(),
            sources: vec![DelegatedSource::InlineText {
                label: "source".into(),
                content: "observed material".into(),
            }],
            allowed_read_tools: vec!["summarize_text".into()],
            model_route: "local".into(),
            budget: ResourceBudget {
                max_input_tokens: 256,
                max_output_tokens: 128,
                max_tool_calls: 1,
                timeout_ms: 10_000,
                max_response_bytes: 16_384,
            },
        };
        let plan = create(
            &engine,
            &CreateDelegationPlanRequest {
                schema_version: 1,
                project_id: project.project_id,
                task_run_id,
                parent_session_id: Some("parent".into()),
                parent_model_route: "local".into(),
                parent_depth: 0,
                aggregate_budget: AggregateBudget {
                    max_input_tokens: 1024,
                    max_output_tokens: 512,
                    max_tool_calls: 3,
                    timeout_ms: 20_000,
                },
                children: vec![child; 3],
            },
        )
        .unwrap();
        assert_eq!(plan.children.len(), 3);
        assert!(plan.children.iter().all(|item| item.state == "planned"));
        let _ = std::fs::remove_dir_all(root);
    }
}
