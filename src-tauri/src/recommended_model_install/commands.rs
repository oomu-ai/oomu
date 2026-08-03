use serde::Serialize;
use std::sync::Arc;
use tauri::State;

use super::{
    InstallCommandError, InstallEventSink, InstallPhase, InstallProgress,
    RecommendedModelInstallState, RecommendedModelInstaller, TauriInstallEventSink,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationGrantResponse {
    pub location_grant_id: String,
    pub display_path: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartInstallResponse {
    pub install_id: String,
    pub attached: bool,
    pub progress: InstallProgress,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardPartialResponse {
    pub discarded: bool,
    pub state: InstallPhase,
}

#[tauri::command]
pub fn get_recommended_model_install_state(
    installer: State<'_, RecommendedModelInstaller>,
) -> RecommendedModelInstallState {
    installer.state()
}

#[tauri::command(rename_all = "camelCase")]
pub async fn choose_recommended_model_install_location(
    installer: State<'_, RecommendedModelInstaller>,
    dialog_title: String,
) -> Result<Option<LocationGrantResponse>, InstallCommandError> {
    installer
        .choose_location(dialog_title)
        .await
        .map_err(Into::into)
}

#[tauri::command(rename_all = "camelCase")]
pub fn start_recommended_model_install(
    app: tauri::AppHandle,
    installer: State<'_, RecommendedModelInstaller>,
    location_grant_id: Option<String>,
) -> Result<StartInstallResponse, InstallCommandError> {
    installer
        .start(
            location_grant_id,
            Arc::new(TauriInstallEventSink(app)) as Arc<dyn InstallEventSink>,
        )
        .map_err(Into::into)
}

#[tauri::command(rename_all = "camelCase")]
pub fn cancel_recommended_model_install(
    installer: State<'_, RecommendedModelInstaller>,
    install_id: String,
) -> Result<InstallProgress, InstallCommandError> {
    installer.cancel(&install_id).map_err(Into::into)
}

#[tauri::command(rename_all = "camelCase")]
pub fn discard_recommended_model_partial(
    installer: State<'_, RecommendedModelInstaller>,
    install_id: String,
) -> Result<DiscardPartialResponse, InstallCommandError> {
    installer.discard(&install_id).map_err(Into::into)
}
