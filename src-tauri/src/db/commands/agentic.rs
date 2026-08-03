use super::super::*;

#[tauri::command]
pub async fn save_agentic_state(
    request: SaveAgenticStateRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<PersistenceResponse, AgenticLoopError> {
    persistence
        .save_intent(request.plan)
        .await
        .map_err(AgenticLoopError::from_persistence)?;

    Ok(PersistenceResponse {
        db_path: PRIVATE_PERSISTENCE_STORE_ID.to_string(),
        message: "Agentic intent metadata cached locally.".to_string(),
    })
}

#[tauri::command]
pub async fn get_agentic_state(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<AgenticState, AgenticLoopError> {
    persistence
        .load_state()
        .await
        .map_err(AgenticLoopError::from_persistence)
}

#[tauri::command]
pub async fn get_recoverable_actions(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<RecoverableAction>, AgenticLoopError> {
    let state = persistence
        .load_state()
        .await
        .map_err(AgenticLoopError::from_persistence)?;
    Ok(state.recoverable_actions)
}
