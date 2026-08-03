//! Receipt-bound activation for the recommended local-model installer.
//!
//! Filesystem promotion is owned by `recommended_model_install`. This module
//! commits the matching settings and provider record only after that package
//! has passed native inspection, then verifies the real resident classifier.

use crate::{
    agent_manager::{AgentManager, ConfiguredProvider},
    gemma::{GemmaError, GemmaService, StartupModelAssignment},
    recommended_model_install::{
        CompletedProviderEvidence, DestinationKind, FinalizationFuture, FinalizationRequest,
        InstallError, PreviousConfiguration, RecommendedModelInstallFinalizer, CANONICAL_MODEL_ID,
        IMMUTABLE_REVISION,
    },
    settings,
};
use std::{collections::BTreeSet, sync::Arc};
use tauri::Manager;

pub(crate) struct AppRecommendedModelFinalizer {
    app: tauri::AppHandle,
}

impl AppRecommendedModelFinalizer {
    pub(crate) fn new(app: tauri::AppHandle) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

impl RecommendedModelInstallFinalizer for AppRecommendedModelFinalizer {
    fn snapshot_previous_configuration(&self) -> Result<PreviousConfiguration, InstallError> {
        let (active_models_root, prewarmed_model_id) =
            settings::snapshot_local_model_configuration(&self.app).map_err(|error| {
                InstallError::new("model_install_configuration_snapshot_failed", true, error)
            })?;
        Ok(PreviousConfiguration {
            active_models_root,
            prewarmed_model_id,
        })
    }

    fn finalize(&self, request: FinalizationRequest) -> FinalizationFuture {
        let app = self.app.clone();
        Box::pin(async move { finalize_recommended_model(app, request).await })
    }
}

async fn finalize_recommended_model(
    app: tauri::AppHandle,
    request: FinalizationRequest,
) -> Result<CompletedProviderEvidence, InstallError> {
    validate_request(&request)?;
    let service = app
        .try_state::<GemmaService>()
        .ok_or_else(|| {
            InstallError::new(
                "model_install_prewarm_unavailable",
                true,
                "the resident local-model service is unavailable",
            )
        })?
        .inner()
        .clone();
    let prior_assignment = ready_assignment(&service);
    let manager = app
        .try_state::<AgentManager>()
        .ok_or_else(|| {
            InstallError::new(
                "model_install_provider_unavailable",
                true,
                "the provider store is unavailable",
            )
        })?
        .inner()
        .clone();
    let previous_provider = load_local_provider(manager.clone()).await?;
    let previous_model_root = settings::resolved_local_model_directory(&app).ok();
    let mut verified_model_ids = ready_model_ids_at(&request.destination_root).await;
    if let Some(root) = previous_model_root.filter(|root| root != &request.destination_root) {
        verified_model_ids.extend(ready_model_ids_at(&root).await);
    }
    let provider = provider_for_install(
        previous_provider,
        localized_local_provider_name(&app),
        &request.canonical_model_id,
        &verified_model_ids,
    );

    settings::commit_verified_local_model_directory(
        &app,
        (request.destination_kind == DestinationKind::Granted)
            .then_some(request.destination_root.as_path()),
    )
    .map_err(|error| InstallError::new("model_install_configuration_failed", true, error))?;

    if let Err(error) =
        settings::set_default_prewarmed_model(app.clone(), request.canonical_model_id.clone()).await
    {
        let rollback = restore_previous_settings(&app, &request.previous_configuration);
        return Err(if rollback.is_ok() {
            InstallError::new("model_install_prewarm_failed", true, error)
        } else {
            InstallError::new(
                "model_install_rollback_failed",
                false,
                "settings rollback failed after local-model preparation",
            )
        });
    }

    match save_provider(manager, provider).await {
        Ok(saved) => Ok(CompletedProviderEvidence::verified_local(saved.id, None)),
        Err(provider_error) => {
            let settings_rollback =
                restore_previous_settings(&app, &request.previous_configuration);
            let runtime_rollback = restore_runtime(service, prior_assignment).await;
            if settings_rollback.is_err() || runtime_rollback.is_err() {
                Err(InstallError::new(
                    "model_install_rollback_failed",
                    false,
                    "provider activation failed and the previous local-model state could not be fully restored",
                ))
            } else {
                Err(provider_error)
            }
        }
    }
}

fn restore_previous_settings(
    app: &tauri::AppHandle,
    previous: &PreviousConfiguration,
) -> Result<(), String> {
    settings::restore_local_model_configuration(
        app,
        previous.active_models_root.as_deref(),
        previous.prewarmed_model_id.as_deref(),
    )
}

fn validate_request(request: &FinalizationRequest) -> Result<(), InstallError> {
    let expected_directory = request.destination_root.join(CANONICAL_MODEL_ID);
    if request.canonical_model_id != CANONICAL_MODEL_ID
        || request.manifest_revision != IMMUTABLE_REVISION
        || request.canonical_model_directory != expected_directory
        || !request.inspection.accepted
        || request.inspection.multimodal_projector_count != 1
    {
        return Err(InstallError::new(
            "model_install_activation_evidence_invalid",
            false,
            "activation request did not match the release-controlled package evidence",
        ));
    }
    Ok(())
}

fn ready_assignment(service: &GemmaService) -> Option<StartupModelAssignment> {
    let health = service.classifier_health();
    service
        .startup_model_assignment()
        .filter(|assignment| health.is_ready() && health.matches_startup_assignment(assignment))
}

async fn load_local_provider(
    manager: AgentManager,
) -> Result<Option<ConfiguredProvider>, InstallError> {
    tauri::async_runtime::spawn_blocking(move || manager.select_provider_configs())
        .await
        .map_err(|error| {
            InstallError::new(
                "model_install_provider_unavailable",
                true,
                error.to_string(),
            )
        })?
        .map_err(|error| {
            InstallError::new(
                "model_install_provider_unavailable",
                true,
                error.to_string(),
            )
        })
        .map(|providers| {
            providers
                .into_iter()
                .find(|provider| is_local_provider(&provider.provider_id))
        })
}

async fn save_provider(
    manager: AgentManager,
    provider: ConfiguredProvider,
) -> Result<ConfiguredProvider, InstallError> {
    tauri::async_runtime::spawn_blocking(move || manager.upsert_provider_config(provider))
        .await
        .map_err(|error| {
            InstallError::new("model_install_provider_failed", true, error.to_string())
        })?
        .map_err(|error| {
            InstallError::new("model_install_provider_failed", true, error.to_string())
        })
}

fn provider_for_install(
    existing: Option<ConfiguredProvider>,
    localized_name: String,
    model_id: &str,
    verified_model_ids: &BTreeSet<String>,
) -> ConfiguredProvider {
    let existing_name = existing
        .as_ref()
        .map(|provider| provider.provider_name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    let custom_model_ids = merge_model_ids(
        model_id,
        existing
            .as_ref()
            .map(|provider| provider.custom_model_ids.as_str())
            .unwrap_or_default(),
        verified_model_ids,
    );
    ConfiguredProvider {
        id: existing
            .as_ref()
            .map(|provider| provider.id.clone())
            .unwrap_or_else(|| "local-model".to_string()),
        provider_id: "local_model".to_string(),
        provider_name: existing_name.unwrap_or(localized_name),
        auth_method: "custom".to_string(),
        base_url: String::new(),
        api_key_label: String::new(),
        api_key: None,
        credential_configured: false,
        custom_model_ids,
        auto_route_target: false,
        created_at_ms: existing
            .as_ref()
            .map(|provider| provider.created_at_ms)
            .unwrap_or_default(),
        updated_at_ms: existing
            .as_ref()
            .map(|provider| provider.updated_at_ms)
            .unwrap_or_default(),
    }
}

async fn ready_model_ids_at(root: &std::path::Path) -> BTreeSet<String> {
    let root = root.to_path_buf();
    match tauri::async_runtime::spawn_blocking(move || {
        crate::gemma::scan_models(&root).map(|models| {
            models
                .into_iter()
                .filter(|model| model.format == "gguf" && model.compatibility == "ready")
                .map(|model| model.id)
                .collect::<Vec<_>>()
        })
    })
    .await
    {
        Ok(Ok(model_ids)) => model_ids
            .into_iter()
            .map(|model_id| model_id.to_ascii_lowercase())
            .collect(),
        Ok(Err(error)) => {
            eprintln!(
                "OOMU_MODEL_INSTALL_EXISTING_MODEL_SCAN_FAILED code={}",
                crate::redaction::redacted_log_text(error.code)
            );
            BTreeSet::new()
        }
        Err(error) => {
            eprintln!(
                "OOMU_MODEL_INSTALL_EXISTING_MODEL_SCAN_FAILED code=worker_join detail={}",
                crate::redaction::redacted_log_text(&error.to_string())
            );
            BTreeSet::new()
        }
    }
}

fn merge_model_ids(
    canonical: &str,
    existing: &str,
    verified_model_ids: &BTreeSet<String>,
) -> String {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for model_id in std::iter::once(canonical).chain(existing.split(['\n', ','])) {
        let model_id = model_id.trim();
        let key = model_id.to_ascii_lowercase();
        let allowed = model_id == canonical || verified_model_ids.contains(&key);
        if !model_id.is_empty() && allowed && seen.insert(key) {
            ordered.push(model_id.to_string());
        }
    }
    ordered.join("\n")
}

fn localized_local_provider_name(app: &tauri::AppHandle) -> String {
    app.try_state::<crate::db::PersistenceEngine>()
        .and_then(|persistence| {
            settings::locale_state_for_engine(persistence.inner(), None)
                .ok()
                .and_then(|state| {
                    state
                        .translations
                        .pointer("/models/provider_names/local_model")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_string)
                })
        })
        .unwrap_or_else(|| crate::gemma::canonical_display_name(CANONICAL_MODEL_ID))
}

async fn restore_runtime(
    service: GemmaService,
    prior_assignment: Option<StartupModelAssignment>,
) -> Result<(), String> {
    let Some(prior_assignment) = prior_assignment else {
        service.enter_degraded(GemmaError {
            code: "recommended_model_activation_rolled_back",
            message: "Recommended model activation was rolled back before completion.".to_string(),
        });
        return Ok(());
    };
    let recovery_epoch = service.mark_classifier_recovering();
    let worker = service.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        worker
            .reconfigure_startup_model_assignment_for_recovery(prior_assignment, recovery_epoch)?;
        worker.verify_classifier_readiness_for_recovery_sync(recovery_epoch)
    })
    .await
    .map_err(|error| error.to_string())?;
    result.map(|_| ()).map_err(|error| error.message)
}

fn is_local_provider(provider_id: &str) -> bool {
    matches!(
        provider_id
            .trim()
            .replace('-', "_")
            .to_ascii_lowercase()
            .as_str(),
        "local" | "local_model" | "local_gemma"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_model_is_first_without_discarding_existing_ready_models() {
        let verified = BTreeSet::from(["other-model".to_string()]);
        assert_eq!(
            merge_model_ids(
                CANONICAL_MODEL_ID,
                "other-model, unverified-model, GEMMA-4-E2B-it-qat-q4_0-gguf",
                &verified,
            ),
            format!("{CANONICAL_MODEL_ID}\nother-model")
        );
    }

    #[test]
    fn local_provider_identity_accepts_only_native_local_aliases() {
        assert!(is_local_provider("local-model"));
        assert!(is_local_provider("LOCAL_GEMMA"));
        assert!(!is_local_provider("google"));
    }
}
