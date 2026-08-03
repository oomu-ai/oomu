mod commands;
mod deletion;
#[cfg(test)]
mod deletion_tests;
pub(crate) mod path_scope;
mod policy;
pub(crate) mod repository;
#[cfg(test)]
mod reserved_project_tests;
pub(crate) mod terminal_scope;

pub use commands::*;
pub use policy::{
    evaluate_project_policy, evaluate_project_provider_for_session, ProjectProviderPolicyResult,
    ProjectTransmissionRequest, ProjectTransmissionResult,
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectDataPolicy {
    LocalOnly,
    AskBeforeCloud,
    AllowConfiguredCloud,
}

impl ProjectDataPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::LocalOnly => "local_only",
            Self::AskBeforeCloud => "ask_before_cloud",
            Self::AllowConfiguredCloud => "allow_configured_cloud",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateProjectRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_policy")]
    pub data_policy: ProjectDataPolicy,
}

fn default_policy() -> ProjectDataPolicy {
    ProjectDataPolicy::AskBeforeCloud
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateProjectRequest {
    pub project_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectIdRequest {
    pub project_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteProjectRequest {
    pub project_id: String,
    #[serde(default)]
    pub permanently_remove_project_record: bool,
    #[serde(default)]
    pub detach_dependents: bool,
    #[serde(default)]
    pub delete_project_files: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachProjectSourceRequest {
    pub project_id: String,
    pub path: String,
    pub grant_reference: String,
    pub source_kind: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectSourceRequest {
    pub project_id: String,
    pub source_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetProjectInstructionsRequest {
    pub project_id: String,
    pub instructions: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetProjectPolicyRequest {
    pub project_id: String,
    pub data_policy: ProjectDataPolicy,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindProjectRecordRequest {
    pub project_id: Option<String>,
    pub record_kind: String,
    pub record_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub data_policy: ProjectDataPolicy,
    pub instructions: String,
    pub archived_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub source_count: usize,
    pub conversation_count: usize,
    pub workflow_count: usize,
    pub task_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSourceRecord {
    pub source_id: String,
    pub project_id: String,
    pub source_kind: String,
    pub canonical_path: String,
    pub grant_state: String,
    pub indexing_state: String,
    pub file_count: usize,
    pub last_indexed_at_ms: Option<i64>,
    pub failure_code: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDeletionPreview {
    pub project_id: String,
    pub conversations_to_detach: usize,
    pub workflows_to_detach: usize,
    pub schedules_to_detach: usize,
    pub task_runs_to_detach: usize,
    pub sources_to_remove: usize,
    pub user_files_to_delete: usize,
    pub default_action: String,
}
