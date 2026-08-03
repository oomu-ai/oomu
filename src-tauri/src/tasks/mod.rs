mod adapters;
mod commands;
mod effect_verification;
mod repository;

pub use commands::*;
pub use repository::reconcile_all;
pub(crate) use repository::{
    get as get_task_for_remote, record_domain_event, record_domain_event_with_sequence,
    require_agent_runtime_task, require_bound_task, task_for_connector,
};

pub(crate) fn register_runtime_bridge() -> Result<(), String> {
    fn require_bound(
        engine: &crate::db::PersistenceEngine,
        task_run_id: &str,
        project_id: &str,
    ) -> Result<(), String> {
        repository::require_bound_task(engine, task_run_id, project_id).map(|_| ())
    }
    fn require_agent_runtime(
        engine: &crate::db::PersistenceEngine,
        execution_id: &str,
    ) -> Result<crate::tools::task_runtime::AgentRuntimeTaskBinding, String> {
        let task = repository::require_agent_runtime_task(engine, execution_id)?;
        Ok(crate::tools::task_runtime::AgentRuntimeTaskBinding {
            task_id: task.task_id,
            task_run_id: task.task_run_id,
            project_id: task
                .project_id
                .ok_or_else(|| "connector_task_project_required".to_string())?,
        })
    }
    crate::tools::task_runtime::register(crate::tools::task_runtime::TaskRuntimeRegistration {
        record_event: repository::record_domain_event,
        record_event_with_sequence: repository::record_domain_event_with_sequence,
        require_bound_task: require_bound,
        require_agent_runtime_task: require_agent_runtime,
    })
}

use crate::p0_contracts::TaskState;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskFilter {
    pub project_id: Option<String>,
    pub state: Option<TaskState>,
    pub origin: Option<String>,
    pub runtime_kind: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskRunRequest {
    pub task_run_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskEventsRequest {
    pub task_run_id: String,
    #[serde(default)]
    pub after_sequence: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskEffectRequest {
    pub task_run_id: String,
    pub idempotency_key: String,
    pub effect_kind: String,
    pub result_digest: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskEffectVerificationDecision {
    DidNotHappen,
    Happened,
    StopWithoutRepeating,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveTaskEffectVerificationRequest {
    pub task_run_id: String,
    pub task_id: String,
    pub runtime_record_id: String,
    pub verification_sequence: Option<u64>,
    pub node_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub effect_kind: Option<String>,
    pub decision: TaskEffectVerificationDecision,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRunRecord {
    pub task_run_id: String,
    pub task_id: String,
    pub project_id: Option<String>,
    pub runtime_kind: String,
    pub runtime_record_id: String,
    pub state: TaskState,
    pub origin: String,
    pub correlation_id: String,
    pub summary: String,
    pub last_error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub acknowledged_at_ms: Option<i64>,
    pub recovery_state: String,
    pub effect_verification_required: bool,
    pub valid_controls: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecoveryReport {
    pub inspected: usize,
    pub reconciled: usize,
    pub lost: usize,
    pub runtime_unavailable: usize,
}
