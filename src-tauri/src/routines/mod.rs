mod authority;
#[cfg(test)]
mod authority_tests;
pub(crate) mod background;
mod commands;
pub(crate) mod control;
mod history;
pub(crate) mod parser;
pub(crate) mod repository;

pub(crate) use authority::reviewed_effect_requires_explicit_approval;
pub use authority::{
    reviewed_workflow_scope_required, verify_reviewed_workflow_scope, workflow_review_capabilities,
    WorkflowReviewCapabilities, TERMINAL_DELIVERY_NODE_ID, TERMINAL_DELIVERY_TOOL,
};
pub use background::should_keep_alive;
pub use background::BackgroundRuntimeSupervisor;
pub use commands::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposeRoutineRequest {
    pub text: String,
    pub timezone: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineProposal {
    pub schedule_expression: String,
    pub schedule_kind: String,
    pub timezone: String,
    pub normalized_summary: String,
    pub next_runs_ms: Vec<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRoutineRequest {
    pub confirmed: bool,
    pub label: String,
    pub project_id: String,
    pub workflow_id: String,
    pub workflow_version: u32,
    pub schedule_expression: String,
    pub schedule_kind: String,
    pub timezone: String,
    pub active_window_start_minute: Option<u16>,
    pub active_window_end_minute: Option<u16>,
    #[serde(default)]
    pub end_boundary: Option<RoutineEndBoundary>,
    #[serde(default)]
    pub run_once_after_create: bool,
    pub missed_run_policy: String,
    pub missed_run_cap: u8,
    #[serde(default)]
    pub task_template: Value,
    #[serde(default)]
    pub model_route: Value,
    #[serde(default)]
    pub delivery_target: Value,
    #[serde(default)]
    pub authority: Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RoutineEndBoundary {
    Midnight,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoutineIdRequest {
    pub routine_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryRoutineDeliveryRequest {
    pub routine_id: String,
    pub confirmed_absent: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteRoutineRequest {
    pub routine_id: String,
    pub confirmed: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateRoutineRequest {
    pub routine_id: String,
    pub label: String,
    pub schedule_expression: String,
    pub timezone: String,
    pub missed_run_policy: String,
    pub missed_run_cap: u8,
    #[serde(default)]
    pub delivery_target: Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GrantRoutineAuthorityRequest {
    pub routine_id: String,
    pub action_name: String,
    #[serde(default)]
    pub arguments: Value,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineRecord {
    pub routine_id: String,
    pub label: String,
    pub project_id: Option<String>,
    pub workflow_id: String,
    pub workflow_version: Option<u32>,
    pub schedule_expression: String,
    pub schedule_kind: String,
    pub timezone: String,
    pub is_active: bool,
    pub next_run_at_ms: Option<i64>,
    pub next_runs_ms: Vec<i64>,
    pub missed_run_policy: String,
    pub consecutive_failures: u32,
    pub failure_threshold: u32,
    pub paused_reason: Option<String>,
    pub delivery_target: Value,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub delivery_state: Option<String>,
    pub delivery_error_code: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundServiceStatus {
    pub user_enabled: bool,
    pub verified_active: bool,
    pub state: String,
    pub registration_state: String,
    pub registration_backend: String,
    pub process_state: String,
    pub registration_generation: Option<String>,
    pub process_id: Option<i64>,
    pub build_number: i64,
    pub build_identity: String,
    pub profile_class: String,
    pub profile_generation_sha256: String,
    pub heartbeat_at_ms: Option<i64>,
    pub heartbeat_age_ms: Option<i64>,
    pub menu_visible: bool,
    pub error_code: Option<String>,
    pub detail: String,
    pub checked_at_ms: i64,
    pub recent_receipts: Vec<BackgroundRuntimeReceipt>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundRuntimeReceipt {
    pub receipt_id: String,
    pub kind: String,
    pub outcome: String,
    pub runtime_state: String,
    pub requested_enabled: bool,
    pub registration_generation: Option<String>,
    pub process_id: Option<i64>,
    pub build_number: i64,
    pub build_identity: String,
    pub profile_class: String,
    pub profile_generation_sha256: String,
    pub detail_code: Option<String>,
    pub subject_id_hash: Option<String>,
    pub result_digest: Option<String>,
    pub created_at_ms: i64,
}

#[cfg(test)]
mod deletion_contract_tests {
    use super::DeleteRoutineRequest;

    #[test]
    fn routine_deletion_requires_an_explicit_confirmation_field() {
        assert!(
            serde_json::from_value::<DeleteRoutineRequest>(serde_json::json!({
                "routineId": "routine_task_11111111-1111-4111-8111-111111111111"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<DeleteRoutineRequest>(serde_json::json!({
                "routineId": "routine_task_11111111-1111-4111-8111-111111111111",
                "confirmed": true,
                "unexpected": true
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<DeleteRoutineRequest>(serde_json::json!({
                "routineId": "routine_task_11111111-1111-4111-8111-111111111111",
                "confirmed": true
            }))
            .is_ok()
        );
    }
}
