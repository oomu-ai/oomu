use super::super::*;

#[tauri::command]
pub async fn list_channel_configs(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<ChannelConfigSummary>, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.select_channel_config_summaries())
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn save_channel_config(
    request: SaveChannelConfigRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    gateway: tauri::State<'_, crate::gateway::SovereignGatewayService>,
    identity: tauri::State<'_, crate::sovereign_identity::SovereignIdentity>,
) -> Result<ChannelConfigSummary, AgenticLoopError> {
    if request.is_active {
        let existing = persistence
            .inner()
            .select_channel_config(&request.platform)
            .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
        let credentials = request
            .credentials_json
            .as_deref()
            .or_else(|| {
                existing
                    .as_ref()
                    .map(|config| config.credentials_json.as_str())
            })
            .unwrap_or("{}");
        let owner_id = request.owner_id.as_deref().or_else(|| {
            existing
                .as_ref()
                .and_then(|config| config.owner_id.as_deref())
        });
        crate::gateway::validate_channel_activation(&request.platform, credentials, owner_id)
            .await
            .map_err(AgenticLoopError::from_persistence)?;
        crate::gateway::validate_slack_channel_authority(
            &request.platform,
            credentials,
            owner_id,
            persistence.inner().clone(),
            identity.inner().clone(),
        )
        .await
        .map_err(AgenticLoopError::from_persistence)?;
    }
    let engine = persistence.inner().clone();
    let saved = tauri::async_runtime::spawn_blocking(move || engine.upsert_channel_config(request))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
    gateway
        .refresh_workers(persistence.inner())
        .await
        .map_err(AgenticLoopError::from_persistence)?;
    Ok(ChannelConfigSummary::from(&saved))
}
