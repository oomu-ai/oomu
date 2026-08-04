use crate::{agent_manager::ConfiguredProvider, gemma::resolve_canonical_ready_local_model};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AutoRouteIdentityError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl std::fmt::Display for AutoRouteIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AutoRouteIdentityError {}

fn identity_error(
    code: &'static str,
    message: impl Into<String>,
    retryable: bool,
) -> AutoRouteIdentityError {
    AutoRouteIdentityError {
        code,
        boundary: "auto_route_provider_identity",
        message: message.into(),
        retryable,
    }
}

pub(crate) fn provider_supports_model(provider: &ConfiguredProvider, model_id: &str) -> bool {
    provider
        .custom_model_ids
        .split([',', '\n'])
        .map(str::trim)
        .any(|candidate| candidate.eq_ignore_ascii_case(model_id.trim()))
}

pub(crate) fn resolve_verified_auto_route_baseline(
    providers: &[ConfiguredProvider],
    request: &super::AutoRouteSessionBaselineRequest,
    model_root: &Path,
) -> Result<super::VerifiedAutoRouteBaseline, AutoRouteIdentityError> {
    let requested_config = request.provider_config_id.as_str();
    let requested_type = request.provider_type.as_str();
    let requested_model = request.model_id.as_str();
    if request.reasoning_depth.trim().is_empty() || request.context_budget <= 0 {
        return Err(identity_error(
            "auto_route_baseline_incomplete",
            "Choose a complete on-device model configuration before turning on Auto-route.",
            false,
        ));
    }

    let direct = providers
        .iter()
        .find(|provider| provider.id == requested_config);
    let provider = if let Some(provider) = direct {
        provider
    } else if is_local_provider(requested_config) {
        let mut candidates = providers.iter().filter(|provider| {
            is_local_provider(&provider.provider_id)
                && provider_supports_model(provider, requested_model)
        });
        let candidate = candidates.next().ok_or_else(|| {
            identity_error(
                "auto_route_provider_configuration_missing",
                "The selected on-device model configuration is no longer available.",
                true,
            )
        })?;
        if candidates.next().is_some() {
            return Err(identity_error(
                "auto_route_provider_choice_required",
                "Choose which on-device model configuration Auto-route should use.",
                false,
            ));
        }
        candidate
    } else {
        return Err(identity_error(
            "auto_route_provider_configuration_missing",
            "The selected on-device model configuration is no longer available.",
            true,
        ));
    };

    if !is_local_provider(&provider.provider_id) {
        return Err(identity_error(
            "auto_route_provider_not_local",
            "Auto-route needs an on-device model configuration.",
            false,
        ));
    }
    if !provider.provider_id.eq_ignore_ascii_case(requested_type) {
        return Err(identity_error(
            "auto_route_provider_identity_mismatch",
            "The selected model configuration changed. Choose it again before turning on Auto-route.",
            true,
        ));
    }
    if !provider_supports_model(provider, requested_model) {
        return Err(identity_error(
            "auto_route_provider_model_mismatch",
            "The selected on-device model no longer belongs to this configuration.",
            true,
        ));
    }
    let canonical_model = resolve_canonical_ready_local_model(model_root, requested_model)
        .map_err(|error| {
            identity_error(
                error.code,
                format!(
                    "The selected on-device model is not ready. {}",
                    error.message
                ),
                true,
            )
        })?;
    Ok(super::VerifiedAutoRouteBaseline {
        provider_config_id: super::ProviderConfigurationId::try_from(provider.id.clone()).map_err(
            |message| identity_error("auto_route_provider_configuration_missing", message, true),
        )?,
        provider_type: super::ProviderTypeId::try_from(provider.provider_id.clone()).map_err(
            |message| identity_error("auto_route_provider_identity_mismatch", message, true),
        )?,
        model_id: super::CanonicalModelId::try_from(canonical_model.id).map_err(|message| {
            identity_error("auto_route_model_identity_invalid", message, true)
        })?,
        reasoning_depth: request.reasoning_depth.trim().to_string(),
        context_budget: request.context_budget,
        provenance: super::AutoRouteProvenance::ExplicitSession,
    })
}

pub(super) fn canonical_ready_local_baseline(
    model_root: &Path,
    provider_id: &str,
    model_id: &str,
) -> rusqlite::Result<String> {
    if !is_local_provider(provider_id) {
        return Err(rusqlite::Error::InvalidParameterName(
            "Auto-route requires an on-device model.".to_string(),
        ));
    }
    let model =
        resolve_canonical_ready_local_model(model_root, model_id.trim()).map_err(|error| {
            rusqlite::Error::InvalidParameterName(format!(
                "Auto-route model is not ready: {}",
                error.message
            ))
        })?;
    Ok(model.id)
}

pub(crate) fn is_local_provider(provider_id: &str) -> bool {
    matches!(
        provider_id
            .trim()
            .replace('-', "_")
            .to_ascii_lowercase()
            .as_str(),
        "local" | "local_model" | "local_gemma" | "gemma"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_local_provider_resolves_distinct_config_and_type_ids() {
        let model_id = crate::gemma::CLEAN_INSTALL_STARTUP_MODEL_ID;
        let provider = ConfiguredProvider {
            id: "prov-ondevice-e2b".to_string(),
            provider_id: "local_model".to_string(),
            provider_name: "On-device".to_string(),
            auth_method: "none".to_string(),
            base_url: String::new(),
            api_key_label: String::new(),
            api_key: None,
            credential_configured: true,
            custom_model_ids: model_id.to_string(),
            auto_route_target: false,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let request = super::super::AutoRouteSessionBaselineRequest {
            provider_config_id: super::super::ProviderConfigurationId::try_from(
                provider.id.clone(),
            )
            .expect("configuration ID"),
            provider_type: super::super::ProviderTypeId::try_from(provider.provider_id.clone())
                .expect("provider type"),
            model_id: super::super::CanonicalModelId::try_from(model_id.to_string())
                .expect("model ID"),
            reasoning_depth: "medium".to_string(),
            context_budget: 12_288,
        };
        let model_root = crate::db::tests::test_local_models::root();

        let verified = resolve_verified_auto_route_baseline(&[provider], &request, &model_root)
            .expect("the typed configured provider resolves");

        assert_eq!(verified.provider_config_id.as_str(), "prov-ondevice-e2b");
        assert_eq!(verified.provider_type.as_str(), "local_model");
        assert_eq!(verified.model_id.as_str(), model_id);
    }
}
