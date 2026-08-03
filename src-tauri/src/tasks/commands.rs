use super::*;
use crate::{db::PersistenceEngine, p0_contracts::P0EventEnvelope};

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn list_task_runs(
    filter: TaskFilter,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<TaskRunRecord>, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::list(&engine, filter)).await
}
#[tauri::command]
pub async fn get_task_run(
    request: TaskRunRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<TaskRunRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::get(&engine, &request.task_run_id)).await
}
#[tauri::command]
pub async fn cancel_task_run(
    request: TaskRunRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<TaskRunRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::cancel(&engine, &request.task_run_id)).await
}
#[tauri::command]
pub async fn resume_task_run(
    request: TaskRunRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<TaskRunRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::resume(&engine, &request.task_run_id)).await
}
#[tauri::command]
pub async fn retry_task_run(
    request: TaskRunRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<TaskRunRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::retry(&engine, &request.task_run_id)).await
}
#[tauri::command]
pub async fn acknowledge_task_failure(
    request: TaskRunRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<TaskRunRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::acknowledge(&engine, &request.task_run_id)).await
}
#[tauri::command]
pub async fn reconnect_task_events(
    request: TaskEventsRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<P0EventEnvelope>, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::events(&engine, request)).await
}
#[tauri::command]
pub async fn reconcile_task_runs(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<TaskRecoveryReport, String> {
    let engine = persistence.inner().clone();
    blocking(move || reconcile_all(&engine)).await
}
#[tauri::command]
pub async fn reserve_task_effect(
    request: TaskEffectRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<bool, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::reserve_effect(&engine, request)).await
}
#[tauri::command]
pub async fn verify_task_effect(
    request: TaskEffectRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<(), String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::verify_effect(&engine, request)).await
}

#[tauri::command]
pub async fn resolve_task_effect_verification(
    request: ResolveTaskEffectVerificationRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<TaskRunRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || effect_verification::resolve(&engine, request)).await
}
