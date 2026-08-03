mod commands;
mod policy;
mod repository;
mod sources;
mod worker;

pub use commands::*;

use crate::p0_contracts::ChildRunId;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc, Mutex},
};

pub const DELEGATION_SCHEMA_VERSION: u16 = 1;
pub const MAX_PARALLEL_CHILDREN: usize = 8;

#[derive(Clone, Default)]
pub struct DelegationRuntime {
    cancellations: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceBudget {
    pub max_input_tokens: usize,
    pub max_output_tokens: usize,
    pub max_tool_calls: usize,
    pub timeout_ms: u64,
    pub max_response_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AggregateBudget {
    pub max_input_tokens: usize,
    pub max_output_tokens: usize,
    pub max_tool_calls: usize,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DelegatedSource {
    InlineText {
        label: String,
        content: String,
    },
    ProjectFile {
        source_id: String,
        relative_path: String,
    },
    WebSearch {
        query: String,
        max_results: Option<usize>,
        authorization: DelegatedWebSearchAuthorization,
    },
    BrowserSnapshot {
        session_id: String,
    },
    TaskEvidence {
        event_types: Vec<String>,
    },
}

/// Immutable, persisted evidence that a delegated network read came from an
/// explicit user request and is bound to one exact query. A task identifier by
/// itself is never search authorization.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DelegatedWebSearchAuthorization {
    pub originating_user_objective: String,
    pub approved_query: String,
}

impl DelegatedSource {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::InlineText { .. } => "inline_text",
            Self::ProjectFile { .. } => "project_file",
            Self::WebSearch { .. } => "web_search",
            Self::BrowserSnapshot { .. } => "browser_snapshot",
            Self::TaskEvidence { .. } => "task_evidence",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildProposal {
    pub goal: String,
    pub expected_output_schema: String,
    pub sources: Vec<DelegatedSource>,
    pub allowed_read_tools: Vec<String>,
    pub model_route: String,
    pub budget: ResourceBudget,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateDelegationPlanRequest {
    pub schema_version: u16,
    pub project_id: String,
    pub task_run_id: String,
    pub parent_session_id: Option<String>,
    pub parent_model_route: String,
    pub parent_depth: u8,
    pub aggregate_budget: AggregateBudget,
    pub children: Vec<ChildProposal>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DelegationPlanRequest {
    pub plan_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildControlRequest {
    pub plan_id: String,
    pub child_run_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SuggestionReviewRequest {
    pub plan_id: String,
    pub suggestion_id: String,
    pub accept: bool,
    pub rejection_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSuggestionView {
    pub suggestion_id: String,
    pub child_run_id: String,
    pub kind: String,
    pub summary: String,
    pub state: String,
    pub rejection_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationPlanView {
    pub plan_id: String,
    pub project_id: String,
    pub task_run_id: String,
    pub parent_model_route: String,
    pub state: String,
    pub aggregate_budget: AggregateBudget,
    pub synthesis: Option<DelegationSynthesis>,
    pub children: Vec<ChildRunView>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildRunView {
    pub child_run_id: String,
    pub goal: String,
    pub source_scope: Vec<String>,
    pub allowed_read_tools: Vec<String>,
    pub model_route: String,
    pub budget: ResourceBudget,
    pub state: String,
    pub progress_summary: String,
    pub result: Option<ChildResult>,
    pub error_code: Option<String>,
    pub attempt: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildResult {
    pub findings: Vec<Finding>,
    pub sources: Vec<SourceEvidence>,
    pub uncertainties: Vec<String>,
    pub limitations: Vec<String>,
    pub complete: bool,
    pub actual_model_route: String,
    pub elapsed_ms: u64,
    pub input_tokens_estimate: usize,
    pub output_tokens_estimate: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Finding {
    pub statement: String,
    pub source_refs: Vec<String>,
    pub confidence: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceEvidence {
    pub source_ref: String,
    pub source_kind: String,
    pub digest: String,
    pub observed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DelegationSynthesis {
    pub findings: Vec<Finding>,
    pub uncertainties: Vec<String>,
    pub incomplete_child_run_ids: Vec<String>,
    pub ready_for_parent_synthesis: bool,
}

pub(crate) fn child_id() -> String {
    ChildRunId::new().to_string()
}

pub(crate) fn validate_summary_template(source: &str) -> Result<(), String> {
    let proposal = ChildProposal {
        goal: "Produce a grounded summary from the explicitly delegated source.".to_string(),
        expected_output_schema: "findings_sources_uncertainties_v1".to_string(),
        sources: vec![DelegatedSource::InlineText {
            label: "legacy-summary-template".to_string(),
            content: source.to_string(),
        }],
        allowed_read_tools: vec!["summarize_text".to_string()],
        model_route: "local".to_string(),
        budget: ResourceBudget {
            max_input_tokens: source.len().div_ceil(4).clamp(64, 32_000),
            max_output_tokens: 2_048,
            max_tool_calls: 1,
            timeout_ms: 120_000,
            max_response_bytes: 256 * 1024,
        },
    };
    policy::validate_child_template(&proposal)
}

pub(crate) fn execute_summary_template_sync(
    gemma: &crate::gemma::GemmaService,
    goal: &str,
    source: &str,
) -> Result<crate::gemma::InferResponse, crate::gemma::GemmaError> {
    worker::grounded_inference_sync(gemma, goal, source)
}

pub(crate) fn load_plan(
    persistence: &crate::db::PersistenceEngine,
    plan_id: &str,
) -> Result<DelegationPlanView, String> {
    repository::get(persistence, plan_id)
}

#[cfg(test)]
mod authorization_contract_tests {
    use super::*;

    #[test]
    fn delegated_search_approval_survives_persisted_source_round_trip() {
        let source = DelegatedSource::WebSearch {
            query: "Rust release notes".into(),
            max_results: Some(5),
            authorization: DelegatedWebSearchAuthorization {
                originating_user_objective: "Search online for Rust release notes".into(),
                approved_query: "Rust release notes".into(),
            },
        };
        let encoded = serde_json::to_string(&source).expect("source encodes");
        let decoded: DelegatedSource = serde_json::from_str(&encoded).expect("source decodes");
        assert_eq!(decoded, source);
    }
}
