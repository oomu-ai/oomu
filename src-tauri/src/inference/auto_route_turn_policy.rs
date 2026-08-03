use super::{
    auto_route_execution,
    chat_turn_persistence::PreClaimAcceptedTurnGuard,
    chat_turn_response_claim_error,
    dynamic_routing::{self, private_apple_read::PrivateAppleReadKind},
    private_auto_route, session_snapshot_is_dynamic, AgentManager, GemmaService, InferenceError,
    PersistenceEngine, DYNAMIC_ROUTE_ID,
};
use crate::db::{AutoRouteTurnPolicyRecord, QueuedAutoRouteIdentityRecord};

pub(super) struct FreezeTurnPolicyRequest<'a> {
    pub dynamic_routing_active: bool,
    pub parent_turn_exists: bool,
    pub turn_kind: &'a str,
    pub queued_execution: bool,
    pub session_id: Option<&'a str>,
    pub session_snapshot: Option<&'a auto_route_execution::SessionRouteSnapshot>,
    pub requested_reasoning: &'a str,
    pub context_budget: Option<i32>,
    pub display_message: Option<&'a str>,
    pub message: &'a str,
    pub turn_id: &'a str,
    pub generation_token: &'a str,
    pub root_turn_id: &'a str,
    pub agent_id: &'a str,
    pub requested_provider_id: Option<&'a str>,
    pub requested_model_id: Option<&'a str>,
    pub queued_identity: Option<&'a QueuedAutoRouteIdentityRecord>,
    pub private_apple_read: Option<PrivateAppleReadKind>,
}

pub(super) struct FrozenTurnPolicyOutcome {
    pub policy: Option<AutoRouteTurnPolicyRecord>,
    pub accepted_turn_guard: Option<PreClaimAcceptedTurnGuard>,
}

pub(super) fn effective_reasoning(
    dynamic_routing_active: bool,
    parent_turn_exists: bool,
    policy: Option<&AutoRouteTurnPolicyRecord>,
    snapshot: Option<&auto_route_execution::SessionRouteSnapshot>,
    requested: &str,
) -> String {
    if !dynamic_routing_active || parent_turn_exists {
        return requested.to_string();
    }
    policy
        .map(|value| value.local_reasoning.as_str())
        .or_else(|| snapshot.and_then(|value| value.local_reasoning.as_deref()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(requested)
        .to_lowercase()
}

pub(super) async fn freeze_turn_policy(
    request: FreezeTurnPolicyRequest<'_>,
    app: &tauri::AppHandle,
    agent_manager: &AgentManager,
    persistence: &PersistenceEngine,
    gemma: &GemmaService,
) -> Result<FrozenTurnPolicyOutcome, InferenceError> {
    if !request.dynamic_routing_active || request.parent_turn_exists || request.turn_kind != "root"
    {
        return Ok(FrozenTurnPolicyOutcome {
            policy: None,
            accepted_turn_guard: None,
        });
    }
    if request.queued_execution {
        return freeze_queued_policy(request, agent_manager, gemma).map(|policy| {
            FrozenTurnPolicyOutcome {
                policy: Some(policy),
                accepted_turn_guard: None,
            }
        });
    }
    freeze_live_policy(request, app, agent_manager, persistence, gemma).await
}

fn freeze_queued_policy(
    request: FreezeTurnPolicyRequest<'_>,
    agent_manager: &AgentManager,
    gemma: &GemmaService,
) -> Result<AutoRouteTurnPolicyRecord, InferenceError> {
    let identity = request.queued_identity.ok_or_else(|| {
        InferenceError::routing_attention(
            "auto_route_queued_identity_missing",
            "message_queue",
            "This queued Auto-route turn has no frozen model identity. Nothing was sent to a provider.",
        )
    })?;
    super::queued_execution::verify_frozen_auto_route_identity(
        identity,
        request.requested_provider_id,
        request.requested_model_id,
    )?;
    let cloud =
        private_auto_route::cloud_snapshot_for_turn(agent_manager, request.private_apple_read)?;
    let classifier_health = gemma.classifier_health();
    Ok(AutoRouteTurnPolicyRecord {
        local_provider_id: identity.provider_config_id.clone(),
        local_provider_type: identity.provider_type.clone(),
        local_model_id: identity.model_id.clone(),
        local_reasoning: identity.reasoning.clone(),
        local_context_budget: identity.context_budget,
        local_source: identity.provenance.clone(),
        route_generation: identity.route_generation,
        cloud_provider_id: cloud.as_ref().map(|target| target.provider_id.clone()),
        cloud_model_id: cloud.as_ref().and_then(|target| target.model_id.clone()),
        cloud_provider_name: cloud.as_ref().map(|target| target.provider_name.clone()),
        classifier_model_id: classifier_health.classifier_model_id,
        classifier_version: dynamic_routing::SEMANTIC_CLASSIFIER_VERSION.to_string(),
        policy_version: dynamic_routing::AUTO_ROUTE_POLICY_VERSION.to_string(),
        frozen_at_ms: identity.frozen_at_ms,
    })
}

async fn freeze_live_policy(
    request: FreezeTurnPolicyRequest<'_>,
    app: &tauri::AppHandle,
    agent_manager: &AgentManager,
    persistence: &PersistenceEngine,
    gemma: &GemmaService,
) -> Result<FrozenTurnPolicyOutcome, InferenceError> {
    let active_session_id = request
        .session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            InferenceError::routing_attention(
                "auto_route_session_missing",
                "chat_turn_acceptance",
                "Auto-route requires a durable chat session before classification. Nothing was sent to a provider.",
            )
        })?
        .to_string();
    let snapshot = request
        .session_snapshot
        .filter(|snapshot| session_snapshot_is_dynamic(snapshot))
        .ok_or_else(|| {
            InferenceError::routing_attention(
                "auto_route_session_binding_invalid",
                "chat_sessions",
                "Auto-route is enabled, but the saved session binding is not dynamic/dynamic. Nothing was sent to a provider.",
            )
        })?;
    let baseline = auto_route_execution::verified_session_baseline(
        snapshot,
        app,
        gemma,
        agent_manager,
        request.requested_reasoning,
        request.context_budget,
    )?;
    let acceptance = crate::db::AcceptChatTurnRequest {
        turn_id: request.turn_id.to_string(),
        generation_token: request.generation_token.to_string(),
        parent_turn_id: None,
        root_turn_id: request.root_turn_id.to_string(),
        turn_kind: request.turn_kind.to_string(),
        session_id: active_session_id.clone(),
        agent_id: request.agent_id.to_string(),
        provider_id: DYNAMIC_ROUTE_ID.to_string(),
        model_id: DYNAMIC_ROUTE_ID.to_string(),
        message: request
            .display_message
            .unwrap_or(request.message)
            .to_string(),
    };
    let accepted_context = acceptance.persistence_context();
    let persistence_for_acceptance = persistence.clone();
    let resume_request = acceptance.clone();
    tauri::async_runtime::spawn_blocking(move || {
        match persistence_for_acceptance.accept_chat_turn(acceptance) {
            Ok(accepted) => Ok(accepted),
            Err(accept_error) => persistence_for_acceptance
                .resume_interrupted_chat_turn(resume_request)
                .map_err(|_| accept_error),
        }
    })
    .await
    .map_err(|error| InferenceError::worker(error.to_string()))?
    .map_err(chat_turn_response_claim_error)?;
    let accepted_turn_guard = PreClaimAcceptedTurnGuard::new(persistence.clone(), accepted_context);

    let cloud =
        private_auto_route::cloud_snapshot_for_turn(agent_manager, request.private_apple_read)?;
    let classifier_health = gemma.classifier_health();
    let candidate = AutoRouteTurnPolicyRecord {
        local_provider_id: baseline.provider_config_id,
        local_provider_type: baseline.provider_type,
        local_model_id: baseline.model_id,
        local_reasoning: baseline.reasoning,
        local_context_budget: baseline.context_budget,
        local_source: baseline.provenance,
        route_generation: baseline.route_generation,
        cloud_provider_id: cloud.as_ref().map(|target| target.provider_id.clone()),
        cloud_model_id: cloud.as_ref().and_then(|target| target.model_id.clone()),
        cloud_provider_name: cloud.as_ref().map(|target| target.provider_name.clone()),
        classifier_model_id: classifier_health.classifier_model_id,
        classifier_version: dynamic_routing::SEMANTIC_CLASSIFIER_VERSION.to_string(),
        policy_version: dynamic_routing::AUTO_ROUTE_POLICY_VERSION.to_string(),
        frozen_at_ms: crate::foundation::clock::unix_time_ms_i64(),
    };
    let policy = persist_policy(
        persistence,
        request.turn_id,
        request.generation_token,
        &active_session_id,
        request.agent_id,
        candidate,
    )
    .await?;
    Ok(FrozenTurnPolicyOutcome {
        policy: Some(policy),
        accepted_turn_guard: Some(accepted_turn_guard),
    })
}

async fn persist_policy(
    persistence: &PersistenceEngine,
    turn_id: &str,
    generation_token: &str,
    session_id: &str,
    agent_id: &str,
    candidate: AutoRouteTurnPolicyRecord,
) -> Result<AutoRouteTurnPolicyRecord, InferenceError> {
    let persistence = persistence.clone();
    let turn_id = turn_id.to_string();
    let generation_token = generation_token.to_string();
    let session_id = session_id.to_string();
    let agent_id = agent_id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        persistence.freeze_auto_route_turn_policy(
            &turn_id,
            &generation_token,
            &session_id,
            &agent_id,
            candidate,
        )
    })
    .await
    .map_err(|error| InferenceError::worker(error.to_string()))?
    .map_err(|error| {
        InferenceError::routing_attention(
            "auto_route_turn_policy_persistence_failed",
            "chat_turn_acceptance",
            format!(
                "Auto-route could not freeze this accepted turn's model policy. Nothing was sent to a provider. {error}"
            ),
        )
    })
}
