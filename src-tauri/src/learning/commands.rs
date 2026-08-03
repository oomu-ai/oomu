use super::*;
use crate::db::PersistenceEngine;

#[tauri::command]
pub async fn prepare_learning_offer(
    request: TaskLearningRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<LearningOfferView, String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || repository::extract(&engine, &request.task_run_id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_learning_offers(
    request: TaskLearningRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<LearningOfferView>, String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        repository::list_offers(&engine, &request.task_run_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn review_learning_offer(
    request: ReviewLearningOfferRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Option<SavedMethodView>, String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || repository::review(&engine, &request))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn list_saved_methods(
    request: ProjectMethodsRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<SavedMethodView>, String> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        repository::list_methods(&engine, &request.project_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_saved_method_enabled(
    request: MethodControlRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SavedMethodView, String> {
    repository::control(persistence.inner(), &request, "enabled")
}
#[tauri::command]
pub async fn forget_saved_method(
    request: MethodControlRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SavedMethodView, String> {
    repository::control(persistence.inner(), &request, "forget")
}
#[tauri::command]
pub async fn undo_forget_saved_method(
    request: MethodControlRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SavedMethodView, String> {
    repository::control(persistence.inner(), &request, "undo")
}
#[tauri::command]
pub async fn go_back_saved_method(
    request: MethodControlRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SavedMethodView, String> {
    repository::control(persistence.inner(), &request, "go_back")
}
#[tauri::command]
pub async fn edit_saved_method(
    request: MethodControlRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SavedMethodView, String> {
    repository::control(persistence.inner(), &request, "edit")
}

#[tauri::command]
pub async fn export_saved_method(
    request: MethodControlRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Value, String> {
    repository::export(persistence.inner(), &request.method_id)
}
