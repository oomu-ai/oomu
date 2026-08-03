use super::super::*;

#[tauri::command]
pub async fn get_workflows(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<SavedWorkflowProjectionRecord>, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.select_workflows())
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn delete_workflow(
    id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<bool, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.delete_workflow_by_id(&id))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn update_workflow_last_run(
    id: String,
    last_run_at: i64,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<bool, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.update_workflow_last_run(&id, last_run_at))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}
