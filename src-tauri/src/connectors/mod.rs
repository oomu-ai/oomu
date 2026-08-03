mod adapter;
mod api;
mod auth;
mod auth_refresh;
mod commands;
mod manifest;
mod microsoft365;
mod oauth_broker;
mod oauth_callback;
mod oauth_protocol;
mod repository;
#[cfg(test)]
mod reserved_project_tests;
mod runtime;
mod setup;
mod task_tool_bridge;

pub use commands::*;
pub use setup::*;

pub(crate) fn register_task_tool() -> Result<(), String> {
    register_slack_gateway_port()?;
    task_tool_bridge::register_task_tool()
}

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn slack_gateway_credential(
    engine: &crate::db::PersistenceEngine,
    connector_id: &str,
    identity: &crate::sovereign_identity::SovereignIdentity,
) -> Result<crate::native_app_ports::SlackGatewayCredential, String> {
    if repository::account_manifest(engine, connector_id)? != "slack" {
        return Err("slack_connector_identity_mismatch".to_string());
    }
    let credential = auth::refresh_if_needed(engine, connector_id, Some(identity))?;
    if !credential.scopes.iter().any(|scope| scope == "chat:write") {
        return Err("slack_messaging_consent_required".to_string());
    }
    Ok(crate::native_app_ports::SlackGatewayCredential {
        connector_id: connector_id.to_string(),
        bot_access_token: credential.slack_bot_token()?.to_string(),
    })
}

fn slack_socket_url(
    connector_id: &str,
    identity: &crate::sovereign_identity::SovereignIdentity,
) -> Result<String, String> {
    let client_id = manifest::oauth_client_id("slack")
        .ok_or_else(|| "slack_oauth_identity_unavailable".to_string())?;
    oauth_broker::open_socket(connector_id, client_id, identity)
}

pub(crate) fn register_slack_gateway_port() -> Result<(), String> {
    crate::native_app_ports::install_slack_connector(crate::native_app_ports::SlackConnectorPort {
        resolve_credential: slack_gateway_credential,
        open_socket: slack_socket_url,
    });
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorManifest {
    pub manifest_id: String,
    pub name: String,
    pub version: u32,
    pub transport: String,
    pub auth_method: String,
    pub tools: Vec<ConnectorTool>,
    pub requested_permissions: Vec<String>,
    pub base_scopes: Vec<String>,
    pub operation_grants: Vec<ConnectorOperationGrant>,
    pub data_destinations: Vec<String>,
    pub project_eligible: bool,
    pub supported: bool,
    pub availability_reason_code: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorOperationGrant {
    pub operation: String,
    pub purpose_code: String,
    pub required_scopes: Vec<String>,
    pub access_level: String,
    pub remote_mutation: bool,
    pub admin_consent_required: bool,
    pub available: bool,
    pub unavailable_reason_code: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorTool {
    pub name: String,
    pub risk: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorAccount {
    pub connector_id: String,
    pub manifest_id: String,
    pub account_label: String,
    pub granted_scopes: Vec<String>,
    pub connection_state: String,
    pub schema_version: u32,
    pub token_expires_at_ms: Option<i64>,
    pub last_probe_at_ms: Option<i64>,
    pub last_probe_code: Option<String>,
    pub all_projects_enabled: bool,
    pub project_scope_reviewed_at_ms: Option<i64>,
    pub enabled_project_ids: Vec<String>,
    pub identity_binding_hash: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_label: Option<String>,
    pub account_id: Option<String>,
    pub account_principal: Option<String>,
    pub account_kind: Option<String>,
    pub capability_grants: Vec<ConnectorCapabilityGrant>,
    pub data_routing: Vec<String>,
    pub consent_reviewed_at_ms: Option<i64>,
    pub identity_verified_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorCapabilityGrant {
    pub capability_id: String,
    pub access_level: String,
    pub required_scopes: Vec<String>,
    pub granted: bool,
    pub admin_consent_required: bool,
    pub remote_mutation: bool,
    pub available: bool,
    pub unavailable_reason_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectorIdRequest {
    pub connector_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlackConversation {
    pub id: String,
    pub name: Option<String>,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorConnectionStatus {
    pub connector_id: String,
    pub connection_state: String,
    pub granted_scopes: Vec<String>,
    pub last_probe_at_ms: Option<i64>,
    pub last_probe_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeginOAuthRequest {
    pub manifest_id: String,
    #[serde(default)]
    pub connector_id: Option<String>,
    #[serde(default)]
    pub requested_operations: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginOAuthResponse {
    pub connector_id: String,
    pub authorization_url: String,
    pub expires_at_ms: i64,
    pub requested_scopes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetConnectorProjectScopeRequest {
    pub connector_id: String,
    pub all_projects_enabled: bool,
    pub enabled_project_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorProjectScope {
    pub connector_id: String,
    pub all_projects_enabled: bool,
    pub enabled_project_ids: Vec<String>,
    pub project_scope_reviewed_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectorOperationRequest {
    pub connector_id: String,
    pub project_id: String,
    pub task_id: Option<String>,
    #[serde(default)]
    pub task_run_id: Option<String>,
    pub operation: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorOperationResult {
    pub connector_id: String,
    pub manifest_id: String,
    pub project_id: String,
    pub task_id: Option<String>,
    pub task_run_id: Option<String>,
    pub operation: String,
    pub observed_at_ms: i64,
    pub source: ConnectorResultSource,
    pub account_binding_hash: Option<String>,
    pub tenant_binding_hash: Option<String>,
    pub partial: bool,
    pub result: Value,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorResultSource {
    pub origin: String,
    pub citation: String,
    pub freshness: String,
    pub observed_at_ms: i64,
}

#[derive(Clone, Debug)]
pub(super) struct ConnectorIdentityMetadata {
    pub tenant_id: String,
    pub tenant_label: String,
    pub account_id: String,
    pub account_principal: String,
    pub account_kind: String,
    pub identity_binding_hash: String,
    pub data_routing: Vec<String>,
    pub consent_reviewed_at_ms: i64,
    pub identity_verified_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityHealth {
    pub capability_id: String,
    pub state: String,
    pub detail: String,
    pub detail_code: Option<String>,
    pub repair_action: Option<String>,
    pub repair_action_code: Option<String>,
    pub checked_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupState {
    pub current_step: String,
    pub model_path: Option<String>,
    pub completion_channel: Option<String>,
    pub sample_project_id: Option<String>,
    pub completed_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveSetupProgressRequest {
    pub current_step: String,
    pub model_path: Option<String>,
    pub completion_channel: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunSetupSampleRequest {
    pub model_route: String,
    #[serde(default = "complete_setup_by_default")]
    pub complete_setup: bool,
}

fn complete_setup_by_default() -> bool {
    true
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupCommandError {
    pub code: String,
}

impl SetupCommandError {
    pub(crate) fn new(code: &str) -> Self {
        Self {
            code: code.to_string(),
        }
    }

    pub(crate) fn internal(error: impl std::fmt::Display) -> Self {
        eprintln!(
            "SETUP_COMMAND_FAILED {}",
            crate::redaction::redacted_log_text(&error.to_string())
        );
        Self::new("setup_internal_error")
    }

    pub(crate) fn operational(code: &str, error: impl std::fmt::Display) -> Self {
        eprintln!(
            "SETUP_COMMAND_FAILED {} {}",
            code,
            crate::redaction::redacted_log_text(&error.to_string())
        );
        Self::new(code)
    }
}
