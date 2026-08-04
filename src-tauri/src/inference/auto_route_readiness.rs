use super::{dynamic_routing, InferenceError, SessionRouteSnapshot};
use crate::{
    agent_manager::AgentManager,
    db::{ChatSessionRoutePolicyRecord, PersistenceEngine},
    gemma::{
        resolve_canonical_ready_local_model, AutoRouteClassifierStatus, GemmaError, GemmaService,
        LocalModelOption, StartupModelAssignment,
    },
    settings,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoRouteSessionReadiness {
    pub status: String,
    pub session_id: String,
    pub dynamic_binding_valid: bool,
    pub classifier_model_id: Option<String>,
    pub classifier_ready: bool,
    pub local_provider_id: Option<String>,
    pub local_provider_type: Option<String>,
    pub local_model_id: Option<String>,
    pub route_generation: i64,
    pub local_model_ready: bool,
    pub recommended_local_provider_id: Option<String>,
    pub recommended_local_model_id: Option<String>,
    pub context_budget_valid: bool,
    pub cloud_target_required: bool,
    pub cloud_target_ready: bool,
    pub storage_ready: bool,
    pub audit_ready: bool,
    pub readiness_generation: u64,
    pub last_verified_at_ms: Option<i64>,
    pub failure_code: Option<String>,
    pub failure_boundary: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairAutoRouteSessionBaselineRequest {
    pub session_id: String,
    pub turn_id: String,
    pub generation_token: String,
    pub local_provider_id: String,
    pub local_model_id: String,
}

#[tauri::command]
pub async fn get_auto_route_session_readiness(
    session_id: String,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
    agent_manager: tauri::State<'_, AgentManager>,
    gemma: tauri::State<'_, GemmaService>,
) -> Result<AutoRouteSessionReadiness, InferenceError> {
    Ok(readiness_snapshot(
        &session_id,
        &app,
        persistence.inner(),
        agent_manager.inner(),
        gemma.inner(),
    ))
}

#[tauri::command]
pub async fn repair_auto_route_session_baseline(
    request: RepairAutoRouteSessionBaselineRequest,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
    agent_manager: tauri::State<'_, AgentManager>,
    gemma: tauri::State<'_, GemmaService>,
) -> Result<AutoRouteSessionReadiness, InferenceError> {
    let model_root = settings::resolved_local_model_directory(&app).map_err(|message| {
        repair_error(
            "local_model_directory_unavailable",
            "local_model_store",
            message,
        )
    })?;
    let model = resolve_auto_route_local_model(&model_root, &request.local_model_id)
        .map_err(|error| repair_error(error.code, "auto_route_session_baseline", error.message))?;
    let providers = agent_manager
        .select_provider_configs_metadata()
        .map_err(|error| {
            repair_error(
                "auto_route_provider_store_unavailable",
                "auto_route_provider_identity",
                error.to_string(),
            )
        })?;
    let requested_provider = request.local_provider_id.trim();
    let provider = if let Some(provider) = providers
        .iter()
        .find(|provider| provider.id == requested_provider)
    {
        provider
    } else if is_local_provider(requested_provider) {
        let mut matches = providers.iter().filter(|provider| {
            crate::db::auto_route_validation::is_local_provider(&provider.provider_id)
                && crate::db::auto_route_validation::provider_supports_model(provider, &model.id)
        });
        let provider = matches.next().ok_or_else(|| {
            repair_error(
                "auto_route_provider_configuration_missing",
                "auto_route_provider_identity",
                "Choose an on-device model configuration to repair this chat.",
            )
        })?;
        if matches.next().is_some() {
            return Err(repair_error(
                "auto_route_provider_choice_required",
                "auto_route_provider_identity",
                "Choose which on-device model configuration this chat should use.",
            ));
        }
        provider
    } else {
        return Err(repair_error(
            "auto_route_provider_configuration_missing",
            "auto_route_provider_identity",
            "Choose an on-device model configuration to repair this chat.",
        ));
    };
    if !crate::db::auto_route_validation::is_local_provider(&provider.provider_id)
        || !crate::db::auto_route_validation::provider_supports_model(provider, &model.id)
    {
        return Err(repair_error(
            "auto_route_provider_model_mismatch",
            "auto_route_provider_identity",
            "The selected on-device model does not belong to this configuration.",
        ));
    }
    let engine = persistence.inner().clone();
    let session_id = request.session_id.trim().to_string();
    let session_id_for_write = session_id.clone();
    let canonical_model_id = model.id;
    let provider_config_id = provider.id.clone();
    let provider_type = provider.provider_id.clone();
    let model_root_for_write = model_root;
    tauri::async_runtime::spawn_blocking(move || {
        engine.repair_auto_route_session_baseline(
            &session_id_for_write,
            &request.turn_id,
            &request.generation_token,
            &provider_config_id,
            &provider_type,
            &canonical_model_id,
            &model_root_for_write,
        )
    })
    .await
    .map_err(|error| {
        repair_error(
            "auto_route_repair_worker_failed",
            "auto_route_session_baseline",
            error.to_string(),
        )
    })?
    .map_err(|error| {
        repair_error(
            "auto_route_repair_persistence_failed",
            "auto_route_session_baseline",
            error.to_string(),
        )
    })?;

    Ok(readiness_snapshot(
        &session_id,
        &app,
        persistence.inner(),
        agent_manager.inner(),
        gemma.inner(),
    ))
}

fn require_current_classifier_assignment(
    app: &tauri::AppHandle,
    gemma: &GemmaService,
) -> Result<(), InferenceError> {
    let model_root = settings::resolved_local_model_directory(app).map_err(|message| {
        repair_error(
            "local_model_directory_unavailable",
            "local_model_store",
            message,
        )
    })?;
    let preference = settings::resolved_startup_model_preference(app).map_err(|message| {
        repair_error(
            "auto_route_startup_assignment_unavailable",
            "auto_route_classifier_assignment",
            message,
        )
    })?;
    let assignment =
        crate::gemma::resolve_verified_startup_model_assignment(&model_root, &preference).map_err(
            |error| {
                repair_error(
                    error.code,
                    "auto_route_classifier_assignment",
                    error.message,
                )
            },
        )?;
    if gemma
        .classifier_health()
        .matches_startup_assignment(&assignment)
    {
        return Ok(());
    }
    Err(repair_error(
        "auto_route_classifier_assignment_changed",
        "auto_route_classifier_assignment",
        "The selected on-device model changed. OOMU must finish preparing it before Auto-route can continue.",
    ))
}

pub(super) fn source<'a>(
    snapshot: &'a SessionRouteSnapshot,
    app: &tauri::AppHandle,
    gemma: &GemmaService,
    agent_manager: &AgentManager,
) -> Result<&'a str, InferenceError> {
    require_current_classifier_assignment(app, gemma)?;
    verified_provider_identity(snapshot, agent_manager)?;
    verified_local_source(snapshot)
}

fn verified_provider_identity(
    snapshot: &SessionRouteSnapshot,
    agent_manager: &AgentManager,
) -> Result<(), InferenceError> {
    let config_id = snapshot
        .local_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            repair_error(
                "auto_route_provider_configuration_missing",
                "auto_route_provider_identity",
                "This chat needs an on-device model configuration before Auto-route can continue.",
            )
        })?;
    let provider_type = snapshot
        .local_provider_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            repair_error(
                "auto_route_provider_identity_mismatch",
                "auto_route_provider_identity",
                "This chat's on-device model configuration changed. Choose it again to continue.",
            )
        })?;
    let model_id = snapshot
        .local_model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            repair_error(
                "auto_route_model_identity_invalid",
                "auto_route_provider_identity",
                "This chat needs an on-device model before Auto-route can continue.",
            )
        })?;
    if snapshot.route_generation <= 0 {
        return Err(repair_error(
            "auto_route_route_generation_unverified",
            "auto_route_provider_identity",
            "This chat's saved model route needs to be repaired before Auto-route can continue.",
        ));
    }
    let providers = agent_manager
        .select_provider_configs_metadata()
        .map_err(|error| {
            repair_error(
                "auto_route_provider_store_unavailable",
                "auto_route_provider_identity",
                error.to_string(),
            )
        })?;
    let provider = providers
        .iter()
        .find(|provider| provider.id == config_id)
        .ok_or_else(|| {
            repair_error(
                "auto_route_provider_configuration_missing",
                "auto_route_provider_identity",
                "The selected on-device model configuration is no longer available.",
            )
        })?;
    if provider.provider_id != provider_type {
        return Err(repair_error(
            "auto_route_provider_identity_mismatch",
            "auto_route_provider_identity",
            "The selected on-device model configuration changed. Choose it again to continue.",
        ));
    }
    if !super::super::db::auto_route_validation::is_local_provider(&provider.provider_id) {
        return Err(repair_error(
            "auto_route_provider_not_local",
            "auto_route_provider_identity",
            "Auto-route needs an on-device model configuration.",
        ));
    }
    if !super::super::db::auto_route_validation::provider_supports_model(provider, model_id) {
        return Err(repair_error(
            "auto_route_provider_model_mismatch",
            "auto_route_provider_identity",
            "The selected on-device model no longer belongs to this configuration.",
        ));
    }
    Ok(())
}

pub(super) fn verified_turn_provider_identity_locked(
    policy: &crate::db::AutoRouteTurnPolicyRecord,
    agent_manager: &AgentManager,
) -> Result<(), InferenceError> {
    if policy.route_generation <= 0 {
        return Err(repair_error(
            "auto_route_route_generation_unverified",
            "auto_route_provider_identity",
            "This turn's saved model route could not be verified.",
        ));
    }
    let providers = agent_manager
        .select_provider_configs_metadata_locked()
        .map_err(|error| {
            repair_error(
                "auto_route_provider_store_unavailable",
                "auto_route_provider_identity",
                error.to_string(),
            )
        })?;
    let provider = providers
        .iter()
        .find(|provider| provider.id == policy.local_provider_id)
        .ok_or_else(|| {
            repair_error(
                "auto_route_provider_configuration_missing",
                "auto_route_provider_identity",
                "This turn's on-device model configuration is no longer available.",
            )
        })?;
    if provider.provider_id != policy.local_provider_type {
        return Err(repair_error(
            "auto_route_provider_identity_mismatch",
            "auto_route_provider_identity",
            "This turn's on-device model configuration changed before it could run.",
        ));
    }
    if !crate::db::auto_route_validation::is_local_provider(&provider.provider_id) {
        return Err(repair_error(
            "auto_route_provider_not_local",
            "auto_route_provider_identity",
            "Auto-route needs an on-device model configuration.",
        ));
    }
    if !crate::db::auto_route_validation::provider_supports_model(provider, &policy.local_model_id)
    {
        return Err(repair_error(
            "auto_route_provider_model_mismatch",
            "auto_route_provider_identity",
            "This turn's on-device model no longer belongs to its saved configuration.",
        ));
    }
    Ok(())
}

fn readiness_snapshot(
    session_id: &str,
    app: &tauri::AppHandle,
    persistence: &PersistenceEngine,
    agent_manager: &AgentManager,
    gemma: &GemmaService,
) -> AutoRouteSessionReadiness {
    let session_id = session_id.trim();
    let policy = match persistence.select_chat_session_route_policy(session_id) {
        Ok(policy) => policy,
        Err(_) => {
            return unavailable_snapshot(
                session_id,
                gemma,
                "recovering",
                "auto_route_policy_persistence_unavailable",
                "auto_route_policy_persistence",
            )
        }
    };
    let Some(policy) = policy else {
        return unavailable_snapshot(
            session_id,
            gemma,
            "degraded",
            "auto_route_session_missing",
            "auto_route_session_baseline",
        );
    };
    build_readiness(app, persistence, agent_manager, gemma, policy)
}

fn build_readiness(
    app: &tauri::AppHandle,
    persistence: &PersistenceEngine,
    agent_manager: &AgentManager,
    gemma: &GemmaService,
    policy: ChatSessionRoutePolicyRecord,
) -> AutoRouteSessionReadiness {
    let health = gemma.classifier_health();
    let model_root = settings::resolved_local_model_directory(app).ok();
    let persisted_startup_assignment = model_root.as_deref().and_then(|root| {
        settings::resolved_startup_model_preference(app)
            .ok()
            .and_then(|preference| {
                crate::gemma::resolve_verified_startup_model_assignment(root, &preference).ok()
            })
    });
    let dynamic_binding_valid = policy.session_provider_id.eq_ignore_ascii_case("dynamic")
        && policy.session_model_id.eq_ignore_ascii_case("dynamic")
        && policy.dynamic_routing_override != Some(false);
    let context_budget_valid = policy.context_budget.is_some_and(|value| value > 0);
    let source_valid = matches!(
        policy.local_source.as_deref(),
        Some(
            "explicit_session" | "agent_assignment" | "startup_default" | "verified_legacy_repair"
        )
    );
    let provider_identity_failure = readiness_provider_identity_failure(agent_manager, &policy);
    let local_model_ready = if source_valid && provider_identity_failure.is_none() {
        policy
            .local_model_id
            .as_deref()
            .and_then(|model_id| model_root.clone().map(|root| (root, model_id)))
            .is_some_and(|(root, model_id)| resolve_auto_route_local_model(&root, model_id).is_ok())
    } else {
        false
    };
    let audit_ready = persistence
        .auto_route_audit_storage_ready()
        .unwrap_or(false);
    let cloud_target_ready = dynamic_routing::configured_cloud_route_snapshot(agent_manager)
        .ok()
        .flatten()
        .is_some_and(|route| route.is_runnable());
    let (recommended_local_provider_id, recommended_local_model_id) =
        if recommendation_required(provider_identity_failure, local_model_ready) {
            model_root
                .as_deref()
                .and_then(|model_root| {
                    recommended_local_model(
                        model_root,
                        agent_manager,
                        &policy.agent_id,
                        persisted_startup_assignment.as_ref(),
                    )
                })
                .and_then(|model_id| {
                    unique_local_provider_for_model(agent_manager, &model_id)
                        .map(|provider_id| (Some(provider_id), Some(model_id)))
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
        };
    let classifier_assignment_matches = persisted_startup_assignment
        .as_ref()
        .is_some_and(|assignment| health.matches_startup_assignment(assignment));
    let classifier_ready = health.is_ready() && classifier_assignment_matches;
    let (status, failure_code, failure_boundary) = readiness_failure(
        &policy,
        dynamic_binding_valid,
        provider_identity_failure,
        context_budget_valid,
        local_model_ready,
        classifier_ready,
        classifier_assignment_matches,
        audit_ready,
        &health.status,
    );
    let readiness = AutoRouteSessionReadiness {
        status: status.to_string(),
        session_id: policy.session_id,
        dynamic_binding_valid,
        classifier_model_id: health.classifier_model_id,
        classifier_ready,
        local_provider_id: policy.local_provider_id,
        local_provider_type: policy.local_provider_type,
        local_model_id: policy.local_model_id,
        route_generation: policy.route_generation,
        local_model_ready,
        recommended_local_provider_id,
        recommended_local_model_id,
        context_budget_valid,
        cloud_target_required: false,
        cloud_target_ready,
        storage_ready: true,
        audit_ready,
        readiness_generation: health.readiness_generation,
        last_verified_at_ms: health.last_verified_at_ms,
        failure_code: failure_code.map(str::to_string),
        failure_boundary: failure_boundary.map(str::to_string),
    };
    emit_readiness_receipt(&readiness, policy.local_source.as_deref());
    readiness
}

#[cfg(test)]
fn recommended_model_coordinates<'root, 'assignment>(
    model_root: &'root Path,
    assignment: &'assignment StartupModelAssignment,
) -> (&'root Path, &'assignment str) {
    (model_root, assignment.identity.canonical_id())
}

fn recommended_local_model(
    model_root: &Path,
    agent_manager: &AgentManager,
    agent_id: &str,
    startup_assignment: Option<&StartupModelAssignment>,
) -> Option<String> {
    let agent_resolution = agent_manager
        .resolve_local_model_assignment_for_agent(agent_id, model_root)
        .ok()?;
    let model_id = recommendation_candidate(&agent_resolution, startup_assignment)?;
    resolve_auto_route_local_model(model_root, model_id)
        .ok()
        .map(|model| model.id)
}

fn recommendation_candidate<'a>(
    agent_resolution: &'a crate::gemma::LegacyIdentityResolution,
    startup_assignment: Option<&'a StartupModelAssignment>,
) -> Option<&'a str> {
    match agent_resolution {
        crate::gemma::LegacyIdentityResolution::Unique(identity) => {
            Some(identity.canonical_id.as_str())
        }
        crate::gemma::LegacyIdentityResolution::Ambiguous => None,
        crate::gemma::LegacyIdentityResolution::Unavailable => {
            startup_assignment.map(|assignment| assignment.identity.canonical_id())
        }
    }
}

fn recommendation_required(
    provider_identity_failure: Option<&str>,
    local_model_ready: bool,
) -> bool {
    provider_identity_failure.is_some() || !local_model_ready
}

fn resolve_auto_route_local_model(
    model_root: &Path,
    model_id: &str,
) -> Result<LocalModelOption, GemmaError> {
    resolve_canonical_ready_local_model(model_root, model_id)
}

fn readiness_provider_identity_failure(
    agent_manager: &AgentManager,
    policy: &ChatSessionRoutePolicyRecord,
) -> Option<&'static str> {
    if policy.route_generation <= 0 {
        return Some("auto_route_route_generation_unverified");
    }
    let Some(config_id) = policy.local_provider_id.as_deref().map(str::trim) else {
        return Some("auto_route_provider_configuration_missing");
    };
    let Some(provider_type) = policy.local_provider_type.as_deref().map(str::trim) else {
        return Some("auto_route_provider_identity_mismatch");
    };
    let Some(model_id) = policy.local_model_id.as_deref().map(str::trim) else {
        return Some("auto_route_model_identity_invalid");
    };
    if config_id.is_empty() {
        return Some("auto_route_provider_configuration_missing");
    }
    if provider_type.is_empty() {
        return Some("auto_route_provider_identity_mismatch");
    }
    if model_id.is_empty() {
        return Some("auto_route_model_identity_invalid");
    }
    let providers = match agent_manager.select_provider_configs_metadata() {
        Ok(providers) => providers,
        Err(_) => return Some("auto_route_provider_store_unavailable"),
    };
    let Some(provider) = providers.iter().find(|provider| provider.id == config_id) else {
        return Some("auto_route_provider_configuration_missing");
    };
    if provider.provider_id != provider_type {
        return Some("auto_route_provider_identity_mismatch");
    }
    if !crate::db::auto_route_validation::is_local_provider(&provider.provider_id) {
        return Some("auto_route_provider_not_local");
    }
    if !crate::db::auto_route_validation::provider_supports_model(provider, model_id) {
        return Some("auto_route_provider_model_mismatch");
    }
    None
}

fn unique_local_provider_for_model(agent_manager: &AgentManager, model_id: &str) -> Option<String> {
    let providers = agent_manager.select_provider_configs_metadata().ok()?;
    let mut matches = providers.iter().filter(|provider| {
        crate::db::auto_route_validation::is_local_provider(&provider.provider_id)
            && crate::db::auto_route_validation::provider_supports_model(provider, model_id)
    });
    let provider = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(provider.id.clone())
}

fn emit_readiness_receipt(readiness: &AutoRouteSessionReadiness, provenance: Option<&str>) {
    crate::diagnostic_output::write_diagnostic_line(format_args!(
        "OOMU_NATIVE_RECEIPT {}",
        serde_json::json!({
            "kind": "auto_route_readiness",
            "receiptId": format!(
                "auto-route-readiness-{}-{}-{}",
                readiness.session_id,
                readiness.route_generation,
                readiness.status
            ),
            "sessionId": readiness.session_id,
            "providerConfigId": readiness.local_provider_id,
            "providerType": readiness.local_provider_type,
            "modelId": readiness.local_model_id,
            "provenance": provenance,
            "routeGeneration": readiness.route_generation,
            "status": readiness.status,
            "errorCode": readiness.failure_code,
            "committed": readiness.status == "ready",
            "rolledBack": false,
            "retryable": readiness.status == "loading" || readiness.status == "recovering",
        })
    ));
}

fn readiness_failure<'a>(
    policy: &'a ChatSessionRoutePolicyRecord,
    dynamic_binding_valid: bool,
    provider_identity_failure: Option<&'static str>,
    context_budget_valid: bool,
    local_model_ready: bool,
    classifier_ready: bool,
    classifier_assignment_matches: bool,
    audit_ready: bool,
    classifier_status: &AutoRouteClassifierStatus,
) -> (&'static str, Option<&'static str>, Option<&'static str>) {
    if !dynamic_binding_valid {
        return (
            "degraded",
            Some("auto_route_session_binding_invalid"),
            Some("auto_route_session_baseline"),
        );
    }
    if let Some(code) = provider_identity_failure {
        return (
            if code == "auto_route_provider_store_unavailable" {
                "recovering"
            } else {
                "degraded"
            },
            Some(code),
            Some("auto_route_provider_identity"),
        );
    }
    if policy.local_source.as_deref() == Some("needs_user_choice") {
        return (
            "degraded",
            Some("auto_route_session_baseline_choice_required"),
            Some("auto_route_session_baseline"),
        );
    }
    if policy.local_source.as_deref() == Some("legacy_unverified") {
        return (
            "degraded",
            Some("auto_route_session_baseline_unverified"),
            Some("auto_route_session_baseline"),
        );
    }
    if !local_model_ready {
        return (
            "degraded",
            Some("auto_route_session_local_model_unavailable"),
            Some("local_model_store"),
        );
    }
    if health_is_ready(classifier_status) && !classifier_assignment_matches {
        return (
            "recovering",
            Some("auto_route_classifier_assignment_changed"),
            Some("auto_route_classifier_assignment"),
        );
    }
    if !context_budget_valid {
        return (
            "degraded",
            Some("auto_route_session_context_invalid"),
            Some("auto_route_session_baseline"),
        );
    }
    if !classifier_ready {
        let status = match classifier_status {
            AutoRouteClassifierStatus::Loading => "loading",
            AutoRouteClassifierStatus::Recovering => "recovering",
            AutoRouteClassifierStatus::Shutdown => "shutdown",
            AutoRouteClassifierStatus::Ready | AutoRouteClassifierStatus::Degraded => "degraded",
        };
        return (
            status,
            Some("auto_route_classifier_not_ready"),
            Some("auto_route_classifier_readiness"),
        );
    }
    if !audit_ready {
        return (
            "recovering",
            Some("auto_route_audit_persistence_unavailable"),
            Some("auto_route_audit_persistence"),
        );
    }
    ("ready", None, None)
}

fn health_is_ready(status: &AutoRouteClassifierStatus) -> bool {
    *status == AutoRouteClassifierStatus::Ready
}

fn unavailable_snapshot(
    session_id: &str,
    gemma: &GemmaService,
    status: &str,
    failure_code: &str,
    failure_boundary: &str,
) -> AutoRouteSessionReadiness {
    let health = gemma.classifier_health();
    let classifier_ready = health.is_ready();
    AutoRouteSessionReadiness {
        status: status.to_string(),
        session_id: session_id.to_string(),
        dynamic_binding_valid: false,
        classifier_model_id: health.classifier_model_id,
        classifier_ready,
        local_provider_id: None,
        local_provider_type: None,
        local_model_id: None,
        route_generation: 0,
        local_model_ready: false,
        recommended_local_provider_id: None,
        recommended_local_model_id: None,
        context_budget_valid: false,
        cloud_target_required: false,
        cloud_target_ready: false,
        storage_ready: false,
        audit_ready: false,
        readiness_generation: health.readiness_generation,
        last_verified_at_ms: None,
        failure_code: Some(failure_code.to_string()),
        failure_boundary: Some(failure_boundary.to_string()),
    }
}

fn is_local_provider(provider_id: &str) -> bool {
    matches!(
        provider_id
            .trim()
            .replace('-', "_")
            .to_ascii_lowercase()
            .as_str(),
        "local" | "local_model" | "local_gemma" | "gemma"
    )
}

fn repair_error(
    code: impl Into<String>,
    boundary: impl Into<String>,
    message: impl Into<String>,
) -> InferenceError {
    InferenceError {
        code: code.into(),
        boundary: boundary.into(),
        message: message.into(),
    }
}

pub(super) fn verified_local_source(
    snapshot: &SessionRouteSnapshot,
) -> Result<&str, InferenceError> {
    match snapshot.local_source.as_deref() {
        Some(
            source @ ("explicit_session"
            | "agent_assignment"
            | "startup_default"
            | "verified_legacy_repair"),
        ) => Ok(source),
        Some("needs_user_choice") => Err(repair_error(
            "auto_route_session_baseline_choice_required",
            "active_session_configs",
            "This chat needs an on-device model choice before Auto-route can continue. Nothing was sent.",
        )),
        Some("legacy_unverified") | None => Err(repair_error(
            "auto_route_session_baseline_unverified",
            "active_session_configs",
            "OOMU could not safely verify this chat's saved on-device model. Choose a model to continue; nothing was sent.",
        )),
        Some(_) => Err(repair_error(
            "auto_route_session_baseline_source_invalid",
            "active_session_configs",
            "OOMU could not verify this chat's saved on-device model. Choose a model to continue; nothing was sent.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gemma::{
        LegacyIdentityResolution, LocalModelIdentity, LocalModelIdentitySource,
        StartupModelSelectionSource, GEMMA_E2B_CANONICAL_ID, GEMMA_E4B_CANONICAL_ID,
    };
    use std::path::PathBuf;

    #[test]
    fn readiness_rejects_unresolved_provider_identity() {
        let root = std::env::temp_dir().join(format!(
            "oomu-readiness-provider-identity-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ns_u128()
        ));
        let manager = AgentManager::initialize_at(root.join("ops.sqlite"))
            .expect("provider metadata store opens");
        let snapshot = SessionRouteSnapshot {
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            dynamic_routing_override: Some(true),
            local_provider_id: None,
            local_provider_type: Some("local_model".to_string()),
            local_model_id: Some(GEMMA_E2B_CANONICAL_ID.to_string()),
            local_reasoning: Some("medium".to_string()),
            local_context_budget: Some(12_288),
            local_source: Some("explicit_session".to_string()),
            route_generation: 1,
        };

        let error = verified_provider_identity(&snapshot, &manager)
            .expect_err("an unresolved provider configuration must fail closed");

        assert_eq!(error.code, "auto_route_provider_configuration_missing");
        drop(manager);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn root_level_store_is_valid_for_readiness_repair_and_recommendation() {
        let assets =
            PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join("../assets/models");
        let installed = assets.join(GEMMA_E2B_CANONICAL_ID);
        if !installed.is_dir() {
            return;
        }
        let parent = std::env::temp_dir().join(format!(
            "oomu-auto-route-root-model-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let model_root = parent.join("models");
        std::fs::create_dir_all(&parent).expect("create root-level model parent");
        std::os::unix::fs::symlink(&installed, &model_root)
            .expect("link real E2B as the root-level model store");

        let model = resolve_auto_route_local_model(&model_root, GEMMA_E2B_CANONICAL_ID)
            .expect("strictly resolve root-level E2B for every Auto-route recovery path");

        assert_eq!(model.id, GEMMA_E2B_CANONICAL_ID);
        assert_eq!(PathBuf::from(model.path), model_root);
        let _ = std::fs::remove_file(&model_root);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn recommended_model_lookup_uses_store_root_and_canonical_identity() {
        let model_root = PathBuf::from("/private/test/models");
        let model_directory = model_root.join(GEMMA_E2B_CANONICAL_ID);
        let assignment = StartupModelAssignment {
            requested_model_id: GEMMA_E2B_CANONICAL_ID.to_string(),
            resolved_model_id: GEMMA_E2B_CANONICAL_ID.to_string(),
            resolved_directory: model_directory.clone(),
            selection_source: StartupModelSelectionSource::CleanDefault,
            identity: LocalModelIdentity {
                canonical_id: GEMMA_E2B_CANONICAL_ID.to_string(),
                display_name: "Gemma 4 E2B".to_string(),
                storage_directory: model_directory.clone(),
                source: LocalModelIdentitySource::CanonicalRegistry,
            },
        };

        let (lookup_root, lookup_id) = recommended_model_coordinates(&model_root, &assignment);

        assert_eq!(lookup_root, model_root);
        assert_eq!(lookup_id, GEMMA_E2B_CANONICAL_ID);
        assert_eq!(lookup_root.join(lookup_id), model_directory);
        assert_ne!(
            assignment.identity.storage_directory.join(lookup_id),
            model_directory
        );
    }

    #[test]
    fn verified_agent_assignment_precedes_a_different_startup_default() {
        let startup = assignment(GEMMA_E4B_CANONICAL_ID);
        let agent = LegacyIdentityResolution::Unique(LocalModelIdentity {
            canonical_id: GEMMA_E2B_CANONICAL_ID.to_string(),
            display_name: "Gemma 4 E2B".to_string(),
            storage_directory: PathBuf::from("/private/test/e2b"),
            source: LocalModelIdentitySource::CanonicalRegistry,
        });

        assert_eq!(
            recommendation_candidate(&agent, Some(&startup)),
            Some(GEMMA_E2B_CANONICAL_ID)
        );
    }

    #[test]
    fn healthy_session_skips_repair_recommendation_discovery() {
        assert!(!recommendation_required(None, true));
        assert!(recommendation_required(
            Some("auto_route_provider_configuration_missing"),
            true
        ));
        assert!(recommendation_required(None, false));
    }

    #[test]
    fn ambiguous_agent_assignment_requires_a_real_choice() {
        let startup = assignment(GEMMA_E4B_CANONICAL_ID);
        assert_eq!(
            recommendation_candidate(&LegacyIdentityResolution::Ambiguous, Some(&startup)),
            None
        );
    }

    fn assignment(model_id: &str) -> StartupModelAssignment {
        let directory = PathBuf::from("/private/test/models").join(model_id);
        StartupModelAssignment {
            requested_model_id: model_id.to_string(),
            resolved_model_id: model_id.to_string(),
            resolved_directory: directory.clone(),
            selection_source: StartupModelSelectionSource::CleanDefault,
            identity: LocalModelIdentity {
                canonical_id: model_id.to_string(),
                display_name: model_id.to_string(),
                storage_directory: directory,
                source: LocalModelIdentitySource::CanonicalRegistry,
            },
        }
    }
}
