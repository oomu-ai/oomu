use super::{auto_route_readiness, dynamic_routing::DynamicModelRouteDecision, InferenceError};
use crate::{
    agent_manager::AgentManager,
    db::{AutoRouteTurnPolicyRecord, ChatTurnPersistenceContext, PersistenceEngine},
    gemma::GemmaService,
};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Debug, Clone)]
pub(super) struct SessionRouteSnapshot {
    pub(super) provider_id: String,
    pub(super) model_id: String,
    pub(super) dynamic_routing_override: Option<bool>,
    pub(super) local_provider_id: Option<String>,
    pub(super) local_provider_type: Option<String>,
    pub(super) local_model_id: Option<String>,
    pub(super) local_reasoning: Option<String>,
    pub(super) local_context_budget: Option<i32>,
    pub(super) local_source: Option<String>,
    pub(super) route_generation: i64,
}

pub(super) struct VerifiedSessionBaseline {
    pub(super) provider_config_id: String,
    pub(super) provider_type: String,
    pub(super) model_id: String,
    pub(super) reasoning: String,
    pub(super) context_budget: i32,
    pub(super) provenance: String,
    pub(super) route_generation: i64,
}

pub(super) fn verified_session_baseline(
    snapshot: &SessionRouteSnapshot,
    app: &tauri::AppHandle,
    gemma: &GemmaService,
    agent_manager: &AgentManager,
    requested_reasoning: &str,
    requested_context_budget: Option<i32>,
) -> Result<VerifiedSessionBaseline, InferenceError> {
    let provenance = auto_route_readiness::source(snapshot, app, gemma, agent_manager)?;
    let provider_config_id = required_snapshot_text(
        snapshot.local_provider_id.as_deref(),
        "auto_route_session_baseline_missing",
        "This Auto-route session has no saved local model. Nothing was sent to a provider; choose a local model to repair the session.",
    )?;
    let provider_type = required_snapshot_text(
        snapshot.local_provider_type.as_deref(),
        "auto_route_provider_identity_mismatch",
        "This Auto-route chat needs its on-device model configuration repaired before it can continue.",
    )?;
    if snapshot.route_generation <= 0 {
        return Err(InferenceError::routing_attention(
            "auto_route_route_generation_unverified",
            "active_session_configs",
            "This Auto-route chat needs its saved model route repaired before it can continue.",
        ));
    }
    let model_id = required_snapshot_text(
        snapshot.local_model_id.as_deref(),
        "auto_route_session_baseline_missing",
        "This Auto-route session has no saved local model. Nothing was sent to a provider; choose a local model to repair the session.",
    )?;
    let reasoning = snapshot
        .local_reasoning
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(requested_reasoning);
    let context_budget = snapshot
        .local_context_budget
        .or(requested_context_budget)
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            InferenceError::routing_attention(
                "auto_route_session_context_missing",
                "active_session_configs",
                "This Auto-route session has no valid local context budget. Nothing was sent to a provider.",
            )
        })?;
    Ok(VerifiedSessionBaseline {
        provider_config_id: provider_config_id.to_string(),
        provider_type: provider_type.to_string(),
        model_id: model_id.to_string(),
        reasoning: reasoning.to_string(),
        context_budget,
        provenance: provenance.to_string(),
        route_generation: snapshot.route_generation,
    })
}

fn required_snapshot_text<'a>(
    value: Option<&'a str>,
    code: &'static str,
    message: &'static str,
) -> Result<&'a str, InferenceError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| InferenceError::routing_attention(code, "active_session_configs", message))
}

pub(super) struct ExecutorIdentity {
    provider_config_id: String,
    provider_type: String,
    model_id: String,
    provenance: String,
    route_generation: i64,
}

pub(super) fn executor_identity(
    frozen_policy: Option<&AutoRouteTurnPolicyRecord>,
    snapshot: Option<&SessionRouteSnapshot>,
) -> Option<ExecutorIdentity> {
    frozen_policy
        .map(|policy| ExecutorIdentity {
            provider_config_id: policy.local_provider_id.clone(),
            provider_type: policy.local_provider_type.clone(),
            model_id: policy.local_model_id.clone(),
            provenance: policy.local_source.clone(),
            route_generation: policy.route_generation,
        })
        .or_else(|| {
            let snapshot = snapshot?;
            Some(ExecutorIdentity {
                provider_config_id: snapshot.local_provider_id.clone()?,
                provider_type: snapshot.local_provider_type.clone()?,
                model_id: snapshot.local_model_id.clone()?,
                provenance: snapshot.local_source.clone()?,
                route_generation: snapshot.route_generation,
            })
        })
}

pub(super) fn emit_decision_receipt(
    identity: Option<&ExecutorIdentity>,
    session_id: &str,
    turn_id: &str,
    target_provider_id: &str,
    target_model_id: &str,
) {
    emit_receipt(
        "auto_route_decision",
        identity,
        session_id,
        turn_id,
        target_provider_id,
        target_model_id,
    );
}

pub(super) fn emit_executor_receipt(
    identity: Option<&ExecutorIdentity>,
    session_id: &str,
    turn_id: &str,
    target_provider_id: &str,
    target_model_id: &str,
) {
    emit_receipt(
        "auto_route_executor",
        identity,
        session_id,
        turn_id,
        target_provider_id,
        target_model_id,
    );
}

fn emit_receipt(
    kind: &'static str,
    identity: Option<&ExecutorIdentity>,
    session_id: &str,
    turn_id: &str,
    target_provider_id: &str,
    target_model_id: &str,
) {
    let Some(identity) = identity else {
        return;
    };
    crate::diagnostic_output::write_diagnostic_line(format_args!(
        "OOMU_NATIVE_RECEIPT {}",
        serde_json::json!({
            "kind": kind,
            "receiptId": format!("{}-{}-{}", kind.replace('_', "-"), turn_id, identity.route_generation),
            "sessionId": session_id,
            "turnId": turn_id,
            "providerConfigId": identity.provider_config_id,
            "providerType": identity.provider_type,
            "modelId": identity.model_id,
            "provenance": identity.provenance,
            "routeGeneration": identity.route_generation,
            "targetProviderId": target_provider_id,
            "targetModelId": target_model_id,
            "committed": true,
            "rolledBack": false,
            "retryable": false,
        })
    ));
}

pub(super) fn persist_pending_attempt(
    identity: Option<&ExecutorIdentity>,
    route: Option<&DynamicModelRouteDecision>,
    persistence: &PersistenceEngine,
    prompt: &str,
    session_id: &str,
    turn: &ChatTurnPersistenceContext,
) -> Result<(), InferenceError> {
    let Some(route) = route else { return Ok(()) };
    emit_decision_receipt(
        identity,
        session_id,
        &turn.turn_id,
        &route.provider_id,
        &route.model_id,
    );
    let metadata = serde_json::json!({
        "eventKind": "dynamic_routing_attempt",
        "terminalState": "provider_dispatch_pending",
        "sessionId": session_id,
        "turnId": turn.turn_id,
        "rootTurnId": turn.root_turn_id,
        "routingPolicyVersion": route.policy_version,
        "routingClassifierSource": route.classifier_source.as_str(),
        "routingClassifierModelId": route.classifier_model_id.as_deref(),
        "routingReadinessGeneration": route.readiness_generation,
        "routingClassifierLatencyMs": u64::try_from(route.classifier_latency_ms).unwrap_or(u64::MAX),
        "routingRecoveryAttempted": route.recovery_attempted,
        "configuredLocalProviderId": route.local_provider_id.as_str(),
        "configuredLocalModelId": route.local_model_id.as_str(),
        "configuredLocalSource": "session_config",
        "targetProviderId": route.provider_id.as_str(),
        "targetModelId": route.model_id.as_str(),
        "targetTier": route.tier,
        "routingCapability": route.capability.as_str(),
        "routingDemand": route.demand.as_str(),
        "routingConfidence": route.confidence.as_str(),
        "explicitTurnChoice": route.classifier_source.strip_prefix("explicit_turn_choice_v1:"),
        "offDeviceConfirmed": route.classifier_source == "explicit_turn_choice_v1:cloud",
        "providerDispatchAttempted": false,
    });
    persistence.insert_dynamic_routing_audit(prompt, "", &metadata).map_err(|error| {
        InferenceError::routing_attention(
            "dynamic_routing_audit_persistence_failed",
            "encrypted_routing_audit",
            format!("Auto-route chose a model but could not save its native route evidence. Nothing was sent to a provider. {error}"),
        )
    })
}

#[derive(Clone)]
pub(super) struct FailedAttemptAudit {
    persistence: PersistenceEngine,
    prompt: String,
    route: DynamicModelRouteDecision,
    session_id: String,
    turn_id: String,
    root_turn_id: String,
    provider_dispatch_attempted: Arc<AtomicBool>,
}

impl FailedAttemptAudit {
    pub(super) fn new(
        persistence: PersistenceEngine,
        prompt: String,
        route: DynamicModelRouteDecision,
        session_id: String,
        turn_id: String,
        root_turn_id: String,
    ) -> Self {
        Self {
            persistence,
            prompt,
            route,
            session_id,
            turn_id,
            root_turn_id,
            provider_dispatch_attempted: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn mark_provider_dispatch_attempted(&self) {
        self.provider_dispatch_attempted
            .store(true, Ordering::Release);
    }

    pub(super) fn persist(&self, error: &InferenceError) {
        let result = self.persistence.insert_dynamic_routing_audit(
            &self.prompt,
            "",
            &failed_attempt_metadata(
                &self.route,
                &self.session_id,
                &self.turn_id,
                &self.root_turn_id,
                error,
                self.provider_dispatch_attempted.load(Ordering::Acquire),
            ),
        );
        if let Err(audit_error) = result {
            eprintln!(
                "DYNAMIC_ROUTING_FAILURE_AUDIT_FAILED session_id={} turn_id={} error={}",
                self.session_id,
                self.turn_id,
                crate::redaction::redacted_log_text(&audit_error.to_string())
            );
        }
    }
}

pub(super) fn failed_attempt_audits(
    persistence: &PersistenceEngine,
    prompt: &str,
    route: Option<DynamicModelRouteDecision>,
    session_id: &str,
    turn: &ChatTurnPersistenceContext,
) -> (Option<FailedAttemptAudit>, Option<FailedAttemptAudit>) {
    let audit = route.map(|route| {
        FailedAttemptAudit::new(
            persistence.clone(),
            prompt.to_string(),
            route,
            session_id.to_string(),
            turn.turn_id.clone(),
            turn.root_turn_id.clone(),
        )
    });
    (audit.clone(), audit)
}

pub(super) fn mark_provider_dispatch_attempted(audit: Option<&FailedAttemptAudit>) {
    if let Some(audit) = audit {
        audit.mark_provider_dispatch_attempted();
    }
}

pub(super) fn persist_failed_result<T, E>(
    audit: Option<&FailedAttemptAudit>,
    result: &Result<Result<T, InferenceError>, E>,
) {
    if let (Some(audit), Ok(Err(error))) = (audit, result) {
        audit.persist(error);
    }
}

fn failed_attempt_metadata(
    route: &DynamicModelRouteDecision,
    session_id: &str,
    turn_id: &str,
    root_turn_id: &str,
    error: &InferenceError,
    provider_dispatch_attempted: bool,
) -> Value {
    serde_json::json!({
        "eventKind": "dynamic_routing_attempt",
        "terminalState": if provider_dispatch_attempted { "provider_failed" } else { "pre_dispatch_failed" },
        "sessionId": session_id,
        "turnId": turn_id,
        "rootTurnId": root_turn_id,
        "routingPolicyVersion": route.policy_version,
        "routingClassifierSource": route.classifier_source.as_str(),
        "routingClassifierModelId": route.classifier_model_id.as_deref(),
        "routingReadinessGeneration": route.readiness_generation,
        "configuredLocalProviderId": route.local_provider_id.as_str(),
        "configuredLocalModelId": route.local_model_id.as_str(),
        "targetProviderId": route.provider_id.as_str(),
        "targetModelId": route.model_id.as_str(),
        "targetTier": route.tier,
        "firstFailureCode": error.code.as_str(),
        "firstFailureBoundary": error.boundary.as_str(),
        "routingErrorCode": error.code.as_str(),
        "routingErrorBoundary": error.boundary.as_str(),
        "providerDispatchAttempted": provider_dispatch_attempted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprint_304_failure_receipt_retains_the_first_boundary() {
        let route = DynamicModelRouteDecision {
            local_provider_id: "local_model".to_string(),
            local_model_id: "gemma-4-E4B-it-qat-q4_0-gguf".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "gemma-4-E4B-it-qat-q4_0-gguf".to_string(),
            matched_complexity_rules: Vec::new(),
            tier: "local_tier_1",
            reason: "receipt-backed completion".to_string(),
            classifier_source: "hydrated_public_grounding_v1".to_string(),
            capability: "general".to_string(),
            demand: "routine".to_string(),
            confidence: "confident".to_string(),
            classification_reason: "bounded_transformation".to_string(),
            classifier_latency_ms: 0,
            classifier_model_id: None,
            readiness_generation: 7,
            recovery_attempted: false,
            policy_version: "auto_route_policy_v2",
        };
        let error = InferenceError {
            code: "local_inference_unavailable".to_string(),
            boundary: "LocalInferenceWorker".to_string(),
            message: "sensitive detail must not enter the audit".to_string(),
        };
        let metadata =
            failed_attempt_metadata(&route, "session-304", "turn-304", "turn-304", &error, false);

        assert_eq!(metadata["firstFailureCode"], "local_inference_unavailable");
        assert_eq!(metadata["firstFailureBoundary"], "LocalInferenceWorker");
        assert_eq!(metadata["providerDispatchAttempted"], false);
        assert_eq!(metadata["terminalState"], "pre_dispatch_failed");
        assert!(!metadata.to_string().contains("sensitive detail"));
    }
}
