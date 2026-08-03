use super::super::routing_persistence::{
    canonical_model_route_key, routing_preference_from_user_record,
};
use super::super::*;
use crate::agent_manager::{AgentManager, ConfiguredProvider};

#[tauri::command]
pub async fn get_routing_preference(
    key: Option<String>,
    route_key: Option<String>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Option<RoutingPreferenceRecord>, AgenticLoopError> {
    let engine = persistence.inner().clone();
    match routing_lookup_argument(key, route_key) {
        Some(key) => {
            tauri::async_runtime::spawn_blocking(move || engine.select_routing_preference(&key))
                .await
                .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
                .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
        }
        None => tauri::async_runtime::spawn_blocking(move || {
            engine
                .select_user_routing_preference("default")
                .map(|record| record.map(routing_preference_from_user_record))
        })
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string())),
    }
}

#[tauri::command]
pub async fn save_routing_preference(
    primary_route_id: Option<String>,
    fallback_route_id: Option<String>,
    route_key: Option<String>,
    model_id: Option<String>,
    provider_id: Option<String>,
    provider_config_id: Option<String>,
    label: Option<String>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<(), AgenticLoopError> {
    let primary_route_id = clean_optional_text(primary_route_id);
    let fallback_route_id = clean_optional_text(fallback_route_id);
    if primary_route_id.is_some() || fallback_route_id.is_some() {
        let primary_route_id = primary_route_id.ok_or_else(|| {
            AgenticLoopError::from_persistence(
                "Routing preference primary_route_id cannot be empty.".to_string(),
            )
        })?;
        let fallback_route_id = fallback_route_id.ok_or_else(|| {
            AgenticLoopError::from_persistence(
                "Routing preference fallback_route_id cannot be empty.".to_string(),
            )
        })?;
        let engine = persistence.inner().clone();
        return tauri::async_runtime::spawn_blocking(move || {
            engine.upsert_user_routing_preference_pair(
                "default",
                &primary_route_id,
                &fallback_route_id,
            )
        })
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()));
    }

    let route_key = route_key.ok_or_else(|| {
        AgenticLoopError::from_persistence(
            "Routing preference route_key must be primary or fallback.".to_string(),
        )
    })?;
    let provider_id = provider_id.ok_or_else(|| {
        AgenticLoopError::from_persistence(
            "Routing preference provider_id cannot be empty.".to_string(),
        )
    })?;
    let model_id = model_id.ok_or_else(|| {
        AgenticLoopError::from_persistence(
            "Routing preference model_id cannot be empty.".to_string(),
        )
    })?;
    let canonical_key = canonical_model_route_key(&route_key)
        .ok_or_else(|| {
            AgenticLoopError::from_persistence(
                "Routing preference route_key must be primary or fallback.".to_string(),
            )
        })?
        .to_string();
    let provider_id = clean_required_routing_text("provider_id", provider_id)?;
    let model_id = clean_required_routing_text("model_id", model_id)?;
    let provider_config_id = clean_optional_text(provider_config_id);
    let label = clean_optional_text(label);
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.upsert_model_routing_preference(
            &canonical_key,
            &provider_id,
            provider_config_id.as_deref(),
            &model_id,
            label.as_deref(),
        )
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn get_session_config(
    session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Option<SessionConfigRecord>, AgenticLoopError> {
    let session_id = clean_session_config_id(session_id)?;
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.select_session_config(&session_id))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn save_session_config(
    session_id: String,
    reasoning_depth: String,
    context_budget: i32,
    provider_id: Option<String>,
    model_id: Option<String>,
    app: tauri::AppHandle,
    agent_manager: tauri::State<'_, AgentManager>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<(), AgenticLoopError> {
    let session_id = clean_session_config_id(session_id)?;
    let provider_config_id = clean_optional_text(provider_id);
    let context_budget = clean_context_budget(context_budget)?;
    let model_id = clean_optional_text(model_id);
    if provider_config_id.is_some() != model_id.is_some() {
        return Err(session_provider_identity_error());
    }
    let model_root = crate::settings::resolved_local_model_directory(&app)
        .map_err(AgenticLoopError::from_persistence)?;
    let manager = agent_manager.inner().clone();
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AgenticLoopError> {
        let _provider_guard = manager.lock_writes();
        let provider_identity = match (provider_config_id.as_deref(), model_id.as_deref()) {
            (Some(requested_provider_id), Some(requested_model_id)) => {
                let providers = manager
                    .select_provider_configs_metadata_locked()
                    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
                let provider = resolve_session_provider_configuration(
                    &providers,
                    requested_provider_id,
                    requested_model_id,
                )?;
                Some((provider.id.clone(), provider.provider_id.clone()))
            }
            (None, None) => None,
            _ => return Err(session_provider_identity_error()),
        };
        let canonical_provider_config_id = provider_identity
            .as_ref()
            .map(|(provider_config_id, _)| provider_config_id.as_str());
        let provider_type = provider_identity
            .as_ref()
            .map(|(_, provider_type)| provider_type.as_str());
        let reasoning_depth = clean_session_reasoning_depth(provider_type, reasoning_depth)?;
        let model_id = match (provider_type, model_id) {
            (Some(provider_type), Some(model_id))
                if super::super::auto_route_validation::is_local_provider(provider_type) =>
            {
                Some(
                    super::super::auto_route_validation::canonical_ready_local_baseline(
                        &model_root,
                        provider_type,
                        &model_id,
                    )
                    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?,
                )
            }
            (_, model_id) => model_id,
        };
        engine
            .upsert_session_config(
                &session_id,
                &reasoning_depth,
                context_budget,
                canonical_provider_config_id,
                provider_type,
                model_id.as_deref(),
            )
            .map_err(session_config_error)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
}

fn resolve_session_provider_configuration<'a>(
    providers: &'a [ConfiguredProvider],
    requested_provider_id: &str,
    model_id: &str,
) -> Result<&'a ConfiguredProvider, AgenticLoopError> {
    if let Some(provider) = providers
        .iter()
        .find(|provider| provider.id == requested_provider_id)
    {
        return Ok(provider);
    }
    if !super::super::auto_route_validation::is_local_provider(requested_provider_id) {
        return Err(session_provider_identity_error());
    }

    let mut candidates = providers.iter().filter(|provider| {
        super::super::auto_route_validation::is_local_provider(&provider.provider_id)
            && super::super::auto_route_validation::provider_supports_model(provider, model_id)
    });
    let provider = candidates
        .next()
        .ok_or_else(session_provider_identity_error)?;
    if candidates.next().is_some() {
        return Err(session_provider_identity_error());
    }
    Ok(provider)
}

fn session_provider_identity_error() -> AgenticLoopError {
    super::auto_route::domain_error(
        "session_provider_identity_invalid",
        "session_provider_identity",
        "Choose an available model provider before saving this chat.",
    )
}

fn session_config_error(error: rusqlite::Error) -> AgenticLoopError {
    if matches!(
        &error,
        rusqlite::Error::InvalidParameterName(code)
            if code == super::super::routing_persistence::AUTO_ROUTE_LEGACY_SESSION_CONFIG_FORBIDDEN
    ) {
        return super::auto_route::domain_error(
            "auto_route_session_config_authority_required",
            "auto_route_activation",
            "Auto-route model changes must use the verified Auto-route control.",
        );
    }
    AgenticLoopError::from_persistence(error.to_string())
}

fn clean_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(
        id: &str,
        provider_type: &str,
        model_ids: &str,
    ) -> crate::agent_manager::ConfiguredProvider {
        crate::agent_manager::ConfiguredProvider {
            id: id.to_string(),
            provider_id: provider_type.to_string(),
            provider_name: id.to_string(),
            auth_method: "none".to_string(),
            base_url: String::new(),
            api_key_label: String::new(),
            api_key: None,
            credential_configured: true,
            custom_model_ids: model_ids.to_string(),
            auto_route_target: false,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn session_provider_resolution_accepts_exact_configuration_identity() {
        let providers = vec![provider("prov-gemini", "google", "gemini-3.6-flash")];

        let resolved =
            resolve_session_provider_configuration(&providers, "prov-gemini", "gemini-3.6-flash")
                .expect("exact provider configuration identity");

        assert_eq!(resolved.id, "prov-gemini");
        assert_eq!(resolved.provider_id, "google");
    }

    #[test]
    fn session_provider_resolution_canonicalizes_unique_local_alias() {
        let model_id = crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID;
        let providers = vec![provider("prov-ondevice-e2b", "local_model", model_id)];

        let resolved = resolve_session_provider_configuration(&providers, "local_model", model_id)
            .expect("unique local provider alias");

        assert_eq!(resolved.id, "prov-ondevice-e2b");
        assert_eq!(resolved.provider_id, "local_model");
    }

    #[test]
    fn session_provider_resolution_rejects_ambiguous_local_alias() {
        let model_id = crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID;
        let providers = vec![
            provider("prov-ondevice-a", "local_model", model_id),
            provider("prov-ondevice-b", "local_model", model_id),
        ];

        let error = resolve_session_provider_configuration(&providers, "local_model", model_id)
            .expect_err("ambiguous local aliases must fail closed");

        assert_eq!(error.code, "session_provider_identity_invalid");
    }

    #[test]
    fn session_provider_resolution_does_not_resolve_cloud_type_aliases() {
        let providers = vec![provider("prov-gemini", "google", "gemini-3.6-flash")];

        let error =
            resolve_session_provider_configuration(&providers, "google", "gemini-3.6-flash")
                .expect_err("cloud provider type aliases must not bypass configuration identity");

        assert_eq!(error.code, "session_provider_identity_invalid");
    }
}

#[tauri::command]
pub async fn set_routing_preference(
    key: String,
    value: String,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<(), AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.upsert_routing_preference(&key, &value))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}
