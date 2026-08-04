use super::*;
use crate::workflow_ir::{CompiledInstruction, WorkflowNodeKind};
use serde_json::json;
use std::collections::HashMap;
use std::io::Read;

#[path = "../../test_local_models.rs"]
pub(crate) mod test_local_models;

pub(super) fn installed_model_root() -> std::path::PathBuf {
    test_local_models::root()
}

pub(super) fn test_verified_auto_route_baseline(model_id: &str) -> VerifiedAutoRouteBaseline {
    VerifiedAutoRouteBaseline {
        provider_config_id: ProviderConfigurationId::try_from(
            "prov-local-auto-route-test".to_string(),
        )
        .expect("test provider configuration ID"),
        provider_type: ProviderTypeId::try_from("local_model".to_string())
            .expect("test provider type"),
        model_id: CanonicalModelId::try_from(model_id.to_string()).expect("test model ID"),
        reasoning_depth: "medium".to_string(),
        context_budget: 16_384,
        provenance: AutoRouteProvenance::ExplicitSession,
    }
}

mod agent_execution_recovery;
mod agent_execution_recovery_state;
mod agent_execution_restart;
mod auto_route_session_baseline;
mod canonical_model_authority;
mod chat;
mod chat_completion_attention;
mod chat_execution;
mod chat_gateway_grounding;
mod maintenance;
mod migration;
mod persistence;
mod recoverable_chat_deletion;
mod trust;
mod workflow;
