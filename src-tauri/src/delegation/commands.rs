use super::*;
use crate::{
    browser_automation::BrowserAutomationManager, db::PersistenceEngine, gemma::GemmaService,
    p0_contracts::EvidenceClass,
};
use futures_util::future::join_all;
use serde::Deserialize;
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListDelegationPlansRequest {
    pub task_run_id: String,
}

fn register(
    runtime: &DelegationRuntime,
    plan_id: &str,
    child_ids: &[String],
) -> Result<Vec<Arc<AtomicBool>>, String> {
    let mut state = runtime
        .cancellations
        .lock()
        .map_err(|_| "Delegation cancellation coordinator is unavailable.".to_string())?;
    if child_ids.iter().any(|id| state.contains_key(id)) {
        return Err("Delegation plan is already running.".into());
    }
    let flags = child_ids
        .iter()
        .map(|id| {
            let flag = Arc::new(AtomicBool::new(false));
            state.insert(id.clone(), flag.clone());
            flag
        })
        .collect();
    state.insert(plan_id.to_string(), Arc::new(AtomicBool::new(false)));
    Ok(flags)
}
fn unregister(runtime: &DelegationRuntime, plan_id: &str, child_ids: &[String]) {
    if let Ok(mut state) = runtime.cancellations.lock() {
        state.remove(plan_id);
        for id in child_ids {
            state.remove(id);
        }
    }
}
fn cancel_flag(runtime: &DelegationRuntime, id: &str) -> bool {
    runtime
        .cancellations
        .lock()
        .ok()
        .and_then(|state| state.get(id).cloned())
        .map(|flag| {
            flag.store(true, Ordering::SeqCst);
            true
        })
        .unwrap_or(false)
}

#[tauri::command]
pub async fn create_delegation_plan(
    request: CreateDelegationPlanRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<DelegationPlanView, String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || repository::create(&engine, &request))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_delegation_plans(
    request: ListDelegationPlansRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<DelegationPlanView>, String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        repository::list_for_task(&engine, &request.task_run_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn execute_delegation_plan(
    request: DelegationPlanRequest,
    runtime: tauri::State<'_, DelegationRuntime>,
    persistence: tauri::State<'_, PersistenceEngine>,
    browser: tauri::State<'_, BrowserAutomationManager>,
    gemma: tauri::State<'_, GemmaService>,
    app: tauri::AppHandle,
) -> Result<DelegationPlanView, String> {
    let engine = persistence.inner().clone();
    let plan = repository::get(&engine, &request.plan_id)?;
    if !matches!(plan.state.as_str(), "planned" | "partial" | "failed") {
        return Err("Delegation plan is not at a safe execution boundary.".into());
    }
    let ids = plan
        .children
        .iter()
        .filter(|c| {
            matches!(
                c.state.as_str(),
                "planned" | "failed" | "incomplete" | "cancelled"
            )
        })
        .map(|c| c.child_run_id.clone())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Err("Delegation plan has no runnable children.".into());
    }
    let flags = register(runtime.inner(), &plan.plan_id, &ids)?;
    repository::set_plan_state(&engine, &plan.plan_id, "running", None)?;
    let mut futures = Vec::new();
    for (id, flag) in ids.iter().cloned().zip(flags) {
        repository::set_child_running(&engine, &plan.plan_id, &id, false)?;
        let proposal = repository::proposal(&engine, &plan.plan_id, &id)?;
        futures.push(worker::execute(
            proposal,
            plan.project_id.clone(),
            plan.task_run_id.clone(),
            app.clone(),
            engine.clone(),
            browser.inner().clone(),
            gemma.inner().clone(),
            flag,
        ));
    }
    let results = join_all(futures).await;
    for (id, result) in ids.iter().zip(results.iter()) {
        match result {
            Ok(value) => repository::finish_child(&engine, &plan.plan_id, id, Ok(value))?,
            Err(code) => repository::finish_child(&engine, &plan.plan_id, id, Err(code))?,
        }
        crate::tools::task_runtime::record_event(
            &engine,
            &plan.task_run_id,
            "delegation.child_finished",
            EvidenceClass::ModelAssertion,
            json!({"planId":plan.plan_id,"childRunId":id,"state":if result.as_ref().is_ok_and(|r|r.complete){"completed"}else{"incomplete"},"mutationAuthority":false,"sourceEvidence":result.as_ref().ok().map(|r|&r.sources)}),
        )?;
    }
    unregister(runtime.inner(), &plan.plan_id, &ids);
    if repository::get(&engine, &plan.plan_id)?.state == "paused" {
        return repository::get(&engine, &plan.plan_id);
    }
    finalize(&engine, &plan.plan_id)
}

fn finalize(engine: &PersistenceEngine, plan_id: &str) -> Result<DelegationPlanView, String> {
    let view = repository::get(engine, plan_id)?;
    repository::create_research_suggestions(engine, &view)?;
    let synthesis = worker::synthesize(&view.children);
    let completed = view
        .children
        .iter()
        .filter(|c| c.state == "completed")
        .count();
    let state = if completed == view.children.len() {
        "completed"
    } else if completed > 0 {
        "partial"
    } else if view.children.iter().all(|c| c.state == "cancelled") {
        "cancelled"
    } else {
        "failed"
    };
    repository::set_plan_state(engine, plan_id, state, Some(&synthesis))?;
    crate::tools::task_runtime::record_event(
        engine,
        &view.task_run_id,
        "delegation.parent_synthesis_ready",
        EvidenceClass::ModelAssertion,
        json!({"planId":plan_id,"state":state,"ready":synthesis.ready_for_parent_synthesis,"incompleteChildRunIds":synthesis.incomplete_child_run_ids,"parentOwnsMutations":true}),
    )?;
    repository::get(engine, plan_id)
}

#[tauri::command]
pub async fn pause_delegation_plan(
    request: DelegationPlanRequest,
    runtime: tauri::State<'_, DelegationRuntime>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<DelegationPlanView, String> {
    let engine = persistence.inner().clone();
    let plan = repository::get(&engine, &request.plan_id)?;
    cancel_flag(runtime.inner(), &plan.plan_id);
    for child in &plan.children {
        cancel_flag(runtime.inner(), &child.child_run_id);
    }
    repository::pause(&engine, &plan.plan_id)?;
    crate::tools::task_runtime::record_event(
        &engine,
        &plan.task_run_id,
        "delegation.plan_paused",
        EvidenceClass::ExecutedMutation,
        json!({"planId":plan.plan_id,"safeBoundary":true}),
    )?;
    repository::get(&engine, &plan.plan_id)
}

#[tauri::command]
pub async fn resume_delegation_plan(
    request: DelegationPlanRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<DelegationPlanView, String> {
    let engine = persistence.inner().clone();
    let plan = repository::get(&engine, &request.plan_id)?;
    repository::resume(&engine, &plan.plan_id)?;
    crate::tools::task_runtime::record_event(
        &engine,
        &plan.task_run_id,
        "delegation.plan_resumed",
        EvidenceClass::ExecutedMutation,
        json!({"planId":plan.plan_id,"fromSafeBoundary":true}),
    )?;
    repository::get(&engine, &plan.plan_id)
}

#[tauri::command]
pub async fn list_work_suggestions(
    request: DelegationPlanRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<WorkSuggestionView>, String> {
    repository::list_suggestions(persistence.inner(), &request.plan_id)
}

#[tauri::command]
pub async fn review_work_suggestion(
    request: SuggestionReviewRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<WorkSuggestionView>, String> {
    let plan = repository::get(persistence.inner(), &request.plan_id)?;
    let result = repository::review_suggestion(persistence.inner(), &request)?;
    crate::tools::task_runtime::record_event(
        persistence.inner(),
        &plan.task_run_id,
        "delegation.suggestion_reviewed",
        EvidenceClass::ExecutedMutation,
        json!({"suggestionId":request.suggestion_id,"accepted":request.accept,"directMutation":false}),
    )?;
    Ok(result)
}

#[tauri::command]
pub async fn cancel_delegation_child(
    request: ChildControlRequest,
    runtime: tauri::State<'_, DelegationRuntime>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<DelegationPlanView, String> {
    let engine = persistence.inner().clone();
    let plan = repository::get(&engine, &request.plan_id)?;
    if !plan
        .children
        .iter()
        .any(|c| c.child_run_id == request.child_run_id)
    {
        return Err("Child does not belong to the requested plan.".into());
    }
    cancel_flag(runtime.inner(), &request.child_run_id);
    repository::cancel_not_started(&engine, &request.plan_id, Some(&request.child_run_id))?;
    crate::tools::task_runtime::record_event(
        &engine,
        &plan.task_run_id,
        "delegation.child_cancelled",
        EvidenceClass::ExecutedMutation,
        json!({"planId":request.plan_id,"childRunId":request.child_run_id,"partialEvidence":"incomplete"}),
    )?;
    repository::get(&engine, &request.plan_id)
}

#[tauri::command]
pub async fn cancel_delegation_plan(
    request: DelegationPlanRequest,
    runtime: tauri::State<'_, DelegationRuntime>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<DelegationPlanView, String> {
    let engine = persistence.inner().clone();
    let plan = repository::get(&engine, &request.plan_id)?;
    cancel_flag(runtime.inner(), &request.plan_id);
    for child in &plan.children {
        cancel_flag(runtime.inner(), &child.child_run_id);
    }
    repository::cancel_not_started(&engine, &request.plan_id, None)?;
    repository::set_plan_state(&engine, &request.plan_id, "cancelled", None)?;
    crate::tools::task_runtime::record_event(
        &engine,
        &plan.task_run_id,
        "delegation.plan_cancelled",
        EvidenceClass::ExecutedMutation,
        json!({"planId":request.plan_id,"childCount":plan.children.len()}),
    )?;
    repository::get(&engine, &request.plan_id)
}

#[tauri::command]
pub async fn retry_delegation_child(
    request: ChildControlRequest,
    runtime: tauri::State<'_, DelegationRuntime>,
    persistence: tauri::State<'_, PersistenceEngine>,
    browser: tauri::State<'_, BrowserAutomationManager>,
    gemma: tauri::State<'_, GemmaService>,
    app: tauri::AppHandle,
) -> Result<DelegationPlanView, String> {
    let engine = persistence.inner().clone();
    let plan = repository::get(&engine, &request.plan_id)?;
    let child = plan
        .children
        .iter()
        .find(|c| c.child_run_id == request.child_run_id)
        .ok_or_else(|| "Child does not belong to the plan.".to_string())?;
    if child.attempt >= 4 {
        return Err("Child retry budget is exhausted.".into());
    }
    if !matches!(child.state.as_str(), "failed" | "incomplete" | "cancelled") {
        return Err("Only failed, incomplete, or cancelled children can be retried safely.".into());
    }
    let flags = register(
        runtime.inner(),
        &plan.plan_id,
        &[child.child_run_id.clone()],
    )?;
    repository::set_plan_state(&engine, &plan.plan_id, "running", None)?;
    repository::set_child_running(&engine, &plan.plan_id, &child.child_run_id, true)?;
    let proposal = repository::proposal(&engine, &plan.plan_id, &child.child_run_id)?;
    let result = worker::execute(
        proposal,
        plan.project_id.clone(),
        plan.task_run_id.clone(),
        app,
        engine.clone(),
        browser.inner().clone(),
        gemma.inner().clone(),
        flags[0].clone(),
    )
    .await;
    match &result {
        Ok(value) => {
            repository::finish_child(&engine, &plan.plan_id, &child.child_run_id, Ok(value))?
        }
        Err(code) => {
            repository::finish_child(&engine, &plan.plan_id, &child.child_run_id, Err(code))?
        }
    }
    unregister(
        runtime.inner(),
        &plan.plan_id,
        &[child.child_run_id.clone()],
    );
    finalize(&engine, &plan.plan_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_mutations_are_parent_only() {
        for action in [
            "file_write",
            "connector_send",
            "artifact_export",
            "approval_grant",
            "delete_file",
        ] {
            assert!(policy::mandatory_parent_only_action(action));
        }
    }
}
