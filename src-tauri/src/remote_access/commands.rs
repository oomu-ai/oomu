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
pub async fn list_remote_devices(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<RemoteDeviceRecord>, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::list(&engine)).await
}
#[tauri::command]
pub async fn rename_remote_device(
    request: RenameRemoteDeviceRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<RemoteDeviceRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::rename(&engine, request)).await
}
#[tauri::command]
pub async fn revoke_remote_device(
    request: RemoteDeviceRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<RemoteDeviceRecord, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::repository::revoke(&engine, &request.remote_device_id)).await
}
#[tauri::command]
pub async fn execute_remote_command(
    request: SignedRemoteCommand,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, crate::sovereign_identity::SovereignIdentity>,
) -> Result<RemoteCommandResult, String> {
    let engine = persistence.inner().clone();
    let signer = identity.inner().clone();
    blocking(move || super::repository::execute(&engine, &signer, request)).await
}
#[tauri::command]
pub async fn retrieve_remote_artifact(
    request: RetrieveRemoteArtifactRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<EncryptedRemoteArtifact, String> {
    let engine = persistence.inner().clone();
    blocking(move || super::artifact_transfer::retrieve(&engine, request)).await
}
