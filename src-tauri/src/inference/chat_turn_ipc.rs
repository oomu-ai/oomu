use super::{
    run_backend_chat_turn, AgentManager, ChatTurnRequest, ChatTurnResponse, GemmaService,
    InferenceError, KnowledgeStore, MemoryLedger, OomuLaunchOptions, PersistenceEngine,
    SovereignIdentity,
};

#[tauri::command]
pub async fn chat_turn(
    request: ChatTurnRequest,
    app: tauri::AppHandle,
    agent_manager: tauri::State<'_, AgentManager>,
    persistence: tauri::State<'_, PersistenceEngine>,
    knowledge: tauri::State<'_, KnowledgeStore>,
    memory_ledger: tauri::State<'_, MemoryLedger>,
    identity: tauri::State<'_, SovereignIdentity>,
    gemma: tauri::State<'_, GemmaService>,
    launch_options: tauri::State<'_, OomuLaunchOptions>,
) -> Result<ChatTurnResponse, InferenceError> {
    validate_public_turn_kind(request.turn_kind.as_deref())?;
    run_backend_chat_turn(
        request,
        app,
        agent_manager.inner().clone(),
        persistence.inner().clone(),
        knowledge.inner().clone(),
        memory_ledger.inner().clone(),
        identity.inner().clone(),
        gemma.inner().clone(),
        launch_options.inner().safe_mode,
    )
    .await
}

fn validate_public_turn_kind(turn_kind: Option<&str>) -> Result<(), InferenceError> {
    if turn_kind == Some(crate::db::AUTO_TURN_KIND) {
        return Err(InferenceError::invalid(
            "Background completion turns can only be created by OOMU's native execution runtime.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_public_turn_kind;

    #[test]
    fn renderer_cannot_inject_a_native_background_turn() {
        let error = validate_public_turn_kind(Some(crate::db::AUTO_TURN_KIND))
            .expect_err("the renderer must not claim native background authority");
        assert_eq!(error.code, "invalid_request");
        assert!(validate_public_turn_kind(Some("root")).is_ok());
    }
}
