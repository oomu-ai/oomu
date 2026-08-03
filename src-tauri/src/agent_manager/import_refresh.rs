use super::*;

pub(super) async fn target_agent<'a>(
    manager: &AgentManager,
    context: (&'a ExecuteAgentImportRequest, &'a AgentMetadata, &'a str),
) -> Result<(String, AgentConfig), AgentManagerError> {
    let (request, metadata, system_prompt) = context;
    let target = request
        .target_agent_id
        .as_deref()
        .map(|value| guard_agent_config_text("target_agent_id", value))
        .transpose()
        .map_err(|error| AgentManagerError::persistence(error.to_string()))?;
    let agent_id = target
        .clone()
        .unwrap_or_else(|| format!("imported_{}", unix_time_ms()));
    let config = if target.is_some() {
        manager
            .get_active_agent_config(agent_id.clone())
            .await
            .map_err(AgentManagerError::persistence)?
            .ok_or_else(|| {
                AgentManagerError::authorization("The agent to refresh was not found.".to_string())
            })?
    } else {
        manager
            .save_agent_config(
                SaveAgentConfigRequest {
                    id: agent_id.clone(),
                    name: request.agent_name.clone(),
                    system_prompt: system_prompt.to_string(),
                    model_id: request.model_id.clone(),
                    provider_id: Some(request.provider_id.clone()),
                    description: Some(request.agent_description.clone()),
                    image: None,
                    personality_profile: Some(imported_agent_personality_profile(
                        request, metadata,
                    )),
                    favorited: Some(false),
                    status: Some(AgentConfigStatus::Active),
                },
                "imported_configuration",
            )
            .await
            .map_err(AgentManagerError::persistence)?
    };
    Ok((agent_id, config))
}
