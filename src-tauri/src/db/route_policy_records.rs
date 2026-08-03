use rusqlite::Row;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfigRecord {
    pub session_id: String,
    pub reasoning_depth: String,
    pub context_budget: i32,
    pub model_id: Option<String>,
    pub local_provider_config_id: Option<String>,
    pub local_provider_type: Option<String>,
    pub local_route_generation: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionRoutePolicyRecord {
    pub session_id: String,
    pub agent_id: String,
    pub session_provider_id: String,
    pub session_model_id: String,
    pub dynamic_routing_override: Option<bool>,
    pub local_provider_id: Option<String>,
    pub local_provider_type: Option<String>,
    pub local_model_id: Option<String>,
    pub reasoning_depth: Option<String>,
    pub context_budget: Option<i32>,
    pub baseline_updated_at: Option<String>,
    pub local_source: Option<String>,
    pub local_reconciled_at_ms: Option<i64>,
    pub route_generation: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutoRouteTurnPolicyRecord {
    pub local_provider_id: String,
    #[serde(default)]
    pub local_provider_type: String,
    pub local_model_id: String,
    pub local_reasoning: String,
    pub local_context_budget: i32,
    pub local_source: String,
    #[serde(default)]
    pub route_generation: i64,
    pub cloud_provider_id: Option<String>,
    pub cloud_model_id: Option<String>,
    pub cloud_provider_name: Option<String>,
    pub classifier_model_id: Option<String>,
    pub classifier_version: String,
    pub policy_version: String,
    pub frozen_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueuedAutoRouteIdentityRecord {
    pub provider_config_id: String,
    pub provider_type: String,
    pub model_id: String,
    pub reasoning: String,
    pub context_budget: i32,
    pub provenance: String,
    pub route_generation: i64,
    pub frozen_at_ms: i64,
}

pub(super) fn session_config_from_row(row: &Row<'_>) -> rusqlite::Result<SessionConfigRecord> {
    Ok(SessionConfigRecord {
        session_id: row.get(0)?,
        reasoning_depth: row.get(1)?,
        context_budget: row.get(2)?,
        model_id: row.get(3)?,
        updated_at: row.get(4)?,
        local_provider_config_id: row.get(5)?,
        local_provider_type: row.get(6)?,
        local_route_generation: row.get(7)?,
    })
}
