use super::*;
use crate::db::PersistenceEngine;
use tauri::Manager;

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn create_project(
    request: CreateProjectRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ProjectRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::create(&engine, request)).await
}

#[tauri::command]
pub async fn list_projects(
    include_archived: Option<bool>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<ProjectRecord>, String> {
    let engine = persistence.inner().clone();
    blocking(move || repository::list(&engine, include_archived.unwrap_or(false))).await
}

#[tauri::command]
pub async fn get_project(
    request: ProjectIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ProjectRecord, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let engine = persistence.inner().clone();
    blocking(move || repository::get(&engine, &request.project_id)).await
}

#[tauri::command]
pub async fn update_project(
    request: UpdateProjectRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ProjectRecord, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let engine = persistence.inner().clone();
    blocking(move || repository::update(&engine, request)).await
}

#[tauri::command]
pub async fn archive_project(
    request: ProjectIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ProjectRecord, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let engine = persistence.inner().clone();
    blocking(move || repository::archive(&engine, &request.project_id)).await
}

#[tauri::command]
pub async fn preview_project_deletion(
    request: ProjectIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<ProjectDeletionPreview, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let engine = persistence.inner().clone();
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    blocking(move || repository::deletion_preview(&engine, &request.project_id, &app_data)).await
}

#[tauri::command]
pub async fn delete_project(
    request: DeleteProjectRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    knowledge: tauri::State<'_, crate::knowledge::KnowledgeStore>,
    memory: tauri::State<'_, crate::memory_ledger::MemoryLedger>,
    app: tauri::AppHandle,
) -> Result<ProjectDeletionPreview, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let engine = persistence.inner().clone();
    let knowledge = knowledge.inner().clone();
    let memory = memory.inner().clone();
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    blocking(move || repository::delete(&engine, &knowledge, &memory, &app_data, request)).await
}

#[tauri::command]
pub async fn attach_project_source(
    request: AttachProjectSourceRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ProjectSourceRecord, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let engine = persistence.inner().clone();
    blocking(move || repository::attach_source(&engine, request)).await
}

#[tauri::command]
pub async fn choose_project_root(
    request: ProjectIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Option<ProjectSourceRecord>, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let Some(handle) = rfd::AsyncFileDialog::new().pick_folder().await else {
        return Ok(None);
    };
    let engine = persistence.inner().clone();
    let project_id = request.project_id;
    let path = handle.path().to_path_buf();
    blocking(move || repository::attach_picked_root(&engine, &project_id, &path))
        .await
        .map(Some)
}

#[tauri::command]
pub async fn list_project_sources(
    request: ProjectIdRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<ProjectSourceRecord>, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let engine = persistence.inner().clone();
    blocking(move || repository::list_sources(&engine, &request.project_id)).await
}

#[tauri::command]
pub async fn refresh_project_source(
    request: ProjectSourceRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ProjectSourceRecord, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let engine = persistence.inner().clone();
    blocking(move || repository::refresh_source(&engine, request)).await
}

#[tauri::command]
pub async fn revoke_project_source(
    request: ProjectSourceRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ProjectSourceRecord, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let engine = persistence.inner().clone();
    blocking(move || repository::revoke_source(&engine, request)).await
}

#[tauri::command]
pub async fn set_project_instructions(
    request: SetProjectInstructionsRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ProjectRecord, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let engine = persistence.inner().clone();
    blocking(move || repository::set_instructions(&engine, request)).await
}

#[tauri::command]
pub async fn set_project_policy(
    request: SetProjectPolicyRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ProjectRecord, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let engine = persistence.inner().clone();
    blocking(move || repository::set_policy(&engine, request)).await
}

#[tauri::command]
pub async fn bind_project_record(
    request: BindProjectRecordRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<(), String> {
    if let Some(project_id) = request.project_id.as_deref() {
        repository::user_managed_project_id(project_id)?;
    }
    let engine = persistence.inner().clone();
    blocking(move || repository::bind_record(&engine, request)).await
}

#[tauri::command]
pub async fn project_policy_preflight(
    request: ProjectTransmissionRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ProjectTransmissionResult, String> {
    repository::user_managed_project_id(&request.project_id)?;
    let engine = persistence.inner().clone();
    blocking(move || evaluate_project_policy(&engine, request)).await
}
