use crate::{agent_manager::AgentManager, inference::InferenceError};

#[derive(Debug, Clone)]
pub(crate) struct ConfiguredCloudRouteSnapshot {
    pub provider_id: String,
    pub model_id: Option<String>,
    pub provider_name: String,
    pub credential_configured: bool,
}

impl ConfiguredCloudRouteSnapshot {
    pub(crate) fn is_runnable(&self) -> bool {
        self.credential_configured && self.model_id.is_some()
    }
}

pub(crate) fn configured_cloud_route_snapshot(
    agent_manager: &AgentManager,
) -> Result<Option<ConfiguredCloudRouteSnapshot>, InferenceError> {
    let target = agent_manager.get_active_auto_route_target().map_err(|error| {
        InferenceError::routing_attention(
            "auto_route_cloud_target_lookup_failed",
            "auto_route_cloud_target",
            format!(
                "Auto-route could not read the configured cloud target. Nothing was sent to a provider. {error}"
            ),
        )
    })?;
    Ok(target.map(|target| ConfiguredCloudRouteSnapshot {
        provider_id: if target.id.trim().starts_with("prov-") {
            target.id.clone()
        } else {
            target.provider_id.clone()
        },
        model_id: first_configured_model_id(&target.custom_model_ids),
        provider_name: target.provider_name,
        credential_configured: target.credential_configured,
    }))
}

pub(super) fn require_configured_cloud_route(
    cloud: Option<&ConfiguredCloudRouteSnapshot>,
) -> Result<(String, String, String), InferenceError> {
    let cloud = cloud.ok_or_else(|| {
        InferenceError::routing_attention(
            "auto_route_cloud_target_missing",
            "auto_route_cloud_target",
            "Auto-route selected advanced work, but no cloud target is configured. Nothing was sent to a provider.",
        )
    })?;
    if !cloud.credential_configured {
        return Err(InferenceError::routing_attention(
            "auto_route_cloud_credential_missing",
            "auto_route_cloud_target",
            format!(
                "Auto-route selected advanced work, but {} is not connected. Add its API key in Providers, then retry. Nothing was sent.",
                cloud.provider_name
            ),
        ));
    }
    let model_id = cloud.model_id.clone().ok_or_else(|| {
        InferenceError::routing_attention(
            "auto_route_cloud_model_missing",
            "auto_route_cloud_target",
            format!(
                "Auto-route selected advanced work, but {} has no configured model. Nothing was sent to a provider.",
                cloud.provider_name
            ),
        )
    })?;
    Ok((
        cloud.provider_id.clone(),
        model_id,
        cloud.provider_name.clone(),
    ))
}

pub(super) fn configured_cloud_route(
    agent_manager: &AgentManager,
) -> Result<(String, String, String), InferenceError> {
    let snapshot = configured_cloud_route_snapshot(agent_manager)?;
    require_configured_cloud_route(snapshot.as_ref())
}

fn first_configured_model_id(custom_model_ids: &str) -> Option<String> {
    custom_model_ids
        .split(|character| character == ',' || character == '\n')
        .map(str::trim)
        .find(|model_id| !model_id.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_manager::ConfiguredProvider;

    #[test]
    fn configured_cloud_target_without_credential_is_not_runnable() {
        let path = std::env::temp_dir().join(format!(
            "oomu-cloud-readiness-{}-{}.db",
            std::process::id(),
            crate::foundation::clock::unix_time_ns_u128()
        ));
        let manager = AgentManager::initialize_at(path.clone()).expect("manager initializes");
        manager
            .upsert_provider_config(ConfiguredProvider {
                id: "prov-cloud-without-key".to_string(),
                provider_id: "google".to_string(),
                provider_name: "Google Gemini".to_string(),
                auth_method: "api_key".to_string(),
                base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
                api_key_label: "GOOGLE_API_KEY".to_string(),
                api_key: None,
                credential_configured: false,
                custom_model_ids: "gemini-3.6-flash".to_string(),
                auto_route_target: true,
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .expect("disconnected target saves");

        let snapshot = configured_cloud_route_snapshot(&manager)
            .expect("target lookup succeeds")
            .expect("target exists");
        assert!(!snapshot.is_runnable());
        let error = require_configured_cloud_route(Some(&snapshot))
            .expect_err("a target without a real credential is not runnable");
        assert_eq!(error.code, "auto_route_cloud_credential_missing");
        let _ = std::fs::remove_file(path);
    }
}
