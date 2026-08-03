mod commands;
mod repository;

pub use commands::*;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunAnalysisRequest {
    pub project_id: String,
    pub task_run_id: String,
    pub source_id: String,
    pub relative_path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListAnalysisRequest {
    pub task_run_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisView {
    pub analysis_id: String,
    pub project_id: String,
    pub task_run_id: String,
    pub file_name: String,
    pub answer: String,
    pub table: Value,
    pub chart: Value,
    pub method: Value,
    pub input_sha256: String,
    pub output_sha256: String,
    pub environment_sha256: String,
    pub completed_at_ms: i64,
}
