use super::{ChatTurnRequest, ChatTurnResponse, InferenceError};
use crate::db::{QueuedAutoRouteIdentityRecord, QueuedMessageRecord};

pub(super) fn verify_frozen_auto_route_identity(
    identity: &QueuedAutoRouteIdentityRecord,
    provider_id: Option<&str>,
    model_id: Option<&str>,
) -> Result<(), InferenceError> {
    let complete = [
        identity.provider_config_id.as_str(),
        identity.provider_type.as_str(),
        identity.model_id.as_str(),
        identity.reasoning.as_str(),
        identity.provenance.as_str(),
    ]
    .iter()
    .all(|value| !value.trim().is_empty() && !value.eq_ignore_ascii_case("dynamic"));
    let request_matches = provider_id.is_some_and(|value| value == identity.provider_config_id)
        && model_id.is_some_and(|value| value == identity.model_id);
    if !complete
        || identity.context_budget <= 0
        || identity.route_generation <= 0
        || identity.frozen_at_ms <= 0
        || !request_matches
    {
        return Err(InferenceError::routing_attention(
            "auto_route_queued_identity_invalid",
            "message_queue",
            "This queued Auto-route turn's frozen model identity is incomplete or changed. Nothing was sent to a provider.",
        ));
    }
    Ok(())
}

pub(super) fn request_from_record(record: &QueuedMessageRecord) -> ChatTurnRequest {
    ChatTurnRequest {
        turn_id: record.turn_id.clone(),
        generation_token: record.generation_token.clone(),
        parent_turn_id: record.parent_turn_id.clone(),
        root_turn_id: record.root_turn_id.clone(),
        turn_kind: record.turn_kind.clone(),
        agent_id: record.agent_id.clone(),
        message: record.message.clone(),
        display_message: None,
        attachments: record.attachments.clone(),
        session_id: record.session_id.clone(),
        provider_id: record.provider_id.clone(),
        model_id: record.model_id.clone(),
        locale: None,
        requested_mod_id: None,
        stream_id: None,
        reasoning: record.reasoning.clone(),
        context: record.context.clone(),
        context_budget: None,
        steering: record.steering.clone(),
        steering_only: None,
        persist_steering_message: None,
        verified_native_execution_receipt: None,
        native_execution_receipt_id: None,
        automated_web_grounding_enabled: record.automated_web_grounding_enabled,
        dynamic_routing_override: record.dynamic_routing_override,
        queued_execution: true,
        queued_auto_route_identity: record.auto_route_identity.clone(),
        auto_route_choice: None,
        auto_route_cloud_confirmed: None,
        project_cloud_confirmed: None,
    }
}

pub(super) fn route_escalation_failure(response: &ChatTurnResponse) -> Option<String> {
    response.route_escalation.as_ref().map(|_| {
        "Queued work paused because it needs foreground approval or a native-app handoff. Open this chat to continue."
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> QueuedAutoRouteIdentityRecord {
        QueuedAutoRouteIdentityRecord {
            provider_config_id: "local-config-e4b".to_string(),
            provider_type: "local_model".to_string(),
            model_id: "gemma-4-12B-it-qat-q4_0-gguf".to_string(),
            reasoning: "medium".to_string(),
            context_budget: 16_384,
            provenance: "explicit_session".to_string(),
            route_generation: 7,
            frozen_at_ms: 1_750_000_000_000,
        }
    }

    #[test]
    fn queued_executor_accepts_the_exact_frozen_identity() {
        let identity = identity();
        assert!(verify_frozen_auto_route_identity(
            &identity,
            Some("local-config-e4b"),
            Some("gemma-4-12B-it-qat-q4_0-gguf")
        )
        .is_ok());
    }

    #[test]
    fn queued_executor_rejects_a_mutated_route_after_acceptance() {
        let identity = identity();
        assert!(verify_frozen_auto_route_identity(
            &identity,
            Some("replacement-config"),
            Some("replacement-model")
        )
        .is_err());
    }
}
