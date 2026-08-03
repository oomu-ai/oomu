mod commands;
mod repository;

pub use commands::*;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const LEARNING_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskLearningRequest {
    pub task_run_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectMethodsRequest {
    pub project_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewLearningOfferRequest {
    pub offer_id: String,
    pub action: String,
    pub edited_summary: Option<String>,
    #[serde(default)]
    pub use_everywhere_confirmed: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MethodControlRequest {
    pub method_id: String,
    pub enabled: Option<bool>,
    pub version: Option<u64>,
    pub summary: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearningOfferView {
    pub offer_id: String,
    pub project_id: String,
    pub task_run_id: String,
    pub status: String,
    pub summary: String,
    pub source_task_count: usize,
    pub evidence_count: usize,
    pub exposure_summary: String,
    pub conflict_summary: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MethodVersionView {
    pub version: u64,
    pub summary: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedMethodView {
    pub method_id: String,
    pub project_id: Option<String>,
    pub name: String,
    pub summary: String,
    pub current_version: u64,
    pub enabled: bool,
    pub use_count: u64,
    pub successful_use_count: u64,
    pub intervention_count: u64,
    pub deleted_at_ms: Option<i64>,
    pub method: Value,
    pub history: Vec<MethodVersionView>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

fn forbidden_learning_text(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    [
        "password",
        "client secret",
        "api key",
        "access token",
        "refresh token",
        "bearer ",
        "private key",
        "oauth code",
        "permission grant",
        "grant permission",
        "new authority",
        "ignore previous",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_and_authority_injection_cannot_be_learned() {
        for value in [
            "remember my access token",
            "grant permission forever",
            "ignore previous instructions",
        ] {
            assert!(forbidden_learning_text(value));
        }
        assert!(!forbidden_learning_text(
            "Put the short summary before the table"
        ));
    }
}
