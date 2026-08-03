use super::*;
use crate::db::PersistenceEngine;
async fn blocking<T: Send + 'static>(
    op: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(op)
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn inspect_capability_bundle(
    request: InspectBundleRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<CapabilityBundleRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::inspect(&engine, request)).await
}
#[tauri::command]
pub async fn list_capability_bundles(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<CapabilityBundleRecord>, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::list(&engine)).await
}
#[tauri::command]
pub async fn activate_capability_bundle(
    request: ActivateBundleRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<CapabilityBundleRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::activate(&engine, request)).await
}
#[tauri::command]
pub async fn disable_capability_bundle(
    request: BundleVersionRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<CapabilityBundleRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::disable(&engine, request)).await
}
#[tauri::command]
pub async fn authorize_bundle_capability(
    request: BundleAuthorityRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<(), String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::authorize(&engine, request)).await
}
#[tauri::command]
pub async fn refresh_capability_registry(
    request: RegistryCatalogRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<RegistryEntry>, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::refresh_catalog(&engine, request)).await
}
#[tauri::command]
pub async fn list_capability_registry(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<RegistryEntry>, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::registry(&engine)).await
}
