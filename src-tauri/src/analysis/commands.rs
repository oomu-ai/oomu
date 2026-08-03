use super::*;
use crate::db::PersistenceEngine;
#[tauri::command]
pub async fn run_project_data_analysis(
    request: RunAnalysisRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<AnalysisView, String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || repository::run(&engine, &request))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn list_task_analyses(
    request: ListAnalysisRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<AnalysisView>, String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || repository::list(&engine, &request.task_run_id))
        .await
        .map_err(|e| e.to_string())?
}
