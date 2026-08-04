use crate::db::{ChatTurnPersistenceContext, PersistenceEngine};
use crate::foundation::clock::{unix_time_ms_u128 as unix_time_ms, unix_time_ms_u64};
use crate::mcp::local_filesystem::NativeLocalFilesystemServer;
use crate::mcp::shield::{sanitize_outgoing_payload_for_transport, McpTransportConfig};
use crate::mcp::taskflow::NativeTaskflowServer;
use crate::mcp::{bootstrap::headless_server_configs_for, client_sse};
use crate::mcp_result::McpToolCallResult;
use crate::network_policy::CanonicalDestination;
use crate::shield_gate::{self, RequestedAction, ShieldApprovalManager};
use crate::tool_security::classify_mcp_tool_call;
use chrono::{Duration as ChronoDuration, NaiveDateTime};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use public_search_session_approval::{
    validate_mcp_chat_turn, PublicSearchApprovalTurnBinding, PublicSearchChatSessionGrant,
};

const MCP_REQUEST_QUEUE_DEPTH: usize = 64;
const MCP_STDERR_LOG_DIR: &str = ".oomu/logs/mcp";
const MCP_STDERR_LOG_FIELD_LIMIT: usize = 4096;
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_CLIENT_NAME: &str = "OOMU";
const MCP_BACKEND_MANAGED_COMMAND: &str = "backend-managed";
const MCP_TOOL_APPROVAL_TTL_MS: u64 = 5 * 60 * 1000;
const MCP_TOOL_APPROVAL_TOKEN_BYTES: usize = 32;
const MCP_MAX_JSON_DEPTH: usize = 48;
const MCP_MAX_JSON_NODES: usize = 30_000;
const MCP_MAX_JSON_CONTAINER_ENTRIES: usize = 10_000;
const MCP_MAX_JSON_STRING_BYTES: usize = 64 * 1024;
const MCP_MAX_TOOL_CATALOG_SIZE: usize = 256;
const MCP_MAX_TOOL_NAME_BYTES: usize = 256;
const MCP_MAX_TOOL_DESCRIPTION_BYTES: usize = 16 * 1024;
const MCP_MAX_TOOL_SCHEMA_BYTES: usize = 256 * 1024;
const MCP_AUTHORIZATION_MESSAGE: &str =
    "MCP workspace boundary requires explicit Shield Gate approval before local tools can run.";
const MCP_VETTED_RUNTIME_ALIASES: &[&str] = &["node", "nodejs", "python", "python3", "bun"];
const MCP_DENIED_SHELL_OPERATORS: &[&str] =
    &[";", "&&", "||", "|", ">", "<", "`", "$(", "\n", "\r"];
const MACOS_APPLESCRIPT_SERVER_NAME: &str = "macos_applescript";
const READ_SYSTEM_CALENDAR_TOOL_NAME: &str = "read_system_calendar";
const READ_SYSTEM_CONTACTS_TOOL_NAME: &str = "read_system_contacts";
const READ_SYSTEM_MUSIC_TOOL_NAME: &str = "read_system_music";
const READ_SYSTEM_PHOTOS_TOOL_NAME: &str = "read_system_photos";
const SYSTEM_CALENDAR_MCP_PREPARATION_TIMEOUT_SECONDS: u64 = 15;
const SYSTEM_CALENDAR_FALLBACK_TIMEOUT_SECONDS: u64 = 30;
const READ_SYSTEM_EMAILS_TOOL_NAME: &str = "read_system_emails";
const SYSTEM_APP_READ_TOOL_NAMES: &[&str] = &[
    "read_apple_app_ui",
    "read_system_calendar",
    "read_system_contacts",
    "read_system_emails",
    "read_system_notes",
    "read_system_music",
    "read_system_photos",
    "read_system_reminders",
];
const SYSTEM_APP_MUTATING_TOOL_NAMES: &[&str] = &[
    "add_system_reminder",
    "create_system_note",
    "draft_system_email",
    "prepare_system_message",
    "capture_disposable_window",
    "preview_camera",
    "send_system_email",
    "trigger_system_notification",
];
const DEFAULT_SYSTEM_CALENDAR_HOURS_AHEAD: f64 = 24.0;
const MIN_SYSTEM_CALENDAR_HOURS_AHEAD: f64 = 0.25;
const MAX_SYSTEM_CALENDAR_HOURS_AHEAD: f64 = 720.0;
const MAX_SYSTEM_CALENDAR_NAME_CHARS: usize = 256;
const MAX_SYSTEM_CALENDAR_DATE_CHARS: usize = 64;
const MAX_SYSTEM_CALENDAR_EVENTS: usize = 200;
static MCP_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
static MCP_PROCESS_BINDING_KEY: OnceLock<[u8; 32]> = OnceLock::new();

fn is_system_calendar_tool(server_name: &str, tool_name: &str) -> bool {
    is_named_system_tool(server_name, tool_name, READ_SYSTEM_CALENDAR_TOOL_NAME)
}
fn is_system_photos_tool(server_name: &str, tool_name: &str) -> bool {
    is_named_system_tool(server_name, tool_name, READ_SYSTEM_PHOTOS_TOOL_NAME)
}
fn is_system_contacts_tool(server_name: &str, tool_name: &str) -> bool {
    is_named_system_tool(server_name, tool_name, READ_SYSTEM_CONTACTS_TOOL_NAME)
}
fn is_system_music_tool(server_name: &str, tool_name: &str) -> bool {
    is_named_system_tool(server_name, tool_name, READ_SYSTEM_MUSIC_TOOL_NAME)
}

fn is_named_system_tool(server_name: &str, tool_name: &str, expected_tool: &str) -> bool {
    server_name
        .trim()
        .eq_ignore_ascii_case(MACOS_APPLESCRIPT_SERVER_NAME)
        && tool_name.trim().eq_ignore_ascii_case(expected_tool)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeCalendarFailure {
    code: String,
    message: String,
    retryable: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub transport: McpTransportConfig,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default = "json_null")]
    pub params: serde_json::Value,
    pub id: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
    pub id: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default = "json_null")]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone)]
enum JsonRpcOutboundMessage {
    Request(JsonRpcRequest),
    Notification(JsonRpcNotification),
}

#[derive(Debug, Clone)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

#[derive(Debug, Clone, Serialize)]
pub struct McpClientError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
}

/// MCP servers reachable over the `Native` transport. Each variant
/// is a real implementation backed by the local sandbox — no child process and
/// no network. Add a variant here to ship a new built-in native server.
enum NativeServer {
    Filesystem(NativeLocalFilesystemServer),
    Taskflow(NativeTaskflowServer),
}

impl NativeServer {
    fn from_config(config: &McpServerConfig) -> Result<Self, McpClientError> {
        let server = match config.name.as_str() {
            "local_filesystem" => NativeServer::Filesystem(
                NativeLocalFilesystemServer::from_env(&config.env).map_err(|error| {
                    McpClientError::transport(format!(
                        "Failed to initialize native MCP server '{}': {error}",
                        config.name
                    ))
                })?,
            ),
            "taskflow_native" => NativeServer::Taskflow(
                NativeTaskflowServer::from_env(&config.env).map_err(|error| {
                    McpClientError::transport(format!(
                        "Failed to initialize native MCP server '{}': {error}",
                        config.name
                    ))
                })?,
            ),
            other => {
                return Err(McpClientError::transport(format!(
                    "Native MCP transport is not available for server '{other}'."
                )));
            }
        };
        Ok(server)
    }

    fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        match self {
            NativeServer::Filesystem(server) => server.handle_request(request),
            NativeServer::Taskflow(server) => server.handle_request(request),
        }
    }

    fn handle_notification(&self, notification: JsonRpcNotification) -> Result<(), String> {
        match self {
            NativeServer::Filesystem(server) => server.handle_notification(notification),
            NativeServer::Taskflow(server) => server.handle_notification(notification),
        }
    }
}

pub struct McpClientSession {
    pub server_name: String,
    config_binding: String,
    trusted_internal_config_binding: Option<String>,
    tx: Option<mpsc::Sender<JsonRpcOutboundMessage>>,
    child_handle: StdMutex<Option<Child>>,
    transport: McpTransportConfig,
    native_server: Option<NativeServer>,
    remote_client: Option<client_sse::RemoteTransportClient>,
    remote_cancellation: Arc<AtomicBool>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<JsonRpcResponse, McpClientError>>>>>,
    server_capabilities: Mutex<Option<Value>>,
    server_info: Mutex<Option<Value>>,
    server_instructions: Mutex<Option<String>>,
    tools_stale: Arc<Mutex<bool>>,
}

#[derive(Clone)]
pub struct McpClientRegistry {
    accepting_work: Arc<AtomicBool>,
    connection_lifecycle: Arc<Mutex<()>>,
    #[cfg(test)]
    remote_connect_test_hook: Arc<Mutex<Option<RemoteConnectTestHook>>>,
    #[cfg(test)]
    remote_tool_execution_test_hook: Arc<Mutex<Option<RemoteToolExecutionTestHook>>>,
    sessions: Arc<Mutex<HashMap<String, Arc<McpClientSession>>>>,
    connecting_remote: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    tool_catalog: Arc<Mutex<HashMap<String, Vec<McpTool>>>>,
    configs: Arc<Mutex<HashMap<String, McpServerConfig>>>,
    trusted_builtin_configs: Arc<Mutex<HashMap<String, McpServerConfig>>>,
    spawn_authorizations: Arc<Mutex<HashMap<String, McpSpawnAuthorization>>>,
    pending_tool_approvals: Arc<Mutex<HashMap<String, PendingMcpToolApproval>>>,
    public_search_chat_session_grants: Arc<Mutex<HashSet<PublicSearchChatSessionGrant>>>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteConnectTestPhase {
    BeforeBuild,
    DuringActivation,
}

#[cfg(test)]
#[derive(Clone)]
struct RemoteToolExecutionTestHook {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[cfg(test)]
impl RemoteToolExecutionTestHook {
    fn new() -> Self {
        Self {
            entered: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

#[cfg(test)]
#[derive(Clone)]
struct RemoteConnectTestHook {
    phase: RemoteConnectTestPhase,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[cfg(test)]
impl RemoteConnectTestHook {
    fn new(phase: RemoteConnectTestPhase) -> Self {
        Self {
            phase,
            entered: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

#[derive(Debug, Clone)]
enum McpSpawnAuthorization {
    Unapproved,
    ShieldGateApproved {
        config_binding: String,
        remote_destination: Option<CanonicalDestination>,
    },
    TrustedInternal {
        config_binding: String,
    },
}

impl McpSpawnAuthorization {
    fn shield_approved(
        config: &McpServerConfig,
        remote_destination: Option<CanonicalDestination>,
    ) -> Self {
        Self::ShieldGateApproved {
            config_binding: mcp_config_binding(config),
            remote_destination,
        }
    }

    fn trusted_internal(config: &McpServerConfig) -> Self {
        Self::TrustedInternal {
            config_binding: mcp_config_binding(config),
        }
    }

    fn is_bound_to(&self, config: &McpServerConfig) -> bool {
        let binding = mcp_config_binding(config);
        match self {
            Self::ShieldGateApproved { config_binding, .. }
            | Self::TrustedInternal { config_binding } => config_binding == &binding,
            Self::Unapproved => false,
        }
    }

    fn allows_stdio_spawn(&self, config: &McpServerConfig) -> bool {
        self.is_bound_to(config)
            && matches!(
                self,
                Self::ShieldGateApproved { .. } | Self::TrustedInternal { .. }
            )
    }

    fn allows_remote_connection(&self, config: &McpServerConfig) -> bool {
        self.is_bound_to(config)
            && matches!(
                self,
                Self::ShieldGateApproved {
                    remote_destination: Some(_),
                    ..
                }
            )
    }

    fn remote_destination(&self) -> Option<&CanonicalDestination> {
        match self {
            Self::ShieldGateApproved {
                remote_destination, ..
            } => remote_destination.as_ref(),
            Self::Unapproved | Self::TrustedInternal { .. } => None,
        }
    }
}

impl Default for McpClientRegistry {
    fn default() -> Self {
        Self {
            accepting_work: Arc::new(AtomicBool::new(true)),
            connection_lifecycle: Arc::new(Mutex::new(())),
            #[cfg(test)]
            remote_connect_test_hook: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            remote_tool_execution_test_hook: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            connecting_remote: Arc::new(Mutex::new(HashMap::new())),
            tool_catalog: Arc::new(Mutex::new(HashMap::new())),
            configs: Arc::new(Mutex::new(HashMap::new())),
            trusted_builtin_configs: Arc::new(Mutex::new(HashMap::new())),
            spawn_authorizations: Arc::new(Mutex::new(HashMap::new())),
            pending_tool_approvals: Arc::new(Mutex::new(HashMap::new())),
            public_search_chat_session_grants: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: serde_json::Value,
    #[serde(default, rename = "outputSchema")]
    pub output_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub annotations: Option<serde_json::Value>,
    #[serde(default, rename = "_meta")]
    pub meta: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub enum McpServerStatus {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpServerState {
    pub name: String,
    pub status: McpServerStatus,
    pub tools: Vec<McpTool>,
    #[serde(default, rename = "protocolVersion")]
    pub protocol_version: Option<String>,
    #[serde(default, rename = "serverInfo")]
    pub server_info: Option<serde_json::Value>,
    #[serde(default)]
    pub capabilities: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpToolSearchResult {
    #[serde(rename = "serverName")]
    pub server_name: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub score: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct McpToolApprovalRequest {
    pub approval_token: String,
    pub server_name: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub message: String,
    pub capability_risk_tier: String,
    pub capability_reason: String,
    pub expires_at_ms: u64,
    pub argument_summary: String,
    pub sensitive_fields: Vec<String>,
    pub canonical_origin: Option<String>,
    pub transport: String,
    pub resolved_destination_class: Option<String>,
    pub destination_binding: Option<String>,
    pub server_identity_binding: Option<String>,
    pub certificate_binding: Option<String>,
    pub tool_definition_binding: String,
    pub audit_id: String,
    pub response_byte_limit: usize,
    pub native_shield_approved: bool,
    #[serde(default)]
    pub chat_session_approved: bool,
    #[serde(default)]
    pub approval_scope_kinds: Vec<String>,
}

/// The exact, non-secret authority that a person reviews before an MCP call.
/// It omits transient tokens and times so resumed work can recheck the stable
/// destination, server, schema, and argument bindings.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpToolApprovalBinding {
    pub server_name: String,
    pub tool_name: String,
    pub arguments_binding: String,
    pub canonical_origin: Option<String>,
    pub transport: String,
    pub resolved_destination_class: Option<String>,
    pub destination_binding: Option<String>,
    pub server_identity_binding: Option<String>,
    pub certificate_binding: Option<String>,
    pub tool_definition_binding: String,
    pub response_byte_limit: usize,
    pub requires_native_shield: bool,
}

#[derive(Clone)]
struct PendingMcpToolApproval {
    request: McpToolApprovalRequest,
    arguments_binding: String,
    session: Option<Arc<McpClientSession>>,
    public_search_turn_binding: Option<PublicSearchApprovalTurnBinding>,
}

#[derive(Clone)]
struct PreparedMcpToolApproval {
    request: McpToolApprovalRequest,
    arguments_binding: String,
    workflow_arguments_binding: String,
    workflow_server_identity_binding: Option<String>,
    workflow_tool_definition_binding: String,
    requires_native_shield: bool,
    session: Option<Arc<McpClientSession>>,
    public_search_turn_binding: Option<PublicSearchApprovalTurnBinding>,
}

impl PreparedMcpToolApproval {
    fn workflow_binding(&self) -> McpToolApprovalBinding {
        McpToolApprovalBinding {
            server_name: self.request.server_name.clone(),
            tool_name: self.request.tool_name.clone(),
            arguments_binding: self.workflow_arguments_binding.clone(),
            canonical_origin: self.request.canonical_origin.clone(),
            transport: self.request.transport.clone(),
            resolved_destination_class: self.request.resolved_destination_class.clone(),
            destination_binding: self.request.destination_binding.clone(),
            server_identity_binding: self.workflow_server_identity_binding.clone(),
            certificate_binding: self.request.certificate_binding.clone(),
            tool_definition_binding: self.workflow_tool_definition_binding.clone(),
            response_byte_limit: self.request.response_byte_limit,
            requires_native_shield: self.requires_native_shield,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteToolAuthority {
    canonical_origin: String,
    transport: String,
    resolved_destination_class: String,
    destination_binding: String,
    server_identity_binding: String,
    workflow_server_identity_binding: String,
    certificate_binding: String,
}

struct VerifiedMcpToolExecution {
    session: Arc<McpClientSession>,
    tool_definition_binding: String,
    remote_authority: Option<RemoteToolAuthority>,
    audit_id: Option<String>,
    approval_scope_kinds: Vec<String>,
    chat_session_approved: bool,
    public_search_turn_binding: Option<PublicSearchApprovalTurnBinding>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct McpToolApproval {
    pub approval_token: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpInitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: serde_json::Value,
    #[serde(default, rename = "serverInfo")]
    pub server_info: Option<serde_json::Value>,
    #[serde(default)]
    pub instructions: Option<String>,
}

impl McpServerConfig {
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            env: HashMap::new(),
            transport: McpTransportConfig::Stdio,
        }
    }

    pub(crate) fn public_builtin_descriptor(&self) -> Self {
        Self {
            name: self.name.clone(),
            command: MCP_BACKEND_MANAGED_COMMAND.to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            transport: self.transport.clone(),
        }
    }
}

impl JsonRpcRequest {
    pub fn new(method: impl Into<String>, params: Value, id: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
            id,
        }
    }
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

impl McpClientSession {
    #[cfg(test)]
    async fn spawn(config: McpServerConfig) -> Result<Self, McpClientError> {
        if config.transport.is_remote() {
            let endpoint = config.transport.endpoint().ok_or_else(|| {
                McpClientError::transport("Test remote endpoint is unavailable.".to_string())
            })?;
            let destination = crate::network_policy::resolve_destination(
                endpoint,
                config.transport.destination_transport().ok_or_else(|| {
                    McpClientError::transport("Test remote transport is unavailable.".to_string())
                })?,
                config.transport.local_origin_grant(),
            )
            .await
            .map_err(|error| McpClientError::permission(error.message))?;
            let authorization =
                McpSpawnAuthorization::shield_approved(&config, Some(destination.clone()));
            return Self::spawn_with_destination(config, Some(&destination), None, authorization)
                .await;
        }
        Self::spawn_with_destination(config, None, None, McpSpawnAuthorization::Unapproved).await
    }

    async fn spawn_with_destination(
        config: McpServerConfig,
        approved_destination: Option<&CanonicalDestination>,
        supplied_remote_cancellation: Option<Arc<AtomicBool>>,
        activation_authorization: McpSpawnAuthorization,
    ) -> Result<Self, McpClientError> {
        let config_binding = mcp_config_binding(&config);
        if !matches!(activation_authorization, McpSpawnAuthorization::Unapproved)
            && !activation_authorization.is_bound_to(&config)
        {
            return Err(McpClientError::permission(
                "MCP session activation did not match the exact server configuration.".to_string(),
            ));
        }
        let trusted_internal_config_binding = match &activation_authorization {
            McpSpawnAuthorization::TrustedInternal { config_binding } => {
                Some(config_binding.clone())
            }
            McpSpawnAuthorization::Unapproved
            | McpSpawnAuthorization::ShieldGateApproved { .. } => None,
        };
        if matches!(config.transport, McpTransportConfig::Native) {
            let native_server = NativeServer::from_config(&config)?;
            return Ok(Self {
                server_name: config.name,
                config_binding,
                trusted_internal_config_binding,
                tx: None,
                child_handle: StdMutex::new(None),
                transport: config.transport,
                native_server: Some(native_server),
                remote_client: None,
                remote_cancellation: Arc::new(AtomicBool::new(false)),
                pending: Arc::new(Mutex::new(HashMap::new())),
                server_capabilities: Mutex::new(None),
                server_info: Mutex::new(None),
                server_instructions: Mutex::new(None),
                tools_stale: Arc::new(Mutex::new(false)),
            });
        }

        if config.transport.is_remote() {
            let remote_cancellation =
                supplied_remote_cancellation.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
            config
                .transport
                .route_class()
                .map_err(|error| McpClientError::transport(error.message))?;
            let approved_destination = approved_destination.ok_or_else(|| {
                McpClientError::permission(
                    "Remote MCP transport cannot be created without a bound native destination approval."
                        .to_string(),
                )
            })?;
            let remote_client = client_sse::build_remote_transport_client(
                &config.transport,
                approved_destination,
                &remote_cancellation,
            )
            .await?;

            return Ok(Self {
                server_name: config.name,
                config_binding,
                trusted_internal_config_binding,
                tx: None,
                child_handle: StdMutex::new(None),
                transport: config.transport,
                native_server: None,
                remote_client: Some(remote_client),
                remote_cancellation,
                pending: Arc::new(Mutex::new(HashMap::new())),
                server_capabilities: Mutex::new(None),
                server_info: Mutex::new(None),
                server_instructions: Mutex::new(None),
                tools_stale: Arc::new(Mutex::new(false)),
            });
        }

        let mut command = Command::new(&config.command);
        if config
            .env
            .get("OOMU_MCP_ENV_ISOLATION")
            .is_some_and(|value| value.eq_ignore_ascii_case("strict"))
        {
            command.env_clear();
        }
        command
            .args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().map_err(|error| {
            McpClientError::transport(format!(
                "Failed to spawn MCP server '{}': {error}",
                config.name
            ))
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            McpClientError::transport(format!(
                "MCP server '{}' did not expose writable stdin.",
                config.name
            ))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            McpClientError::transport(format!(
                "MCP server '{}' did not expose readable stdout.",
                config.name
            ))
        })?;

        let stderr = child.stderr.take();
        let (tx, rx) = mpsc::channel(MCP_REQUEST_QUEUE_DEPTH);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let tools_stale = Arc::new(Mutex::new(false));

        spawn_writer_loop(
            config.name.clone(),
            config.transport.clone(),
            stdin,
            rx,
            pending.clone(),
        );
        spawn_reader_loop(
            config.name.clone(),
            stdout,
            pending.clone(),
            tools_stale.clone(),
        );
        if let Some(stderr) = stderr {
            spawn_stderr_log_loop(config.name.clone(), stderr);
        }

        Ok(Self {
            server_name: config.name,
            config_binding,
            trusted_internal_config_binding,
            tx: Some(tx),
            child_handle: StdMutex::new(Some(child)),
            transport: config.transport,
            native_server: None,
            remote_client: None,
            remote_cancellation: Arc::new(AtomicBool::new(false)),
            pending,
            server_capabilities: Mutex::new(None),
            server_info: Mutex::new(None),
            server_instructions: Mutex::new(None),
            tools_stale,
        })
    }

    fn has_trusted_internal_activation_for(&self, config: &McpServerConfig) -> bool {
        let expected_binding = mcp_config_binding(config);
        self.config_binding == expected_binding
            && self.trusted_internal_config_binding.as_deref() == Some(expected_binding.as_str())
    }

    pub async fn initialize(&self) -> Result<McpInitializeResult, McpClientError> {
        let response = self
            .send_request(JsonRpcRequest::new(
                "initialize",
                serde_json::json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {
                        "name": MCP_CLIENT_NAME,
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
                next_request_id(),
            ))
            .await?;
        let result = json_rpc_response_result(response)?;
        let initialize_result: McpInitializeResult =
            serde_json::from_value(result).map_err(|error| {
                McpClientError::protocol(format!("Invalid MCP initialize result: {error}"))
            })?;

        *self.server_capabilities.lock().await = Some(initialize_result.capabilities.clone());
        *self.server_info.lock().await = initialize_result.server_info.clone();
        *self.server_instructions.lock().await = initialize_result.instructions.clone();

        self.send_notification(JsonRpcNotification::new(
            "notifications/initialized",
            serde_json::json!({}),
        ))
        .await?;

        Ok(initialize_result)
    }

    pub async fn send_request(
        &self,
        request: JsonRpcRequest,
    ) -> Result<JsonRpcResponse, McpClientError> {
        validate_json_rpc_request(&request)?;
        if matches!(self.transport, McpTransportConfig::Native) {
            let native_server = self.native_server.as_ref().ok_or_else(|| {
                McpClientError::transport(format!(
                    "Native MCP server '{}' is not initialized.",
                    self.server_name
                ))
            })?;
            return Ok(native_server.handle_request(request));
        }

        if self.transport.is_remote() {
            let client = self.remote_client.as_ref().ok_or_else(|| {
                McpClientError::transport(format!(
                    "MCP HTTP transport for '{}' is not initialized.",
                    self.server_name
                ))
            })?;
            let result = client_sse::send_remote_request(
                &self.server_name,
                &self.transport,
                client,
                request,
                &self.remote_cancellation,
            )
            .await;
            // A remote timeout, cancellation, malformed response, byte/shape
            // cap, redirect violation, or certificate-policy failure is
            // terminal for this approved transport. Never reuse its pool or
            // prior authority after a boundary failure.
            if result.is_err() {
                self.remote_cancellation.store(true, Ordering::Release);
            }
            return result;
        }

        let id_key = request_id_key(&request.id)?;
        let (response_tx, response_rx) = oneshot::channel();
        let tx = self.tx.as_ref().ok_or_else(|| {
            McpClientError::transport(format!(
                "MCP stdio request channel for '{}' is not available.",
                self.server_name
            ))
        })?;

        self.pending
            .lock()
            .await
            .insert(id_key.clone(), response_tx);
        if let Err(error) = tx.send(JsonRpcOutboundMessage::Request(request)).await {
            self.pending.lock().await.remove(&id_key);
            return Err(McpClientError::transport(format!(
                "Failed to queue MCP request for '{}': {error}",
                self.server_name
            )));
        }

        response_rx.await.map_err(|error| {
            McpClientError::transport(format!(
                "MCP response channel for '{}' closed before delivery: {error}",
                self.server_name
            ))
        })?
    }

    pub async fn send_notification(
        &self,
        notification: JsonRpcNotification,
    ) -> Result<(), McpClientError> {
        validate_json_rpc_method(&notification.method)?;
        if matches!(self.transport, McpTransportConfig::Native) {
            let native_server = self.native_server.as_ref().ok_or_else(|| {
                McpClientError::transport(format!(
                    "Native MCP server '{}' is not initialized.",
                    self.server_name
                ))
            })?;
            return native_server
                .handle_notification(notification)
                .map_err(McpClientError::protocol);
        }

        if self.transport.is_remote() {
            let client = self.remote_client.as_ref().ok_or_else(|| {
                McpClientError::transport(format!(
                    "MCP HTTP transport for '{}' is not initialized.",
                    self.server_name
                ))
            })?;
            let result = client_sse::send_remote_notification(
                &self.server_name,
                &self.transport,
                client,
                notification,
                &self.remote_cancellation,
            )
            .await;
            if result.is_err() {
                self.remote_cancellation.store(true, Ordering::Release);
            }
            return result;
        }

        let tx = self.tx.as_ref().ok_or_else(|| {
            McpClientError::transport(format!(
                "MCP stdio notification channel for '{}' is not available.",
                self.server_name
            ))
        })?;
        tx.send(JsonRpcOutboundMessage::Notification(notification))
            .await
            .map_err(|error| {
                McpClientError::transport(format!(
                    "Failed to queue MCP notification for '{}': {error}",
                    self.server_name
                ))
            })
    }

    pub async fn capabilities(&self) -> Option<Value> {
        self.server_capabilities.lock().await.clone()
    }

    pub async fn server_info(&self) -> Option<Value> {
        self.server_info.lock().await.clone()
    }
}

impl McpClientRegistry {
    pub async fn register_server_configs<I>(&self, configs: I) -> usize
    where
        I: IntoIterator<Item = McpServerConfig>,
    {
        self.register_server_configs_with_authorization(configs, false)
            .await
    }

    pub(crate) async fn register_trusted_server_configs<I>(&self, configs: I) -> usize
    where
        I: IntoIterator<Item = McpServerConfig>,
    {
        self.register_server_configs_with_authorization(configs, true)
            .await
    }

    async fn register_server_configs_with_authorization<I>(
        &self,
        configs: I,
        trusted_internal: bool,
    ) -> usize
    where
        I: IntoIterator<Item = McpServerConfig>,
    {
        if !self.accepting_work.load(Ordering::Acquire) {
            return 0;
        }
        let mut registered = self.configs.lock().await;
        let mut trusted_builtin_configs = self.trusted_builtin_configs.lock().await;
        let mut spawn_authorizations = self.spawn_authorizations.lock().await;
        let mut registered_count = 0;
        for config in configs {
            let name = config.name.clone();
            if !trusted_internal && trusted_builtin_configs.contains_key(&name) {
                continue;
            }
            if trusted_internal {
                if trusted_builtin_configs
                    .get(&name)
                    .is_some_and(|existing| existing != &config)
                {
                    continue;
                }
                trusted_builtin_configs.insert(name.clone(), config.clone());
            }
            if matches!(config.transport, McpTransportConfig::Stdio) && trusted_internal {
                spawn_authorizations.insert(
                    name.clone(),
                    McpSpawnAuthorization::trusted_internal(&config),
                );
            } else {
                spawn_authorizations.remove(&name);
            }
            registered.insert(name, config);
            registered_count += 1;
        }
        registered_count
    }

    pub async fn connect_server(
        &self,
        config: McpServerConfig,
    ) -> Result<McpServerState, McpClientError> {
        self.connect_server_with_authorization(config, McpSpawnAuthorization::Unapproved)
            .await
    }

    async fn connect_server_with_authorization(
        &self,
        config: McpServerConfig,
        authorization: McpSpawnAuthorization,
    ) -> Result<McpServerState, McpClientError> {
        if !self.accepting_work.load(Ordering::Acquire) {
            return Err(McpClientError::cancelled(
                "MCP runtime is shutting down and is not accepting new work.".to_string(),
            ));
        }
        let config = self.resolve_trusted_native_config(config).await?;
        enforce_mcp_connection_authorization(&config, &authorization).await?;
        let server_name = config.name.clone();
        let is_remote = config.transport.is_remote();
        let cancellation = Arc::new(AtomicBool::new(false));
        if is_remote {
            if let Some(replaced) = self
                .connecting_remote
                .lock()
                .await
                .insert(server_name.clone(), cancellation.clone())
            {
                replaced.store(true, Ordering::Release);
            }
            #[cfg(test)]
            self.pause_remote_connect_for_test(RemoteConnectTestPhase::BeforeBuild)
                .await;
        }
        let session = match McpClientSession::spawn_with_destination(
            config.clone(),
            authorization.remote_destination(),
            is_remote.then(|| cancellation.clone()),
            authorization.clone(),
        )
        .await
        {
            Ok(session) => Arc::new(session),
            Err(error) => {
                if is_remote {
                    self.remove_connecting_remote_if_same(&server_name, &cancellation)
                        .await;
                }
                return Err(error);
            }
        };
        let setup_result = async {
            let initialize_result = session.initialize().await?;
            let tools = list_tools_for_session(&session).await?;
            Ok::<_, McpClientError>((initialize_result, tools))
        }
        .await;
        let (initialize_result, tools) = match setup_result {
            Ok(result) => result,
            Err(error) => {
                if is_remote {
                    self.remove_connecting_remote_if_same(&server_name, &cancellation)
                        .await;
                }
                return Err(error);
            }
        };

        let _lifecycle = self.connection_lifecycle.lock().await;
        if !self.accepting_work.load(Ordering::Acquire) {
            session.remote_cancellation.store(true, Ordering::Release);
            return Err(McpClientError::cancelled(format!(
                "MCP server '{server_name}' was stopped because OOMU is shutting down."
            )));
        }
        let mut sessions = self.sessions.lock().await;
        let mut connecting = self.connecting_remote.lock().await;
        if is_remote {
            let is_current = connecting
                .get(&server_name)
                .is_some_and(|current| Arc::ptr_eq(current, &cancellation));
            if cancellation.load(Ordering::Acquire) || !is_current {
                if is_current {
                    connecting.remove(&server_name);
                }
                return Err(McpClientError::cancelled(format!(
                    "Remote MCP connection for '{server_name}' was cancelled before activation."
                )));
            }
        }

        if let Some(replaced) = sessions.get(&server_name) {
            replaced.remote_cancellation.store(true, Ordering::Release);
        }
        self.tool_catalog
            .lock()
            .await
            .insert(server_name.clone(), tools.clone());
        self.configs
            .lock()
            .await
            .insert(server_name.clone(), config.clone());
        self.pending_tool_approvals
            .lock()
            .await
            .retain(|_, pending| pending.request.server_name != server_name);
        self.spawn_authorizations.lock().await.remove(&server_name);
        sessions.insert(server_name.clone(), session.clone());
        if is_remote {
            connecting.remove(&server_name);
        }
        drop(connecting);
        drop(sessions);

        // A connection approval is one-use and is consumed by activation.
        let _ = authorization;

        #[cfg(test)]
        if is_remote {
            self.pause_remote_connect_for_test(RemoteConnectTestPhase::DuringActivation)
                .await;
        }

        if is_remote {
            let is_active = self
                .sessions
                .lock()
                .await
                .get(&server_name)
                .is_some_and(|current| Arc::ptr_eq(current, &session));
            if cancellation.load(Ordering::Acquire) || !is_active {
                self.sessions.lock().await.retain(|name, current| {
                    name != &server_name || !Arc::ptr_eq(current, &session)
                });
                self.tool_catalog.lock().await.remove(&server_name);
                return Err(McpClientError::cancelled(format!(
                    "Remote MCP connection for '{server_name}' was cancelled during activation."
                )));
            }
        }

        Ok(McpServerState {
            name: server_name,
            status: McpServerStatus::Connected,
            tools,
            protocol_version: Some(initialize_result.protocol_version),
            server_info: initialize_result.server_info,
            capabilities: Some(initialize_result.capabilities),
        })
    }

    async fn resolve_trusted_native_config(
        &self,
        config: McpServerConfig,
    ) -> Result<McpServerConfig, McpClientError> {
        if !matches!(config.transport, McpTransportConfig::Native) {
            return Ok(config);
        }
        let trusted = self
            .trusted_builtin_configs
            .lock()
            .await
            .get(&config.name)
            .cloned()
            .ok_or_else(|| {
                McpClientError::permission(
                    "Native MCP configuration is not a backend-trusted built-in descriptor."
                        .to_string(),
                )
            })?;
        if trusted != config {
            return Err(McpClientError::permission(
                "Native MCP configuration did not match its backend-trusted built-in descriptor."
                    .to_string(),
            ));
        }
        Ok(trusted)
    }

    async fn resolve_renderer_connect_config(
        &self,
        config: McpServerConfig,
    ) -> Result<McpServerConfig, McpClientError> {
        let trusted = self
            .trusted_builtin_configs
            .lock()
            .await
            .get(&config.name)
            .cloned();
        let Some(trusted) = trusted else {
            return Ok(config);
        };
        if trusted.public_builtin_descriptor() != config {
            return Err(McpClientError::permission(
                "Built-in MCP connection descriptor did not match the backend-issued public descriptor."
                    .to_string(),
            ));
        }
        Ok(trusted)
    }

    async fn remove_connecting_remote_if_same(
        &self,
        server_name: &str,
        cancellation: &Arc<AtomicBool>,
    ) {
        let mut connecting = self.connecting_remote.lock().await;
        if connecting
            .get(server_name)
            .is_some_and(|current| Arc::ptr_eq(current, cancellation))
        {
            connecting.remove(server_name);
        }
    }

    #[cfg(test)]
    async fn pause_remote_connect_for_test(&self, phase: RemoteConnectTestPhase) {
        let hook = self.remote_connect_test_hook.lock().await.clone();
        if let Some(hook) = hook.filter(|hook| hook.phase == phase) {
            hook.entered.notify_one();
            hook.release.notified().await;
        }
    }

    #[cfg(test)]
    async fn pause_remote_tool_execution_for_test(&self) {
        let hook = self.remote_tool_execution_test_hook.lock().await.clone();
        if let Some(hook) = hook {
            hook.entered.notify_one();
            hook.release.notified().await;
        }
    }

    pub async fn ensure_server_connected(&self, server_name: &str) -> Result<(), McpClientError> {
        match self.list_tools(server_name).await {
            Ok(_) => Ok(()),
            Err(error) if error.code == "mcp_transport_error" => {
                self.restart_server(server_name).await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn list_tools(&self, server_name: &str) -> Result<Vec<McpTool>, McpClientError> {
        let session = self.session(server_name).await?;
        let tools = match list_tools_for_session(&session).await {
            Ok(tools) => tools,
            Err(error) => {
                if session.transport.is_remote() {
                    self.invalidate_exact_remote_session(server_name, &session)
                        .await;
                }
                return Err(error);
            }
        };
        self.tool_catalog
            .lock()
            .await
            .insert(server_name.to_string(), tools.clone());
        Ok(tools)
    }

    /// Returns schemas only for live native sessions. Renderer-provided tool
    /// names are never capability authority for model selection.
    pub async fn connected_tool_catalog(&self) -> Vec<(String, McpTool)> {
        let connected = self
            .sessions
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let catalog = self.tool_catalog.lock().await;
        let mut tools = catalog
            .iter()
            .filter(|(server_name, _)| connected.contains(*server_name))
            .flat_map(|(server_name, tools)| {
                tools
                    .iter()
                    .cloned()
                    .map(|tool| (server_name.clone(), tool))
            })
            .collect::<Vec<_>>();
        tools.sort_by(|(left_server, left_tool), (right_server, right_tool)| {
            left_server
                .cmp(right_server)
                .then_with(|| left_tool.name.cmp(&right_tool.name))
        });
        tools
    }

    pub async fn search_tools(
        &self,
        query: &str,
    ) -> Result<Vec<McpToolSearchResult>, McpClientError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(McpClientError::protocol(
                "MCP tool search query must be non-empty.".to_string(),
            ));
        }

        let terms = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let catalog = self.tool_catalog.lock().await;
        let mut results = Vec::new();

        for (server_name, tools) in catalog.iter() {
            for tool in tools {
                let haystack = format!("{} {}", tool.name, tool.description).to_lowercase();
                let name = tool.name.to_lowercase();
                let mut score = 0_u32;

                for term in &terms {
                    if name.contains(term) {
                        score += 10;
                    }
                    if haystack.contains(term) {
                        score += 3;
                    }
                }

                if score > 0 {
                    results.push(McpToolSearchResult {
                        server_name: server_name.clone(),
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        score,
                    });
                }
            }
        }

        results.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.server_name.cmp(&right.server_name))
                .then_with(|| left.name.cmp(&right.name))
        });
        results.truncate(25);
        Ok(results)
    }

    pub async fn cached_tool_catalog(&self) -> HashMap<String, Vec<McpTool>> {
        self.tool_catalog.lock().await.clone()
    }

    pub async fn cancel_remote_operations(&self, server_name: Option<&str>) -> usize {
        let _lifecycle = self.connection_lifecycle.lock().await;
        let target_names = {
            let mut sessions = self.sessions.lock().await;
            let mut connecting = self.connecting_remote.lock().await;
            let active_targets = sessions
                .iter()
                .filter(|(name, session)| {
                    session.transport.is_remote()
                        && server_name.is_none_or(|requested| requested == name.as_str())
                })
                .map(|(name, session)| (name.clone(), session.clone()))
                .collect::<Vec<_>>();
            let connecting_targets = connecting
                .iter()
                .filter(|(name, _)| server_name.is_none_or(|requested| requested == name.as_str()))
                .map(|(name, cancellation)| (name.clone(), cancellation.clone()))
                .collect::<Vec<_>>();
            let target_names = active_targets
                .iter()
                .map(|(name, _)| name.clone())
                .chain(connecting_targets.iter().map(|(name, _)| name.clone()))
                .collect::<std::collections::HashSet<_>>();
            for (_, session) in &active_targets {
                session.remote_cancellation.store(true, Ordering::Release);
            }
            for (_, cancellation) in &connecting_targets {
                cancellation.store(true, Ordering::Release);
            }
            sessions.retain(|name, _| !target_names.contains(name));
            connecting.retain(|name, _| !target_names.contains(name));
            target_names
        };
        if target_names.is_empty() {
            return 0;
        }
        self.tool_catalog
            .lock()
            .await
            .retain(|name, _| !target_names.contains(name));
        self.pending_tool_approvals
            .lock()
            .await
            .retain(|_, pending| !target_names.contains(&pending.request.server_name));
        target_names.len()
    }

    pub async fn get_tool_details(
        &self,
        server_name: &str,
        tool_name: &str,
    ) -> Result<McpTool, McpClientError> {
        if tool_name.trim().is_empty() {
            return Err(McpClientError::protocol(
                "MCP tool name must be non-empty.".to_string(),
            ));
        }

        let tools = {
            let catalog = self.tool_catalog.lock().await;
            catalog.get(server_name).cloned()
        };
        let tools = match tools {
            Some(tools) => tools,
            None => self.list_tools(server_name).await?,
        };

        tools
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .ok_or_else(|| {
                McpClientError::protocol(format!(
                    "MCP tool '{tool_name}' is not registered on server '{server_name}'."
                ))
            })
    }

    pub async fn prepare_tool_approval(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<McpToolApprovalRequest>, McpClientError> {
        let Some(prepared) = self
            .prepare_tool_approval_candidate(server_name, tool_name, arguments)
            .await?
        else {
            return Ok(None);
        };
        self.activate_prepared_tool_approval(prepared, false)
            .await
            .map(Some)
    }

    pub(crate) async fn prepare_tool_approval_binding_for_review(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<McpToolApprovalBinding>, McpClientError> {
        Ok(self
            .prepare_tool_approval_candidate(server_name, tool_name, arguments)
            .await?
            .map(|prepared| prepared.workflow_binding()))
    }

    pub(crate) async fn activate_tool_approval_after_verified_workflow_review(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        expected_binding: &McpToolApprovalBinding,
    ) -> Result<Option<McpToolApprovalRequest>, McpClientError> {
        let Some(prepared) = self
            .prepare_tool_approval_candidate(server_name, tool_name, arguments)
            .await?
        else {
            return Ok(None);
        };
        if &prepared.workflow_binding() != expected_binding {
            return Err(McpClientError::permission(
                "The connected service changed after permission was reviewed. No action was taken."
                    .to_string(),
            ));
        }
        let native_shield_approved = prepared.requires_native_shield;
        self.activate_prepared_tool_approval(prepared, native_shield_approved)
            .await
            .map(Some)
    }

    async fn prepare_tool_approval_candidate(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<PreparedMcpToolApproval>, McpClientError> {
        validate_tool_arguments(&arguments)?;
        let (tool, remote_authority, session) = self
            .current_tool_and_remote_authority(server_name, tool_name)
            .await?;
        let classification =
            classify_mcp_tool_call(server_name, tool_name, tool.annotations.as_ref());
        if remote_authority.is_none() && !classification.requires_human_approval() {
            return Ok(None);
        }

        let now = unix_time_ms_u64();
        let argument_summary = crate::redaction::redacted_argument_summary(&arguments);
        let sensitive_report = crate::redaction::inspect_sensitive_json(&arguments);
        let sensitive_fields = sensitive_report
            .findings
            .into_iter()
            .map(|finding| format!("{}:{}", finding.class, finding.path))
            .collect::<Vec<_>>();
        let arguments_binding = argument_binding(&arguments);
        let workflow_arguments_binding = workflow_argument_binding(&arguments);
        let tool_definition_binding = tool_definition_binding(&tool);
        let workflow_tool_definition_binding = workflow_tool_definition_binding(&tool);
        let workflow_server_identity_binding = remote_authority
            .as_ref()
            .map(|authority| authority.workflow_server_identity_binding.clone());
        let audit_id = generate_mcp_audit_id();
        let canonical_origin = remote_authority
            .as_ref()
            .map(|authority| authority.canonical_origin.clone());
        let approval = McpToolApprovalRequest {
            approval_token: generate_tool_approval_token(),
            server_name: server_name.to_string(),
            tool_name: tool_name.to_string(),
            arguments: crate::redaction::redact_json_value(&arguments),
            message: format!(
                "Approve MCP {tier} tool {server_name} / {tool_name} at {origin}; arguments={argument_summary}",
                tier = classification.tier.as_str(),
                origin = canonical_origin.as_deref().unwrap_or("local native boundary")
            ),
            capability_risk_tier: classification.tier.as_str().to_string(),
            capability_reason: if remote_authority.is_some() {
                format!(
                    "{}; remote MCP calls always require exact one-use approval",
                    classification.reason
                )
            } else {
                classification.reason
            },
            expires_at_ms: now.saturating_add(MCP_TOOL_APPROVAL_TTL_MS),
            argument_summary,
            sensitive_fields,
            canonical_origin,
            transport: remote_authority
                .as_ref()
                .map(|authority| authority.transport.clone())
                .unwrap_or_else(|| "local".to_string()),
            resolved_destination_class: remote_authority
                .as_ref()
                .map(|authority| authority.resolved_destination_class.clone()),
            destination_binding: remote_authority
                .as_ref()
                .map(|authority| authority.destination_binding.clone()),
            server_identity_binding: remote_authority
                .as_ref()
                .map(|authority| authority.server_identity_binding.clone()),
            certificate_binding: remote_authority
                .as_ref()
                .map(|authority| authority.certificate_binding.clone()),
            tool_definition_binding,
            audit_id,
            response_byte_limit: client_sse::REMOTE_RESPONSE_BYTE_LIMIT,
            native_shield_approved: false,
            chat_session_approved: false,
            approval_scope_kinds: vec!["once".to_string()],
        };

        Ok(Some(PreparedMcpToolApproval {
            request: approval,
            arguments_binding,
            workflow_arguments_binding,
            workflow_server_identity_binding,
            workflow_tool_definition_binding,
            requires_native_shield: remote_authority.is_some(),
            session,
            public_search_turn_binding: None,
        }))
    }

    async fn activate_prepared_tool_approval(
        &self,
        prepared: PreparedMcpToolApproval,
        native_shield_approved: bool,
    ) -> Result<McpToolApprovalRequest, McpClientError> {
        if prepared.requires_native_shield && !native_shield_approved {
            return Err(McpClientError::permission(
                "Remote MCP tool authority was not activated because no native Shield decision was supplied."
                    .to_string(),
            ));
        }
        let now = unix_time_ms_u64();
        if prepared.request.expires_at_ms < now
            && !(prepared.requires_native_shield && native_shield_approved)
        {
            return Err(McpClientError::permission(
                "MCP tool approval expired before authority could be activated.".to_string(),
            ));
        }
        let _lifecycle = self.connection_lifecycle.lock().await;
        if let Some(prepared_session) = prepared.session.as_ref() {
            let is_active = self
                .sessions
                .lock()
                .await
                .get(&prepared.request.server_name)
                .is_some_and(|current| Arc::ptr_eq(current, prepared_session));
            if !is_active || prepared_session.remote_cancellation.load(Ordering::Acquire) {
                return Err(McpClientError::permission(
                    "MCP tool authority was not activated because the approved server session changed."
                        .to_string(),
                ));
            }
        } else if prepared.requires_native_shield {
            return Err(McpClientError::permission(
                "Remote MCP tool authority is missing its exact approved server session."
                    .to_string(),
            ));
        }
        let mut activated_request = prepared.request;
        if prepared.requires_native_shield && native_shield_approved {
            // The native review can remain open longer than the one-use MCP
            // token's original preparation window. Start a fresh, bounded
            // execution window only after the exact server session and
            // bindings have been rechecked above.
            activated_request.expires_at_ms = now.saturating_add(MCP_TOOL_APPROVAL_TTL_MS);
        }
        activated_request.native_shield_approved = native_shield_approved;
        let mut pending = self.pending_tool_approvals.lock().await;
        prune_expired_tool_approvals(&mut pending, now);
        pending.insert(
            activated_request.approval_token.clone(),
            PendingMcpToolApproval {
                request: activated_request.clone(),
                arguments_binding: prepared.arguments_binding,
                session: prepared.session.clone(),
                public_search_turn_binding: prepared.public_search_turn_binding.clone(),
            },
        );
        Ok(activated_request)
    }

    #[cfg(test)]
    async fn prepare_remote_tool_approval_after_native_shield_for_test(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<McpToolApprovalRequest>, McpClientError> {
        let Some(prepared) = self
            .prepare_tool_approval_candidate(server_name, tool_name, arguments)
            .await?
        else {
            return Ok(None);
        };
        if !prepared.requires_native_shield {
            return Err(McpClientError::permission(
                "Test expected a remote Shield-bound MCP call.".to_string(),
            ));
        }
        self.activate_prepared_tool_approval(prepared, true)
            .await
            .map(Some)
    }

    pub async fn reject_tool_approval(&self, approval_token: &str) -> Result<(), McpClientError> {
        if approval_token.trim().is_empty() {
            return Err(McpClientError::permission(
                "MCP approval token must be non-empty.".to_string(),
            ));
        }
        self.pending_tool_approvals
            .lock()
            .await
            .remove(approval_token.trim());
        Ok(())
    }

    pub async fn execute_tool_with_approval(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        approval: Option<McpToolApproval>,
    ) -> Result<McpToolCallResult, McpClientError> {
        self.execute_tool_with_approval_guarded(
            server_name,
            tool_name,
            arguments,
            approval,
            &|| Ok(()),
        )
        .await
    }

    pub async fn execute_tool_with_approval_guarded(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        approval: Option<McpToolApproval>,
        pre_invoke_guard: &(dyn Fn() -> Result<(), McpClientError> + Send + Sync),
    ) -> Result<McpToolCallResult, McpClientError> {
        let verified = self
            .ensure_tool_approval(server_name, tool_name, &arguments, approval)
            .await?;

        #[cfg(test)]
        if verified.remote_authority.is_some() {
            self.pause_remote_tool_execution_for_test().await;
        }

        let result = self
            .execute_tool_on_verified_session(
                server_name,
                tool_name,
                arguments,
                &verified,
                pre_invoke_guard,
            )
            .await;
        if let Some(audit_id) = verified.audit_id.as_deref() {
            eprintln!(
                "MCP_TOOL_SECURITY_EVENT audit_id={} server={} tool={} completion={}",
                audit_id,
                crate::redaction::redacted_log_text(server_name),
                crate::redaction::redacted_log_text(tool_name),
                if result.is_ok() {
                    "success"
                } else {
                    "blocked_or_failed"
                }
            );
        }
        result
    }

    pub async fn execute_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolCallResult, McpClientError> {
        self.execute_tool_with_approval(server_name, tool_name, arguments, None)
            .await
    }

    async fn execute_tool_on_verified_session(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        verified: &VerifiedMcpToolExecution,
        pre_invoke_guard: &(dyn Fn() -> Result<(), McpClientError> + Send + Sync),
    ) -> Result<McpToolCallResult, McpClientError> {
        if tool_name.trim().is_empty() {
            return Err(McpClientError::protocol(
                "MCP tool name must be non-empty.".to_string(),
            ));
        }

        self.revalidate_verified_tool_execution(server_name, tool_name, verified)
            .await?;
        let arguments = prepare_tool_arguments(arguments);
        pre_invoke_guard()?;
        let response = verified
            .session
            .send_request(JsonRpcRequest::new(
                "tools/call",
                serde_json::json!({
                    "name": tool_name,
                    "arguments": arguments,
                }),
                next_request_id(),
            ))
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                if verified.session.transport.is_remote() {
                    self.invalidate_exact_remote_session(server_name, &verified.session)
                        .await;
                }
                return Err(error);
            }
        };
        // A well-formed JSON-RPC application error is not a malformed
        // transport response. Any missing/invalid success result is terminal
        // for the exact approved remote session.
        let is_application_error = response.error.is_some();
        let parsed = json_rpc_response_result(response).and_then(parse_tool_call_result);
        if parsed.is_err() && !is_application_error && verified.session.transport.is_remote() {
            self.invalidate_exact_remote_session(server_name, &verified.session)
                .await;
        }
        parsed
    }

    async fn restart_server(&self, server_name: &str) -> Result<(), McpClientError> {
        let resolved_config = self.resolve_server_config(server_name).await?;
        let trusted_builtin_config = self
            .trusted_builtin_configs
            .lock()
            .await
            .get(server_name)
            .cloned()
            .filter(|trusted| trusted == &resolved_config);
        let (config, authorization) = if matches!(
            resolved_config.transport,
            McpTransportConfig::Native
        ) {
            let authorization = self.spawn_authorization_for(server_name).await;
            (resolved_config, authorization)
        } else if let Some(trusted) = trusted_builtin_config {
            // Bundled local helpers (for example the Apple-app bridge) are
            // registered by the Rust backend with an exact config binding.
            // Recreate only that verified descriptor; arbitrary stdio and all
            // remote transports still require a fresh one-use Shield approval.
            let authorization = McpSpawnAuthorization::trusted_internal(&trusted);
            (trusted, authorization)
        } else {
            self.spawn_authorizations.lock().await.remove(server_name);
            return Err(McpClientError::permission(format!(
                "MCP server '{server_name}' disconnected. A fresh one-use Shield Gate approval is required before reconnecting the non-native transport."
            )));
        };
        enforce_mcp_connection_authorization(&config, &authorization).await?;
        let session = Arc::new(
            McpClientSession::spawn_with_destination(
                config,
                authorization.remote_destination(),
                None,
                authorization.clone(),
            )
            .await?,
        );
        let initialize_result = session.initialize().await?;
        let tools = list_tools_for_session(&session).await?;

        self.sessions
            .lock()
            .await
            .insert(server_name.to_string(), session);
        self.tool_catalog
            .lock()
            .await
            .insert(server_name.to_string(), tools);
        log_mcp_stderr_line(
            server_name,
            &format!(
                "server restarted after transport failure; protocol={}",
                initialize_result.protocol_version
            ),
        )
        .await;
        Ok(())
    }

    async fn resolve_server_config(
        &self,
        server_name: &str,
    ) -> Result<McpServerConfig, McpClientError> {
        if server_name.trim().is_empty() {
            return Err(McpClientError::protocol(
                "MCP server name must be non-empty.".to_string(),
            ));
        }

        if let Some(config) = self.configs.lock().await.get(server_name).cloned() {
            return Ok(config);
        }

        let builtin_configs = headless_server_configs_for(server_name).map_err(|error| {
            McpClientError::transport(format!(
                "MCP server '{server_name}' has no registered config and built-in fallback resolution failed: {error}"
            ))
        })?;

        let mut resolved = None;
        let mut registered = self.configs.lock().await;
        let mut trusted_builtin_configs = self.trusted_builtin_configs.lock().await;
        let mut spawn_authorizations = self.spawn_authorizations.lock().await;
        if let Some(config) = registered.get(server_name).cloned() {
            return Ok(config);
        }
        for config in builtin_configs {
            let name = config.name.clone();
            if name == server_name {
                resolved = Some(config.clone());
            }
            trusted_builtin_configs.insert(name.clone(), config.clone());
            if matches!(config.transport, McpTransportConfig::Stdio) {
                spawn_authorizations.insert(
                    name.clone(),
                    McpSpawnAuthorization::trusted_internal(&config),
                );
            } else {
                spawn_authorizations.remove(&name);
            }
            registered.entry(name).or_insert(config);
        }

        resolved.ok_or_else(|| {
            McpClientError::transport(format!(
                "MCP server '{server_name}' is not registered and is not a built-in MCP server."
            ))
        })
    }

    async fn session(&self, server_name: &str) -> Result<Arc<McpClientSession>, McpClientError> {
        if server_name.trim().is_empty() {
            return Err(McpClientError::protocol(
                "MCP server name must be non-empty.".to_string(),
            ));
        }

        self.sessions
            .lock()
            .await
            .get(server_name)
            .cloned()
            .ok_or_else(|| {
                McpClientError::transport(format!("MCP server '{server_name}' is not connected."))
            })
    }

    async fn has_active_trusted_builtin_session(&self, server_name: &str) -> bool {
        let session = self.sessions.lock().await.get(server_name).cloned();
        let trusted_config = self
            .trusted_builtin_configs
            .lock()
            .await
            .get(server_name)
            .cloned();
        let registered_config = self.configs.lock().await.get(server_name).cloned();
        match (session, trusted_config, registered_config) {
            (Some(session), Some(trusted), Some(registered)) => {
                matches!(
                    trusted.transport,
                    McpTransportConfig::Stdio | McpTransportConfig::Native
                ) && mcp_config_binding(&trusted) == mcp_config_binding(&registered)
                    && session.has_trusted_internal_activation_for(&trusted)
            }
            _ => false,
        }
    }

    async fn spawn_authorization_for(&self, server_name: &str) -> McpSpawnAuthorization {
        self.spawn_authorizations
            .lock()
            .await
            .get(server_name)
            .cloned()
            .unwrap_or(McpSpawnAuthorization::Unapproved)
    }

    async fn ensure_tool_approval(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
        approval: Option<McpToolApproval>,
    ) -> Result<VerifiedMcpToolExecution, McpClientError> {
        validate_tool_arguments(arguments)?;
        let (tool, remote_authority, session) = self
            .current_tool_and_remote_authority(server_name, tool_name)
            .await?;
        let current_tool_binding = tool_definition_binding(&tool);
        let classification =
            classify_mcp_tool_call(server_name, tool_name, tool.annotations.as_ref());
        if remote_authority.is_none() && !classification.requires_human_approval() {
            let session = session.ok_or_else(|| {
                McpClientError::transport(format!(
                    "MCP server '{server_name}' is not connected for exact-session tool execution."
                ))
            })?;
            return Ok(VerifiedMcpToolExecution {
                session,
                tool_definition_binding: current_tool_binding,
                remote_authority,
                audit_id: None,
                approval_scope_kinds: vec!["once".to_string()],
                chat_session_approved: false,
                public_search_turn_binding: None,
            });
        }

        let approval = approval.ok_or_else(|| {
            McpClientError::permission(format!(
                "{MCP_AUTHORIZATION_MESSAGE} Approval is required before executing {tier} tool '{server_name} / {tool_name}': {reason}.",
                tier = classification.tier.as_str(),
                reason = classification.reason
            ))
        })?;
        let session = session.ok_or_else(|| {
            McpClientError::transport(format!(
                "MCP server '{server_name}' is not connected for exact-session tool execution."
            ))
        })?;
        let token = approval.approval_token.trim();
        if token.is_empty() {
            return Err(McpClientError::permission(
                "MCP approval token must be non-empty.".to_string(),
            ));
        }

        let now = unix_time_ms_u64();
        let pending = {
            let mut approvals = self.pending_tool_approvals.lock().await;
            prune_expired_tool_approvals(&mut approvals, now);
            approvals.remove(token)
        }
        .ok_or_else(|| {
            McpClientError::permission(format!(
                "{MCP_AUTHORIZATION_MESSAGE} The approval token is missing, expired, or already consumed."
            ))
        })?;

        if pending.request.expires_at_ms < now {
            return Err(McpClientError::permission(format!(
                "{MCP_AUTHORIZATION_MESSAGE} The approval token has expired."
            )));
        }
        let current_arguments_binding = argument_binding(arguments);
        let session_binding_matches = pending.session.as_ref().map_or_else(
            || remote_authority.is_none(),
            |approved_session| Arc::ptr_eq(approved_session, &session),
        );
        let remote_binding_matches = match (
            remote_authority.as_ref(),
            pending.request.canonical_origin.as_ref(),
        ) {
            (Some(current), Some(pending_origin)) => {
                pending_origin == &current.canonical_origin
                    && pending.request.transport == current.transport
                    && pending.request.resolved_destination_class.as_deref()
                        == Some(current.resolved_destination_class.as_str())
                    && pending.request.destination_binding.as_deref()
                        == Some(current.destination_binding.as_str())
                    && pending.request.server_identity_binding.as_deref()
                        == Some(current.server_identity_binding.as_str())
                    && pending.request.certificate_binding.as_deref()
                        == Some(current.certificate_binding.as_str())
            }
            (None, None) => true,
            _ => false,
        };
        if pending.request.server_name != server_name
            || pending.request.tool_name != tool_name
            || pending.arguments_binding != current_arguments_binding
            || pending.request.tool_definition_binding != current_tool_binding
            || !session_binding_matches
            || !remote_binding_matches
        {
            return Err(McpClientError::permission(format!(
                "{MCP_AUTHORIZATION_MESSAGE} The approval token does not match the requested MCP tool call."
            )));
        }

        eprintln!(
            "MCP_TOOL_SECURITY_EVENT audit_id={} server={} tool={} destination_binding={} response_limit={} decision=approved_consumed",
            pending.request.audit_id,
            crate::redaction::redacted_log_text(server_name),
            crate::redaction::redacted_log_text(tool_name),
            pending.request.destination_binding.as_deref().unwrap_or("local"),
            pending.request.response_byte_limit,
        );
        Ok(VerifiedMcpToolExecution {
            session,
            tool_definition_binding: current_tool_binding,
            remote_authority,
            audit_id: Some(pending.request.audit_id),
            approval_scope_kinds: pending.request.approval_scope_kinds,
            chat_session_approved: pending.request.chat_session_approved,
            public_search_turn_binding: pending.public_search_turn_binding,
        })
    }

    async fn current_tool_and_remote_authority(
        &self,
        server_name: &str,
        tool_name: &str,
    ) -> Result<
        (
            McpTool,
            Option<RemoteToolAuthority>,
            Option<Arc<McpClientSession>>,
        ),
        McpClientError,
    > {
        let session = self.sessions.lock().await.get(server_name).cloned();
        if let Some(session) = session {
            let refreshed = self
                .tool_and_remote_authority_for_session(&session, tool_name)
                .await;
            return match refreshed {
                Ok((tool, remote_authority)) => {
                    let is_active = self
                        .sessions
                        .lock()
                        .await
                        .get(server_name)
                        .is_some_and(|current| Arc::ptr_eq(current, &session));
                    if !is_active || session.remote_cancellation.load(Ordering::Acquire) {
                        Err(McpClientError::permission(
                            "MCP server session changed while tool authority was being refreshed."
                                .to_string(),
                        ))
                    } else {
                        Ok((tool, remote_authority, Some(session)))
                    }
                }
                Err(error) => {
                    if session.transport.is_remote() {
                        self.invalidate_exact_remote_session(server_name, &session)
                            .await;
                    }
                    Err(error)
                }
            };
        }

        let cached_tool = {
            let catalog = self.tool_catalog.lock().await;
            catalog
                .get(server_name)
                .and_then(|tools| tools.iter().find(|tool| tool.name == tool_name))
                .cloned()
        };
        if let Some(tool) = cached_tool {
            return Ok((tool, None, None));
        }
        self.ensure_server_connected(server_name).await?;
        let session = self.session(server_name).await?;
        let (tool, remote_authority) = self
            .tool_and_remote_authority_for_session(&session, tool_name)
            .await?;
        Ok((tool, remote_authority, Some(session)))
    }

    async fn tool_and_remote_authority_for_session(
        &self,
        session: &Arc<McpClientSession>,
        tool_name: &str,
    ) -> Result<(McpTool, Option<RemoteToolAuthority>), McpClientError> {
        // Tool metadata is attacker-controlled. Always refresh it on the exact
        // session whose authority will be approved or consumed.
        let tools = list_tools_for_session(session).await?;
        let tool = tools
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .ok_or_else(|| {
                McpClientError::protocol(format!(
                    "MCP tool '{tool_name}' is not registered on server '{}'.",
                    session.server_name
                ))
            })?;
        let remote_authority = self.remote_authority_for_session(session).await?;
        Ok((tool, remote_authority))
    }

    async fn remote_authority_for_session(
        &self,
        session: &Arc<McpClientSession>,
    ) -> Result<Option<RemoteToolAuthority>, McpClientError> {
        if !session.transport.is_remote() {
            return Ok(None);
        }
        let remote = session.remote_client.as_ref().ok_or_else(|| {
            McpClientError::transport(format!(
                "Remote MCP server '{}' has no destination binding.",
                session.server_name
            ))
        })?;
        let destination = remote.destination();
        client_sse::revalidate_remote_destination(destination, &session.remote_cancellation)
            .await?;
        let certificate_binding = remote.certificate_binding()?;
        let server_identity = serde_json::json!({
            "serverInfo": session.server_info.lock().await.clone(),
            "capabilities": session.server_capabilities.lock().await.clone(),
            "certificateBinding": certificate_binding,
        });
        Ok(Some(RemoteToolAuthority {
            canonical_origin: destination.canonical_origin().to_string(),
            transport: destination.transport().as_str().to_string(),
            resolved_destination_class: destination.destination_class().as_str().to_string(),
            destination_binding: destination.binding_fingerprint().to_string(),
            server_identity_binding: server_identity_binding(&server_identity),
            workflow_server_identity_binding: workflow_server_identity_binding(&server_identity),
            certificate_binding,
        }))
    }

    async fn revalidate_verified_tool_execution(
        &self,
        server_name: &str,
        tool_name: &str,
        verified: &VerifiedMcpToolExecution,
    ) -> Result<(), McpClientError> {
        let refreshed = self
            .tool_and_remote_authority_for_session(&verified.session, tool_name)
            .await;
        let (tool, remote_authority) = match refreshed {
            Ok(refreshed) => refreshed,
            Err(error) => {
                if verified.session.transport.is_remote() {
                    self.invalidate_exact_remote_session(server_name, &verified.session)
                        .await;
                }
                return Err(error);
            }
        };
        let is_active = self
            .sessions
            .lock()
            .await
            .get(server_name)
            .is_some_and(|current| Arc::ptr_eq(current, &verified.session));
        let authority_matches = remote_authority == verified.remote_authority;
        let tool_matches = tool_definition_binding(&tool) == verified.tool_definition_binding;
        if !is_active
            || verified.session.remote_cancellation.load(Ordering::Acquire)
            || !authority_matches
            || !tool_matches
        {
            if verified.session.transport.is_remote() {
                self.invalidate_exact_remote_session(server_name, &verified.session)
                    .await;
            }
            return Err(McpClientError::permission(
                "MCP tool execution was blocked because its exact approved session, destination, certificate, or tool definition changed."
                    .to_string(),
            ));
        }
        Ok(())
    }

    async fn invalidate_exact_remote_session(
        &self,
        server_name: &str,
        session: &Arc<McpClientSession>,
    ) {
        session.remote_cancellation.store(true, Ordering::Release);
        let _lifecycle = self.connection_lifecycle.lock().await;
        let removed = {
            let mut sessions = self.sessions.lock().await;
            if sessions
                .get(server_name)
                .is_some_and(|current| Arc::ptr_eq(current, session))
            {
                sessions.remove(server_name);
                true
            } else {
                false
            }
        };
        if removed {
            self.tool_catalog.lock().await.remove(server_name);
        }
        self.pending_tool_approvals
            .lock()
            .await
            .retain(|_, pending| {
                pending.request.server_name != server_name
                    || pending
                        .session
                        .as_ref()
                        .is_some_and(|approved| !Arc::ptr_eq(approved, session))
            });
    }
}

pub fn validate_mcp_binary_path(command: &str) -> Result<(), McpClientError> {
    let command = command.trim();
    if command.is_empty() {
        return Err(McpClientError::permission(
            "MCP stdio server command must be non-empty.".to_string(),
        ));
    }
    validate_no_shell_operator(command, "MCP stdio server command")?;

    if MCP_VETTED_RUNTIME_ALIASES.contains(&command) {
        return Ok(());
    }

    let command_path = Path::new(command);
    if !command_path.is_absolute() {
        return Err(McpClientError::permission(format!(
            "MCP stdio server command '{command}' is not a vetted runtime alias and must be an absolute executable path."
        )));
    }

    let canonical = command_path.canonicalize().map_err(|error| {
        McpClientError::permission(format!(
            "MCP stdio server command '{}' could not be canonicalized: {error}",
            command_path.display()
        ))
    })?;
    let metadata = canonical.metadata().map_err(|error| {
        McpClientError::permission(format!(
            "MCP stdio server command '{}' could not be inspected: {error}",
            canonical.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(McpClientError::permission(format!(
            "MCP stdio server command '{}' must resolve to a local executable file.",
            canonical.display()
        )));
    }
    if !is_executable_file(&metadata) {
        return Err(McpClientError::permission(format!(
            "MCP stdio server command '{}' is not executable.",
            canonical.display()
        )));
    }

    Ok(())
}

fn validate_mcp_stdio_server_config(config: &McpServerConfig) -> Result<(), McpClientError> {
    validate_mcp_binary_path(&config.command)?;
    for (index, arg) in config.args.iter().enumerate() {
        validate_no_shell_operator(arg, &format!("MCP stdio server argument #{index}"))?;
    }
    Ok(())
}

fn validate_no_shell_operator(value: &str, field_name: &str) -> Result<(), McpClientError> {
    if let Some(operator) = MCP_DENIED_SHELL_OPERATORS
        .iter()
        .find(|operator| value.contains(**operator))
    {
        return Err(McpClientError::permission(format!(
            "{field_name} contains denied shell operator '{operator}'."
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable_file(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable_file(_metadata: &std::fs::Metadata) -> bool {
    true
}

async fn enforce_mcp_connection_authorization(
    config: &McpServerConfig,
    authorization: &McpSpawnAuthorization,
) -> Result<(), McpClientError> {
    if matches!(config.transport, McpTransportConfig::Native) {
        return Ok(());
    }
    if matches!(config.transport, McpTransportConfig::Stdio) {
        validate_mcp_stdio_server_config(config)?;
    }
    let allowed = if matches!(config.transport, McpTransportConfig::Stdio) {
        authorization.allows_stdio_spawn(config)
    } else {
        authorization.allows_remote_connection(config)
    };
    if allowed {
        log_mcp_stderr_line(
            &config.name,
            &format!(
                "non-native MCP boundary authorized transport={} config_binding={}",
                mcp_transport_label(&config.transport),
                mcp_config_binding(config)
            ),
        )
        .await;
        return Ok(());
    }

    log_mcp_stderr_line(
        &config.name,
        &format!(
            "non-native MCP boundary blocked before exact Shield approval transport={}",
            mcp_transport_label(&config.transport)
        ),
    )
    .await;
    Err(McpClientError::permission(format!(
        "{MCP_AUTHORIZATION_MESSAGE} Non-native MCP server '{}' requires an exact, unexpired approval for transport '{}'.",
        config.name,
        mcp_transport_label(&config.transport)
    )))
}

async fn request_mcp_connection_approval(
    app: &tauri::AppHandle,
    approvals: &ShieldApprovalManager,
    config: &McpServerConfig,
) -> Result<McpSpawnAuthorization, McpClientError> {
    if matches!(config.transport, McpTransportConfig::Native) {
        return Ok(McpSpawnAuthorization::Unapproved);
    }

    let remote_destination = if matches!(config.transport, McpTransportConfig::Stdio) {
        validate_mcp_stdio_server_config(config)?;
        None
    } else {
        let endpoint = config.transport.endpoint().ok_or_else(|| {
            McpClientError::permission("Remote MCP endpoint is unavailable.".to_string())
        })?;
        let transport = config.transport.destination_transport().ok_or_else(|| {
            McpClientError::permission("Remote MCP transport is unavailable.".to_string())
        })?;
        Some(
            crate::network_policy::resolve_destination(
                endpoint,
                transport,
                config.transport.local_origin_grant(),
            )
            .await
            .map_err(|error| {
                McpClientError::permission(format!(
                    "Native destination policy blocked MCP connection approval: {}",
                    error.message
                ))
            })?,
        )
    };
    let approval_path = mcp_connection_approval_target(config, remote_destination.as_ref());
    let approval_action = RequestedAction {
        kind: "mcp_connect_server".to_string(),
        principal: Some(crate::redaction::redacted_log_text(&config.name)),
        path: Some(approval_path),
        content: Some(mcp_approval_preview(config, remote_destination.as_ref())),
    };
    let approval_request = shield_gate::build_shield_approval_request(&approval_action)
        .ok_or_else(|| {
            McpClientError::permission(
                "Shield Gate did not classify the non-native MCP connection as a high-risk action."
                    .to_string(),
            )
        })?;

    log_mcp_stderr_line(
        &config.name,
        &format!(
            "Shield Gate approval requested for non-native MCP boundary transport={} config_binding={}",
            mcp_transport_label(&config.transport),
            mcp_config_binding(config)
        ),
    )
    .await;
    match shield_gate::request_user_approval(app, approvals, approval_request).await {
        Ok(()) => {
            log_mcp_stderr_line(
                &config.name,
                &format!(
                    "Shield Gate approval granted for non-native MCP boundary transport={} config_binding={}",
                    mcp_transport_label(&config.transport),
                    mcp_config_binding(config)
                ),
            )
            .await;
            Ok(McpSpawnAuthorization::shield_approved(
                config,
                remote_destination,
            ))
        }
        Err(error) => {
            log_mcp_stderr_line(
                &config.name,
                &format!(
                    "Shield Gate approval denied for non-native MCP boundary transport={} code={}",
                    mcp_transport_label(&config.transport),
                    error.code
                ),
            )
            .await;
            Err(McpClientError::permission(format!(
                "Non-native MCP connection blocked by Shield Gate."
            )))
        }
    }
}

fn mcp_transport_label(transport: &McpTransportConfig) -> &'static str {
    match transport {
        McpTransportConfig::Stdio => "stdio",
        McpTransportConfig::Native => "native",
        McpTransportConfig::Http { .. } => "http",
        McpTransportConfig::Sse { .. } => "sse",
    }
}

fn mcp_connection_approval_target(
    config: &McpServerConfig,
    remote_destination: Option<&CanonicalDestination>,
) -> String {
    remote_destination
        .map(|destination| destination.canonical_origin().to_string())
        .unwrap_or_else(|| crate::redaction::redacted_log_text(&config.command))
}

fn mcp_config_binding(config: &McpServerConfig) -> String {
    let mut environment = config.env.iter().collect::<Vec<_>>();
    environment.sort_by(|left, right| left.0.cmp(right.0));
    let payload = serde_json::json!({
        "name": config.name,
        "command": config.command,
        "args": config.args,
        "environment": environment,
        "transport": config.transport,
    });
    keyed_json_binding(b"OOMU_MCP_CONFIG_BINDING_V1\0", &payload)
}

fn mcp_process_binding_key() -> &'static [u8; 32] {
    MCP_PROCESS_BINDING_KEY.get_or_init(|| {
        let mut key = [0_u8; 32];
        OsRng.fill_bytes(&mut key);
        key
    })
}

fn keyed_json_binding(domain: &[u8], value: &Value) -> String {
    let binding_key = mcp_process_binding_key();
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(binding_key);
    hasher.update(
        serde_json::to_vec(value).expect("JSON value produces a deterministic approval binding"),
    );
    hasher.update(binding_key);
    hex::encode(hasher.finalize())
}

fn stable_workflow_review_binding(domain: &[u8], value: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(
        serde_json::to_vec(value).expect("JSON value produces a deterministic review binding"),
    );
    hex::encode(hasher.finalize())
}

fn mcp_approval_preview(
    config: &McpServerConfig,
    remote_destination: Option<&CanonicalDestination>,
) -> String {
    if let Some(destination) = remote_destination {
        return serde_json::json!({
            "name": crate::redaction::redacted_log_text(&config.name),
            "transport": mcp_transport_label(&config.transport),
            "canonicalOrigin": destination.canonical_origin(),
            "canonicalDestination": destination.redacted_summary(),
            "operation": "initialize_and_list_tools",
            "configBinding": mcp_config_binding(config),
        })
        .to_string();
    }
    let preview = serde_json::json!({
        "name": crate::redaction::redacted_log_text(&config.name),
        "transport": mcp_transport_label(&config.transport),
        "command": crate::redaction::redacted_log_text(&config.command),
        "args": redacted_mcp_stdio_preview_arguments(&config.args),
        "envKeys": config.env.keys().collect::<Vec<_>>(),
        "operation": "initialize_and_list_tools",
        "configBinding": mcp_config_binding(config),
    });
    crate::redaction::redacted_argument_summary(&preview)
}

fn redacted_mcp_stdio_preview_arguments(arguments: &[String]) -> Vec<String> {
    const MAX_PREVIEW_ARGUMENTS: usize = 64;
    let mut redact_next = false;
    let mut preview = Vec::new();
    for argument in arguments.iter().take(MAX_PREVIEW_ARGUMENTS) {
        if redact_next {
            preview.push("[redacted]".to_string());
            redact_next = false;
            continue;
        }
        let (flag, inline_value) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(flag, value)| {
                (flag, Some(value))
            });
        let normalized_flag = flag
            .trim_start_matches('-')
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let sensitive_flag = [
            "authorization",
            "apikey",
            "password",
            "passwd",
            "credential",
            "secret",
            "token",
            "cookie",
            "privatekey",
        ]
        .iter()
        .any(|name| normalized_flag.contains(name));
        if sensitive_flag {
            if inline_value.is_some() {
                preview.push(format!(
                    "{}=[redacted]",
                    crate::redaction::redacted_log_text(flag)
                ));
            } else {
                preview.push(crate::redaction::redacted_log_text(argument));
                redact_next = true;
            }
        } else {
            preview.push(crate::redaction::redacted_log_text(argument));
        }
    }
    if arguments.len() > MAX_PREVIEW_ARGUMENTS {
        preview.push("...[truncated]".to_string());
    }
    preview
}

fn remote_mcp_tool_shield_action(
    prepared: &PreparedMcpToolApproval,
) -> Result<RequestedAction, McpClientError> {
    if !prepared.requires_native_shield {
        return Err(McpClientError::permission(
            "A native Shield request cannot be created for a non-remote MCP call.".to_string(),
        ));
    }
    let request = &prepared.request;
    let canonical_origin = request.canonical_origin.clone().ok_or_else(|| {
        McpClientError::permission(
            "Remote MCP approval is missing its canonical origin binding.".to_string(),
        )
    })?;
    let preview = serde_json::json!({
        "auditId": request.audit_id,
        "operation": "tools/call",
        "argumentsBinding": prepared.arguments_binding,
        "toolDefinitionBinding": request.tool_definition_binding,
        "destinationBinding": request.destination_binding,
        "serverIdentityBinding": request.server_identity_binding,
        "certificateBinding": request.certificate_binding,
        "canonicalOrigin": canonical_origin,
        "transport": request.transport,
        "serverName": crate::redaction::redacted_log_text(&request.server_name),
        "toolName": crate::redaction::redacted_log_text(&request.tool_name),
        "redactedArguments": request.arguments,
        "argumentSummary": request.argument_summary,
        "sensitiveFields": request.sensitive_fields,
        "responseByteLimit": request.response_byte_limit,
    })
    .to_string();
    Ok(RequestedAction {
        kind: "mcp_execute_remote_tool".to_string(),
        principal: Some(format!(
            "{} / {}",
            crate::redaction::redacted_log_text(&request.server_name),
            crate::redaction::redacted_log_text(&request.tool_name)
        )),
        path: Some(canonical_origin),
        content: Some(preview),
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_connect_server(
    config: McpServerConfig,
    registry: tauri::State<'_, McpClientRegistry>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    app: tauri::AppHandle,
) -> Result<McpServerState, String> {
    let config = registry
        .resolve_renderer_connect_config(config)
        .await
        .map_err(|error| error.message)?;
    let authorization = request_mcp_connection_approval(&app, approvals.inner(), &config)
        .await
        .map_err(|error| error.message)?;
    registry
        .connect_server_with_authorization(config, authorization)
        .await
        .map_err(|error| error.message)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_connect_builtin_server(
    server_name: String,
    registry: tauri::State<'_, McpClientRegistry>,
    app: tauri::AppHandle,
) -> Result<McpServerState, String> {
    let server_name = server_name.trim();
    if server_name.is_empty() {
        return Err("Built-in MCP server name must be non-empty.".to_string());
    }
    ensure_trusted_builtin_mcp_server(registry.inner(), &app, server_name).await?;
    let tools = registry
        .list_tools(server_name)
        .await
        .map_err(|error| error.message)?;
    Ok(McpServerState {
        name: server_name.to_string(),
        status: McpServerStatus::Connected,
        tools,
        protocol_version: None,
        server_info: None,
        capabilities: None,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_list_tools(
    server_name: String,
    registry: tauri::State<'_, McpClientRegistry>,
) -> Result<Vec<McpTool>, String> {
    registry
        .list_tools(&server_name)
        .await
        .map_err(|error| error.message)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_search_tools(
    query: String,
    registry: tauri::State<'_, McpClientRegistry>,
) -> Result<Vec<McpToolSearchResult>, String> {
    registry
        .search_tools(&query)
        .await
        .map_err(|error| error.message)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_get_tool_details(
    server_name: String,
    tool_name: String,
    registry: tauri::State<'_, McpClientRegistry>,
) -> Result<McpTool, String> {
    registry
        .get_tool_details(&server_name, &tool_name)
        .await
        .map_err(|error| error.message)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_reject_tool_approval(
    approval_token: String,
    registry: tauri::State<'_, McpClientRegistry>,
) -> Result<(), String> {
    registry
        .reject_tool_approval(&approval_token)
        .await
        .map_err(|error| error.message)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_cancel_remote_operations(
    server_name: Option<String>,
    registry: tauri::State<'_, McpClientRegistry>,
) -> Result<usize, String> {
    Ok(registry
        .cancel_remote_operations(server_name.as_deref())
        .await)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpChatTurnContext {
    turn_id: String,
    generation_token: String,
    session_id: String,
    agent_id: String,
    provider_id: String,
    model_id: String,
    parent_turn_id: Option<String>,
    root_turn_id: String,
    turn_kind: String,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn mcp_execute_tool(
    server_name: String,
    tool_name: String,
    arguments: Value,
    approval: Option<McpToolApproval>,
    approval_scope_kind: Option<String>,
    turn_context: Option<McpChatTurnContext>,
    registry: tauri::State<'_, McpClientRegistry>,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<McpToolCallResult, String> {
    apple_command_execution::mcp_execute_tool(
        server_name,
        tool_name,
        arguments,
        approval,
        approval_scope_kind,
        turn_context,
        registry,
        persistence,
        app,
    )
    .await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn prepare_system_apple_app_tool_approval(
    tool_name: String,
    arguments: Value,
    registry: tauri::State<'_, McpClientRegistry>,
    app: tauri::AppHandle,
) -> Result<Option<McpToolApprovalRequest>, String> {
    apple_command_execution::prepare_system_apple_app_tool_approval(
        tool_name, arguments, registry, app,
    )
    .await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn execute_system_apple_app_tool(
    tool_name: String,
    arguments: Value,
    approval: Option<McpToolApproval>,
    turn_context: Option<McpChatTurnContext>,
    registry: tauri::State<'_, McpClientRegistry>,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<McpToolCallResult, String> {
    apple_command_execution::execute_system_apple_app_tool(
        tool_name,
        arguments,
        approval,
        turn_context,
        registry,
        persistence,
        app,
    )
    .await
}

fn validate_direct_system_read_arguments(
    tool_name: &str,
    arguments: &Value,
) -> Result<bool, String> {
    if direct_system_read_display_name(tool_name).is_none() {
        return Ok(false);
    }
    validate_tool_arguments(arguments).map_err(|error| error.message)?;
    match tool_name {
        READ_SYSTEM_CALENDAR_TOOL_NAME => {}
        READ_SYSTEM_PHOTOS_TOOL_NAME => {
            crate::system_photos::photo_limit_from_arguments(arguments)?;
        }
        READ_SYSTEM_CONTACTS_TOOL_NAME => {
            crate::tools::system_contacts::contact_request_from_arguments(arguments)?;
        }
        READ_SYSTEM_MUSIC_TOOL_NAME => {
            crate::system_music::song_limit_from_arguments(arguments)?;
        }
        _ => unreachable!("direct system read names are exhaustively checked"),
    }
    Ok(true)
}

fn direct_system_read_requires_approval(tool_name: &str) -> bool {
    classify_mcp_tool_call(MACOS_APPLESCRIPT_SERVER_NAME, tool_name, None).requires_human_approval()
}
fn direct_system_read_display_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        READ_SYSTEM_CALENDAR_TOOL_NAME => Some("Calendar"),
        READ_SYSTEM_CONTACTS_TOOL_NAME => Some("Contacts"),
        READ_SYSTEM_MUSIC_TOOL_NAME => Some("Music"),
        READ_SYSTEM_PHOTOS_TOOL_NAME => Some("Photos"),
        _ => None,
    }
}

fn bounded_system_calendar_arguments(
    arguments: &Value,
) -> Result<(String, f64, Option<String>, Option<String>), String> {
    let object = arguments
        .as_object()
        .ok_or_else(|| "Calendar arguments must be a JSON object.".to_string())?;
    let text = |snake_case: &str, camel_case: &str| -> Result<Option<String>, String> {
        let Some(value) = object.get(snake_case).or_else(|| object.get(camel_case)) else {
            return Ok(None);
        };
        value
            .as_str()
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| format!("Calendar {camel_case} must be text."))
    };
    let hours_ahead = object
        .get("hours_ahead")
        .or_else(|| object.get("hoursAhead"))
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| "Calendar hoursAhead must be a number.".to_string())
        })
        .transpose()?;
    let calendar_name = bounded_system_calendar_name(text("calendar_name", "calendarName")?);
    let hours_ahead = bounded_system_calendar_hours(hours_ahead);
    let start_date = bounded_system_calendar_datetime_text(text("start_date", "startDate")?)?;
    let end_date = bounded_system_calendar_datetime_text(text("end_date", "endDate")?)?;
    validate_system_calendar_window(start_date.as_deref(), end_date.as_deref())?;
    Ok((calendar_name, hours_ahead, start_date, end_date))
}

async fn read_system_calendar_with_deadline(
    calendar_name: String,
    hours_ahead: f64,
    start_date: Option<String>,
    end_date: Option<String>,
    registry: &McpClientRegistry,
    app: &tauri::AppHandle,
) -> Result<McpToolCallResult, String> {
    read_system_calendar_inner(
        calendar_name,
        hours_ahead,
        start_date,
        end_date,
        registry,
        app,
    )
    .await
}

pub(crate) async fn read_system_calendar_for_workflow(
    arguments: Value,
    registry: Option<McpClientRegistry>,
    app: Option<tauri::AppHandle>,
) -> Result<McpToolCallResult, String> {
    validate_tool_arguments(&arguments).map_err(|error| error.message)?;
    let (calendar_name, hours_ahead, start_date, end_date) =
        bounded_system_calendar_arguments(&arguments)?;

    if let (Some(registry), Some(app)) = (registry.as_ref(), app.as_ref()) {
        return read_system_calendar_with_deadline(
            calendar_name,
            hours_ahead,
            start_date,
            end_date,
            registry,
            app,
        )
        .await;
    }

    let first_attempt = execute_eventkit_calendar_read(
        &calendar_name,
        hours_ahead,
        start_date.as_deref(),
        end_date.as_deref(),
    )
    .await;
    let result = match first_attempt {
        Err(failure) if failure.retryable && failure.code == "calendar_read_failed" => {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            execute_eventkit_calendar_read(
                &calendar_name,
                hours_ahead,
                start_date.as_deref(),
                end_date.as_deref(),
            )
            .await
        }
        result => result,
    };
    Ok(match result {
        Ok(result) => result,
        Err(failure) => calendar_error_result(&failure),
    })
}

async fn read_system_calendar_inner(
    calendar_name: String,
    hours_ahead: f64,
    start_date: Option<String>,
    end_date: Option<String>,
    registry: &McpClientRegistry,
    app: &tauri::AppHandle,
) -> Result<McpToolCallResult, String> {
    let native_failure = match execute_eventkit_calendar_read(
        &calendar_name,
        hours_ahead,
        start_date.as_deref(),
        end_date.as_deref(),
    )
    .await
    {
        Ok(result) => return Ok(result),
        Err(failure) if !calendar_failure_allows_applescript_fallback(&failure) => {
            return Ok(calendar_error_result(&failure));
        }
        Err(failure) => failure,
    };

    let mut arguments = serde_json::Map::new();
    arguments.insert("calendar_name".to_string(), Value::String(calendar_name));
    arguments.insert("hours_ahead".to_string(), serde_json::json!(hours_ahead));
    if let Some(start_date) = start_date {
        arguments.insert("start_date".to_string(), Value::String(start_date));
    }
    if let Some(end_date) = end_date {
        arguments.insert("end_date".to_string(), Value::String(end_date));
    }

    let server_prepared = tokio::time::timeout(
        std::time::Duration::from_secs(SYSTEM_CALENDAR_MCP_PREPARATION_TIMEOUT_SECONDS),
        ensure_trusted_builtin_mcp_server(registry, app, MACOS_APPLESCRIPT_SERVER_NAME),
    )
    .await;
    if !matches!(server_prepared, Ok(Ok(()))) {
        return Ok(calendar_fallback_error_result(
            &native_failure,
            calendar_applescript_stable_failure("calendar_applescript_unavailable"),
        ));
    }
    let tool_prepared = tokio::time::timeout(
        std::time::Duration::from_secs(SYSTEM_CALENDAR_MCP_PREPARATION_TIMEOUT_SECONDS),
        registry.get_tool_details(
            MACOS_APPLESCRIPT_SERVER_NAME,
            READ_SYSTEM_CALENDAR_TOOL_NAME,
        ),
    )
    .await;
    if !matches!(tool_prepared, Ok(Ok(_))) {
        return Ok(calendar_fallback_error_result(
            &native_failure,
            calendar_applescript_stable_failure("calendar_applescript_unavailable"),
        ));
    }

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(SYSTEM_CALENDAR_FALLBACK_TIMEOUT_SECONDS),
        registry.execute_tool(
            MACOS_APPLESCRIPT_SERVER_NAME,
            READ_SYSTEM_CALENDAR_TOOL_NAME,
            Value::Object(arguments),
        ),
    )
    .await
    .map_err(|_| "calendar_applescript_timeout".to_string())
    .and_then(|result| result.map_err(|error| error.message));
    Ok(match result {
        Ok(result) => decorate_calendar_fallback_result(result, &native_failure),
        Err(code) => calendar_fallback_error_result(
            &native_failure,
            calendar_applescript_stable_failure(if code == "calendar_applescript_timeout" {
                "calendar_applescript_timeout"
            } else {
                "calendar_applescript_failed"
            }),
        ),
    })
}

async fn execute_eventkit_calendar_read(
    calendar_name: &str,
    hours_ahead: f64,
    start_date: Option<&str>,
    end_date: Option<&str>,
) -> Result<McpToolCallResult, NativeCalendarFailure> {
    let now = chrono::Utc::now().timestamp_millis() as f64 / 1_000.0;
    let start_timestamp = start_date
        .map(system_calendar_timestamp)
        .transpose()?
        .unwrap_or(now);
    let end_timestamp = end_date
        .map(system_calendar_timestamp)
        .transpose()?
        .unwrap_or(start_timestamp + hours_ahead * 3_600.0);
    if end_timestamp < start_timestamp
        || end_timestamp - start_timestamp > MAX_SYSTEM_CALENDAR_HOURS_AHEAD * 3_600.0
    {
        return Err(NativeCalendarFailure {
            code: "calendar_invalid_window".to_string(),
            message: "Calendar end date must follow its start date within the allowed range."
                .to_string(),
            retryable: false,
        });
    }

    let result = crate::tools::eventkit_calendar::read_calendar(
        crate::tools::eventkit_calendar::CalendarReadRequest {
            calendar_name: calendar_name.to_string(),
            start_timestamp,
            end_timestamp,
        },
    )
    .await
    .map_err(|failure| {
        log_calendar_operation_receipt(failure.receipt.as_ref());
        NativeCalendarFailure {
            code: failure.code,
            message: failure.message,
            retryable: failure.retryable,
        }
    })?;
    log_calendar_operation_receipt(result.receipt.as_ref());
    let payload = serde_json::json!({
        "ok": true,
        "backend": "eventkit",
        "code": "calendar_read_ok",
        "calendarName": result.calendar_name,
        "window": result.window,
        "events": result.events,
        "returnedCount": result.returned_count,
        "matchedCount": result.matched_count,
        "truncated": result.truncated,
        "receipt": result.receipt,
    });
    let encoded = serde_json::to_vec(&payload).map_err(|_| NativeCalendarFailure {
        code: "calendar_response_encode_failed".to_string(),
        message: "Calendar response could not be encoded.".to_string(),
        retryable: true,
    })?;
    parse_eventkit_calendar_response(&encoded, true)
}

fn log_calendar_operation_receipt(
    receipt: Option<&crate::tools::eventkit_calendar::CalendarOperationReceipt>,
) {
    if let Some(receipt) = receipt.and_then(|value| serde_json::to_string(value).ok()) {
        eprintln!("OOMU_CALENDAR_OPERATION_RECEIPT {receipt}");
    }
}

fn system_calendar_timestamp(value: &str) -> Result<f64, NativeCalendarFailure> {
    use chrono::TimeZone;

    let timestamp_millis = if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(value) {
        parsed.timestamp_millis()
    } else {
        let naive = parse_system_calendar_datetime(value).map_err(|_| NativeCalendarFailure {
            code: "calendar_invalid_window".to_string(),
            message: "Calendar date values are invalid.".to_string(),
            retryable: false,
        })?;
        chrono::Local
            .from_local_datetime(&naive)
            .earliest()
            .ok_or_else(|| NativeCalendarFailure {
                code: "calendar_invalid_window".to_string(),
                message: "Calendar date values are invalid in the current time zone.".to_string(),
                retryable: false,
            })?
            .timestamp_millis()
    };
    Ok(timestamp_millis as f64 / 1_000.0)
}

fn parse_eventkit_calendar_response(
    output: &[u8],
    process_succeeded: bool,
) -> Result<McpToolCallResult, NativeCalendarFailure> {
    let payload: Value = serde_json::from_slice(output).map_err(|_| NativeCalendarFailure {
        code: "calendar_native_invalid_response".to_string(),
        message: "The native Calendar reader returned an invalid response.".to_string(),
        retryable: true,
    })?;
    let object = payload.as_object().ok_or_else(|| NativeCalendarFailure {
        code: "calendar_native_invalid_response".to_string(),
        message: "The native Calendar reader returned an invalid response.".to_string(),
        retryable: true,
    })?;
    let code = object
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("calendar_native_invalid_response")
        .to_string();
    if !process_succeeded || object.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(NativeCalendarFailure {
            message: object
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("The native Calendar read failed.")
                .to_string(),
            retryable: object
                .get("retryable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            code,
        });
    }
    if object.get("backend").and_then(Value::as_str) != Some("eventkit")
        || code != "calendar_read_ok"
    {
        return Err(NativeCalendarFailure {
            code: "calendar_native_invalid_response".to_string(),
            message: "The native Calendar reader returned an invalid response.".to_string(),
            retryable: true,
        });
    }
    let events = object
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| NativeCalendarFailure {
            code: "calendar_native_invalid_response".to_string(),
            message: "The native Calendar reader returned an invalid response.".to_string(),
            retryable: true,
        })?;
    if events.len() > MAX_SYSTEM_CALENDAR_EVENTS {
        return Err(NativeCalendarFailure {
            code: "calendar_native_invalid_response".to_string(),
            message: "The native Calendar reader exceeded its event limit.".to_string(),
            retryable: true,
        });
    }
    let window = object.get("window").and_then(Value::as_object);
    let time_zone = window
        .and_then(|value| value.get("timeZone"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| NativeCalendarFailure {
            code: "calendar_native_invalid_response".to_string(),
            message: "The native Calendar reader omitted its timezone.".to_string(),
            retryable: true,
        })?;
    let returned_count = object
        .get("returnedCount")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| NativeCalendarFailure {
            code: "calendar_native_invalid_response".to_string(),
            message: "The native Calendar reader omitted its result count.".to_string(),
            retryable: true,
        })?;
    let matched_count = object
        .get("matchedCount")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| NativeCalendarFailure {
            code: "calendar_native_invalid_response".to_string(),
            message: "The native Calendar reader omitted its match count.".to_string(),
            retryable: true,
        })?;
    let truncated = object
        .get("truncated")
        .and_then(Value::as_bool)
        .ok_or_else(|| NativeCalendarFailure {
            code: "calendar_native_invalid_response".to_string(),
            message: "The native Calendar reader omitted its truncation state.".to_string(),
            retryable: true,
        })?;
    if returned_count != events.len()
        || matched_count < returned_count
        || truncated != (matched_count > returned_count)
    {
        return Err(NativeCalendarFailure {
            code: "calendar_native_invalid_response".to_string(),
            message: "The native Calendar reader returned inconsistent result counts.".to_string(),
            retryable: true,
        });
    }
    let structured = serde_json::json!({
        "backend": "eventkit",
        "code": "calendar_read_ok",
        "calendarName": object.get("calendarName").and_then(Value::as_str).unwrap_or(""),
        "startDate": window.and_then(|value| value.get("startDate")).and_then(Value::as_str).unwrap_or(""),
        "endDate": window.and_then(|value| value.get("endDate")).and_then(Value::as_str).unwrap_or(""),
        "timeZone": time_zone,
        "events": events,
        "returnedCount": returned_count,
        "matchedCount": matched_count,
        "truncated": truncated,
        "receipt": object.get("receipt").cloned().unwrap_or(Value::Null),
    });
    Ok(McpToolCallResult {
        content: vec![serde_json::json!({
            "type": "text",
            "text": serde_json::to_string_pretty(events).unwrap_or_else(|_| "[]".to_string()),
        })],
        structured_content: Some(structured),
        is_error: false,
        meta: None,
        raw: None,
    })
}

fn calendar_failure_allows_applescript_fallback(failure: &NativeCalendarFailure) -> bool {
    cfg!(target_os = "macos") && failure.retryable && failure.code == "calendar_read_failed"
}

fn calendar_error_result(failure: &NativeCalendarFailure) -> McpToolCallResult {
    let structured = serde_json::json!({
        "backend": "eventkit",
        "code": failure.code,
        "message": failure.message,
        "retryable": failure.retryable,
        "events": [],
    });
    McpToolCallResult {
        content: vec![serde_json::json!({"type": "text", "text": failure.message})],
        structured_content: Some(structured),
        is_error: true,
        meta: None,
        raw: None,
    }
}

fn decorate_calendar_fallback_result(
    mut result: McpToolCallResult,
    native_failure: &NativeCalendarFailure,
) -> McpToolCallResult {
    if result.is_error {
        let failure = calendar_applescript_failure(&result);
        return calendar_fallback_error_result(native_failure, failure);
    }
    let mut structured = result
        .structured_content
        .take()
        .unwrap_or_else(|| serde_json::json!({"events": []}));
    if let Some(object) = structured.as_object_mut() {
        let returned_count = object
            .get("events")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default();
        object.insert(
            "backend".to_string(),
            Value::String("applescript".to_string()),
        );
        object.insert(
            "code".to_string(),
            Value::String("calendar_read_fallback".to_string()),
        );
        object.insert(
            "fallbackFrom".to_string(),
            Value::String(native_failure.code.clone()),
        );
        object.insert(
            "returnedCount".to_string(),
            serde_json::json!(returned_count),
        );
        object.insert(
            "matchedCount".to_string(),
            serde_json::json!(returned_count),
        );
        object.insert("truncated".to_string(), Value::Bool(false));
    }
    result.structured_content = Some(structured);
    result.raw = None;
    result
}

fn calendar_fallback_error_result(
    native_failure: &NativeCalendarFailure,
    fallback_failure: NativeCalendarFailure,
) -> McpToolCallResult {
    let mut result = calendar_error_result(&fallback_failure);
    if let Some(object) = result
        .structured_content
        .as_mut()
        .and_then(Value::as_object_mut)
    {
        object.insert(
            "backend".to_string(),
            Value::String("eventkit+applescript".to_string()),
        );
        object.insert(
            "primaryCode".to_string(),
            Value::String(native_failure.code.clone()),
        );
    }
    result
}

fn calendar_applescript_failure(result: &McpToolCallResult) -> NativeCalendarFailure {
    let structured = result.structured_content.as_ref();
    let warning = structured
        .and_then(|value| value.get("warning"))
        .and_then(Value::as_str);
    let error_type = structured
        .and_then(|value| value.get("error_type"))
        .and_then(Value::as_str);
    if warning == Some("timeout") || error_type == Some("timeout") {
        return calendar_applescript_stable_failure("calendar_applescript_timeout");
    }
    let status = structured
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str);
    if status == Some("permission_blocked_or_timed_out") {
        return calendar_applescript_stable_failure("calendar_applescript_permission_or_timeout");
    }
    calendar_applescript_stable_failure("calendar_applescript_failed")
}

fn calendar_applescript_stable_failure(code: &str) -> NativeCalendarFailure {
    let (message, retryable) = match code {
        "calendar_applescript_timeout" => ("Calendar fallback timed out.", true),
        "calendar_applescript_permission_or_timeout" => (
            "Calendar fallback was blocked by macOS permission or timed out.",
            true,
        ),
        "calendar_applescript_unavailable" => ("Calendar fallback is unavailable.", true),
        _ => ("Calendar fallback failed.", true),
    };
    NativeCalendarFailure {
        code: code.to_string(),
        message: message.to_string(),
        retryable,
    }
}

async fn read_system_contacts_with_fallback(
    request: crate::tools::system_contacts::ContactReadRequest,
    registry: &McpClientRegistry,
    app: &tauri::AppHandle,
) -> Result<McpToolCallResult, String> {
    let primary_failure = match crate::tools::system_contacts::read_contacts(request.clone()).await
    {
        Ok(result) => return Ok(result),
        Err(failure) if !crate::tools::system_contacts::allows_applescript_fallback(&failure) => {
            return Ok(crate::tools::system_contacts::contacts_error_result(
                &failure,
            ));
        }
        Err(failure) => failure,
    };

    let fallback_unavailable = || McpToolCallResult {
        content: Vec::new(),
        structured_content: Some(serde_json::json!({
            "code": "contacts_applescript_unavailable",
            "contacts": [],
        })),
        is_error: true,
        meta: None,
        raw: None,
    };
    let fallback =
        if ensure_trusted_builtin_mcp_server(registry, app, MACOS_APPLESCRIPT_SERVER_NAME)
            .await
            .is_err()
            || registry
                .get_tool_details(
                    MACOS_APPLESCRIPT_SERVER_NAME,
                    READ_SYSTEM_CONTACTS_TOOL_NAME,
                )
                .await
                .is_err()
        {
            fallback_unavailable()
        } else {
            registry
                .execute_tool(
                    MACOS_APPLESCRIPT_SERVER_NAME,
                    READ_SYSTEM_CONTACTS_TOOL_NAME,
                    serde_json::json!({
                        "max_contacts": request.max_contacts,
                        "search_text": request.search_text.clone(),
                    }),
                )
                .await
                .unwrap_or_else(|_| fallback_unavailable())
        };
    Ok(
        crate::tools::system_contacts::contacts_applescript_fallback_result(
            &primary_failure,
            fallback,
            &request,
        ),
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn read_system_emails(
    max_messages: Option<u32>,
    unread_only: Option<bool>,
    turn_context: Option<McpChatTurnContext>,
    registry: tauri::State<'_, McpClientRegistry>,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<McpToolCallResult, String> {
    let turn_context = turn_context.map(ChatTurnPersistenceContext::from);
    let arguments = serde_json::json!({ "max_messages": max_messages, "unread_only": unread_only });
    let receipt = native_apple_receipts::spec_for(
        MACOS_APPLESCRIPT_SERVER_NAME,
        READ_SYSTEM_EMAILS_TOOL_NAME,
        &arguments,
    );
    native_apple_receipts::execute(
        receipt,
        turn_context.as_ref(),
        persistence.inner(),
        true,
        system_mail::execute_turn_bound_mail_read(
            arguments,
            turn_context.as_ref(),
            registry.inner(),
            persistence.inner(),
            &app,
        ),
    )
    .await
}

fn normalize_system_apple_app_tool_name(tool_name: &str) -> Result<String, String> {
    let normalized = tool_name.trim().to_ascii_lowercase();
    if SYSTEM_APP_READ_TOOL_NAMES.contains(&normalized.as_str())
        || SYSTEM_APP_MUTATING_TOOL_NAMES.contains(&normalized.as_str())
    {
        return Ok(normalized);
    }
    Err(format!(
        "Apple system app tool \"{tool_name}\" is not allowed for direct chat execution."
    ))
}

fn bounded_system_calendar_name(calendar_name: Option<String>) -> String {
    calendar_name
        .unwrap_or_default()
        .trim()
        .chars()
        .take(MAX_SYSTEM_CALENDAR_NAME_CHARS)
        .collect()
}

fn bounded_system_calendar_hours(hours_ahead: Option<f64>) -> f64 {
    let value = hours_ahead.unwrap_or(DEFAULT_SYSTEM_CALENDAR_HOURS_AHEAD);
    if !value.is_finite() {
        return DEFAULT_SYSTEM_CALENDAR_HOURS_AHEAD;
    }
    value
        .max(MIN_SYSTEM_CALENDAR_HOURS_AHEAD)
        .min(MAX_SYSTEM_CALENDAR_HOURS_AHEAD)
}

fn bounded_system_calendar_datetime_text(value: Option<String>) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > MAX_SYSTEM_CALENDAR_DATE_CHARS {
        return Err("Calendar date values must be at most 64 characters.".to_string());
    }
    parse_system_calendar_datetime(trimmed)?;
    Ok(Some(trimmed.to_string()))
}

fn validate_system_calendar_window(
    start_date: Option<&str>,
    end_date: Option<&str>,
) -> Result<(), String> {
    let (Some(start_date), Some(end_date)) = (start_date, end_date) else {
        return Ok(());
    };
    let start = parse_system_calendar_datetime(start_date)?;
    let end = parse_system_calendar_datetime(end_date)?;
    if end < start {
        return Err("Calendar endDate must be after startDate.".to_string());
    }
    if end - start > ChronoDuration::hours(MAX_SYSTEM_CALENDAR_HOURS_AHEAD as i64) {
        return Err(format!(
            "Calendar read window must be {} hours or less.",
            MAX_SYSTEM_CALENDAR_HOURS_AHEAD as i64
        ));
    }
    Ok(())
}

fn parse_system_calendar_datetime(value: &str) -> Result<NaiveDateTime, String> {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(parsed.naive_utc());
    }

    for format in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(value, format) {
            return Ok(parsed);
        }
    }

    Err("Calendar date values must use ISO 8601, for example 2026-06-16T09:00:00.".to_string())
}

async fn ensure_trusted_builtin_mcp_server(
    registry: &McpClientRegistry,
    app: &tauri::AppHandle,
    server_name: &str,
) -> Result<(), String> {
    if registry.list_tools(server_name).await.is_ok()
        && registry
            .has_active_trusted_builtin_session(server_name)
            .await
    {
        return Ok(());
    }

    let report = crate::mcp::bootstrap::bootstrap_mcp_runtime(app)?;
    let config = report
        .server_configs
        .into_iter()
        .find(|candidate| candidate.name == server_name)
        .ok_or_else(|| format!("Built-in MCP server \"{server_name}\" is unavailable."))?;

    registry
        .register_trusted_server_configs([config.clone()])
        .await;
    let authorization = McpSpawnAuthorization::trusted_internal(&config);
    registry
        .connect_server_with_authorization(config, authorization)
        .await
        .map(|_| ())
        .map_err(|error| error.message)
}

impl McpClientError {
    pub(crate) fn transport(message: String) -> Self {
        Self {
            code: "mcp_transport_error",
            boundary: "mcp_client",
            message,
        }
    }

    pub(crate) fn protocol(message: String) -> Self {
        Self {
            code: "mcp_protocol_error",
            boundary: "mcp_client",
            message,
        }
    }

    pub(crate) fn permission(message: String) -> Self {
        Self {
            code: "mcp_permission_required",
            boundary: "mcp_permission_gateway",
            message,
        }
    }

    pub(crate) fn cancelled(message: String) -> Self {
        Self {
            code: "mcp_cancelled",
            boundary: "mcp_network_boundary",
            message,
        }
    }
}

impl fmt::Display for McpClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for McpClientError {}

fn prepare_tool_arguments(arguments: serde_json::Value) -> serde_json::Value {
    arguments
}

fn generate_tool_approval_token() -> String {
    let mut buffer = vec![0_u8; MCP_TOOL_APPROVAL_TOKEN_BYTES];
    OsRng.fill_bytes(&mut buffer);
    hex::encode(buffer)
}

fn generate_mcp_audit_id() -> String {
    let mut buffer = [0_u8; 16];
    OsRng.fill_bytes(&mut buffer);
    format!("mcp-audit-{}", hex::encode(buffer))
}

fn argument_binding(arguments: &Value) -> String {
    keyed_json_binding(b"OOMU_MCP_ARGUMENT_BINDING_V1\0", arguments)
}

fn workflow_argument_binding(arguments: &Value) -> String {
    stable_workflow_review_binding(b"OOMU_MCP_WORKFLOW_ARGUMENT_REVIEW_V1\0", arguments)
}

fn server_identity_binding(identity: &Value) -> String {
    keyed_json_binding(b"OOMU_MCP_SERVER_IDENTITY_BINDING_V1\0", identity)
}

fn workflow_server_identity_binding(identity: &Value) -> String {
    stable_workflow_review_binding(b"OOMU_MCP_WORKFLOW_SERVER_REVIEW_V1\0", identity)
}

fn tool_definition_binding(tool: &McpTool) -> String {
    keyed_json_binding(
        b"OOMU_MCP_TOOL_DEFINITION_BINDING_V1\0",
        &serde_json::json!({
            "name": tool.name,
            "description": tool.description,
            "inputSchema": tool.input_schema,
            "outputSchema": tool.output_schema,
            "annotations": tool.annotations,
            "meta": tool.meta,
        }),
    )
}

fn workflow_tool_definition_binding(tool: &McpTool) -> String {
    stable_workflow_review_binding(
        b"OOMU_MCP_WORKFLOW_TOOL_REVIEW_V1\0",
        &serde_json::json!({
            "name": tool.name,
            "description": tool.description,
            "inputSchema": tool.input_schema,
            "outputSchema": tool.output_schema,
            "annotations": tool.annotations,
            "meta": tool.meta,
        }),
    )
}

fn validate_tool_arguments(arguments: &Value) -> Result<(), McpClientError> {
    validate_json_structure(arguments)?;
    let byte_count = serde_json::to_vec(arguments)
        .map_err(|error| {
            McpClientError::protocol(format!("MCP tool arguments could not be measured: {error}"))
        })?
        .len();
    if byte_count > client_sse::REMOTE_REQUEST_BYTE_LIMIT {
        return Err(McpClientError::protocol(format!(
            "MCP tool arguments exceeded the {} byte outbound request limit.",
            client_sse::REMOTE_REQUEST_BYTE_LIMIT
        )));
    }
    Ok(())
}

fn prune_expired_tool_approvals(
    approvals: &mut HashMap<String, PendingMcpToolApproval>,
    now_ms: u64,
) {
    approvals.retain(|_, approval| approval.request.expires_at_ms >= now_ms);
}

fn validate_json_rpc_error_object(error: &Value) -> Result<(), McpClientError> {
    let error = error.as_object().ok_or_else(|| {
        McpClientError::protocol(
            "JSON-RPC error responses must contain an error object.".to_string(),
        )
    })?;
    let code_is_integer = error
        .get("code")
        .is_some_and(|code| code.as_i64().is_some() || code.as_u64().is_some());
    let message_is_string = error.get("message").and_then(Value::as_str).is_some();
    if !code_is_integer || !message_is_string {
        return Err(McpClientError::protocol(
            "JSON-RPC error responses require an integer code and string message.".to_string(),
        ));
    }
    Ok(())
}

pub fn parse_json_rpc_message(line: &str) -> Result<JsonRpcMessage, McpClientError> {
    let value: Value = serde_json::from_str(line)
        .map_err(|error| McpClientError::protocol(format!("Invalid JSON-RPC payload: {error}")))?;
    validate_json_structure(&value)?;
    let object = value.as_object().ok_or_else(|| {
        McpClientError::protocol("JSON-RPC payload must be an object.".to_string())
    })?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(McpClientError::protocol(
            "JSON-RPC payload must declare jsonrpc \"2.0\".".to_string(),
        ));
    }

    let has_method = object.contains_key("method");
    let has_id = object.contains_key("id");
    let has_result = object.contains_key("result");
    let has_error = object.contains_key("error");
    if !has_method && has_id && !has_result && has_error {
        validate_json_rpc_error_object(
            object
                .get("error")
                .expect("error presence was checked before validation"),
        )?;
    }

    match (has_method, has_id, has_result, has_error) {
        (true, true, false, false) => {
            let request = serde_json::from_value::<JsonRpcRequest>(value)
                .map_err(|error| McpClientError::protocol(format!("Invalid request: {error}")))?;
            validate_json_rpc_request(&request)?;
            Ok(JsonRpcMessage::Request(request))
        }
        (true, false, false, false) => {
            let notification =
                serde_json::from_value::<JsonRpcNotification>(value).map_err(|error| {
                    McpClientError::protocol(format!("Invalid notification: {error}"))
                })?;
            validate_json_rpc_method(&notification.method)?;
            Ok(JsonRpcMessage::Notification(notification))
        }
        (false, true, true, false) | (false, true, false, true) => {
            let response = serde_json::from_value::<JsonRpcResponse>(value)
                .map_err(|error| McpClientError::protocol(format!("Invalid response: {error}")))?;
            request_id_key(&response.id)?;
            Ok(JsonRpcMessage::Response(response))
        }
        _ => Err(McpClientError::protocol(
            "JSON-RPC payload shape is not a valid request, response, or notification.".to_string(),
        )),
    }
}

#[derive(Default)]
struct JsonStructureCounts {
    nodes: usize,
    container_entries: usize,
}

fn validate_json_structure(value: &Value) -> Result<(), McpClientError> {
    let mut counts = JsonStructureCounts::default();
    validate_json_structure_at_depth(value, 0, &mut counts)
}

fn validate_json_structure_at_depth(
    value: &Value,
    depth: usize,
    counts: &mut JsonStructureCounts,
) -> Result<(), McpClientError> {
    if depth > MCP_MAX_JSON_DEPTH {
        return Err(McpClientError::protocol(format!(
            "MCP JSON exceeded the maximum structure depth of {MCP_MAX_JSON_DEPTH}."
        )));
    }
    counts.nodes = counts.nodes.saturating_add(1);
    if counts.nodes > MCP_MAX_JSON_NODES {
        return Err(McpClientError::protocol(format!(
            "MCP JSON exceeded the maximum node count of {MCP_MAX_JSON_NODES}."
        )));
    }
    match value {
        Value::String(value) if value.len() > MCP_MAX_JSON_STRING_BYTES => {
            Err(McpClientError::protocol(format!(
                "MCP JSON string exceeded the {} byte field limit.",
                MCP_MAX_JSON_STRING_BYTES
            )))
        }
        Value::Array(values) => {
            counts.container_entries = counts.container_entries.saturating_add(values.len());
            if counts.container_entries > MCP_MAX_JSON_CONTAINER_ENTRIES {
                return Err(McpClientError::protocol(format!(
                    "MCP JSON exceeded the maximum object/array entry count of {MCP_MAX_JSON_CONTAINER_ENTRIES}."
                )));
            }
            for value in values {
                validate_json_structure_at_depth(value, depth + 1, counts)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            counts.container_entries = counts.container_entries.saturating_add(values.len());
            if counts.container_entries > MCP_MAX_JSON_CONTAINER_ENTRIES {
                return Err(McpClientError::protocol(format!(
                    "MCP JSON exceeded the maximum object/array entry count of {MCP_MAX_JSON_CONTAINER_ENTRIES}."
                )));
            }
            for (key, value) in values {
                if key.len() > MCP_MAX_JSON_STRING_BYTES {
                    return Err(McpClientError::protocol(format!(
                        "MCP JSON field name exceeded the {} byte limit.",
                        MCP_MAX_JSON_STRING_BYTES
                    )));
                }
                validate_json_structure_at_depth(value, depth + 1, counts)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn spawn_writer_loop(
    server_name: String,
    transport: McpTransportConfig,
    mut stdin: tokio::process::ChildStdin,
    mut rx: mpsc::Receiver<JsonRpcOutboundMessage>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<JsonRpcResponse, McpClientError>>>>>,
) {
    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            let id_key = match &message {
                JsonRpcOutboundMessage::Request(request) => match request_id_key(&request.id) {
                    Ok(id_key) => Some(id_key),
                    Err(error) => {
                        continue_or_fail_pending(&pending, &request.id, error).await;
                        continue;
                    }
                },
                JsonRpcOutboundMessage::Notification(_) => None,
            };
            let payload = match serialize_outbound_message(&message) {
                Ok(payload) => payload,
                Err(error) => {
                    handle_outbound_failure(&server_name, &pending, id_key.as_deref(), error).await;
                    continue;
                }
            };
            let payload = match sanitize_outgoing_payload_for_transport(&payload, &transport) {
                Ok(payload) => payload,
                Err(error) => {
                    handle_outbound_failure(
                        &server_name,
                        &pending,
                        id_key.as_deref(),
                        McpClientError::transport(format!(
                            "MCP request for '{server_name}' failed routing shield checks: {error}"
                        )),
                    )
                    .await;
                    continue;
                }
            };

            if let Err(error) = stdin.write_all(payload.as_bytes()).await {
                handle_outbound_failure(
                    &server_name,
                    &pending,
                    id_key.as_deref(),
                    McpClientError::transport(format!(
                        "Failed to write MCP request to '{server_name}': {error}"
                    )),
                )
                .await;
                continue;
            }
            if let Err(error) = stdin.write_all(b"\n").await {
                handle_outbound_failure(
                    &server_name,
                    &pending,
                    id_key.as_deref(),
                    McpClientError::transport(format!(
                        "Failed to finish MCP request line for '{server_name}': {error}"
                    )),
                )
                .await;
                continue;
            }
            if let Err(error) = stdin.flush().await {
                handle_outbound_failure(
                    &server_name,
                    &pending,
                    id_key.as_deref(),
                    McpClientError::transport(format!(
                        "Failed to flush MCP request to '{server_name}': {error}"
                    )),
                )
                .await;
            }
        }
    });
}

fn spawn_reader_loop(
    server_name: String,
    stdout: tokio::process::ChildStdout,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<JsonRpcResponse, McpClientError>>>>>,
    tools_stale: Arc<Mutex<bool>>,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => match parse_json_rpc_message(&line) {
                    Ok(JsonRpcMessage::Response(response)) => {
                        if let Ok(id_key) = request_id_key(&response.id) {
                            if let Some(sender) = pending.lock().await.remove(&id_key) {
                                let _ = sender.send(Ok(response));
                            }
                        }
                    }
                    Ok(JsonRpcMessage::Notification(notification)) => {
                        if notification.method == "notifications/tools/list_changed" {
                            *tools_stale.lock().await = true;
                            log_mcp_stderr_line(
                                &server_name,
                                "tools/list_changed notification received; tool catalog marked stale",
                            )
                            .await;
                        }
                    }
                    Ok(JsonRpcMessage::Request(_)) => {}
                    Err(error) => {
                        log_mcp_stderr_line(
                            &server_name,
                            &format!("protocol parse error on stdout: {}", error.message),
                        )
                        .await;
                    }
                },
                Ok(None) => {
                    fail_all_pending(
                        &pending,
                        McpClientError::transport(format!(
                            "MCP server '{server_name}' closed stdout."
                        )),
                    )
                    .await;
                    break;
                }
                Err(error) => {
                    fail_all_pending(
                        &pending,
                        McpClientError::transport(format!(
                            "Failed to read MCP stdout from '{server_name}': {error}"
                        )),
                    )
                    .await;
                    break;
                }
            }
        }
    });
}

fn spawn_stderr_log_loop(server_name: String, stderr: tokio::process::ChildStderr) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log_mcp_stderr_line(&server_name, &line).await;
        }
    });
}

fn serialize_outbound_message(message: &JsonRpcOutboundMessage) -> Result<String, McpClientError> {
    match message {
        JsonRpcOutboundMessage::Request(request) => {
            serde_json::to_string(request).map_err(|error| {
                McpClientError::protocol(format!("Failed to serialize MCP request: {error}"))
            })
        }
        JsonRpcOutboundMessage::Notification(notification) => serde_json::to_string(notification)
            .map_err(|error| {
                McpClientError::protocol(format!("Failed to serialize MCP notification: {error}"))
            }),
    }
}

async fn handle_outbound_failure(
    server_name: &str,
    pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Result<JsonRpcResponse, McpClientError>>>>>,
    id_key: Option<&str>,
    error: McpClientError,
) {
    if let Some(id_key) = id_key {
        fail_pending(pending, id_key, error).await;
    } else {
        log_mcp_stderr_line(server_name, &error.message).await;
    }
}

async fn continue_or_fail_pending(
    pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Result<JsonRpcResponse, McpClientError>>>>>,
    id: &Value,
    error: McpClientError,
) {
    if let Ok(id_key) = request_id_key(id) {
        fail_pending(pending, &id_key, error).await;
    }
}

async fn fail_pending(
    pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Result<JsonRpcResponse, McpClientError>>>>>,
    id_key: &str,
    error: McpClientError,
) {
    if let Some(sender) = pending.lock().await.remove(id_key) {
        let _ = sender.send(Err(error));
    }
}

async fn fail_all_pending(
    pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Result<JsonRpcResponse, McpClientError>>>>>,
    error: McpClientError,
) {
    let senders = {
        let mut locked = pending.lock().await;
        locked.drain().map(|(_, sender)| sender).collect::<Vec<_>>()
    };
    for sender in senders {
        let _ = sender.send(Err(error.clone()));
    }
}

async fn log_mcp_stderr_line(server_name: &str, line: &str) {
    let path = mcp_stderr_log_path(server_name);
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                tokio::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).await;
        }
    }
    let entry = mcp_stderr_log_entry(server_name, line);
    let mut options = tokio::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let file = options.open(&path).await;
    if let Ok(mut file) = file {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await;
        }
        let _ = file.write_all(entry.as_bytes()).await;
    }
}

fn mcp_stderr_log_entry(server_name: &str, line: &str) -> String {
    let server_name = bounded_log_field(&crate::redaction::redacted_log_text(server_name), 256);
    let shield_redacted = crate::mcp::shield::sanitize_payload(line);
    let line = bounded_log_field(
        &crate::redaction::redacted_log_text(shield_redacted.as_ref()),
        MCP_STDERR_LOG_FIELD_LIMIT,
    );
    format!("{} [{}] {}\n", unix_time_ms(), server_name, line)
}

fn bounded_log_field(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...[truncated]", &value[..end])
}

fn mcp_stderr_log_path(server_name: &str) -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let sanitized = server_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    home.join(MCP_STDERR_LOG_DIR)
        .join(format!("{sanitized}.stderr.log"))
}

fn json_null() -> Value {
    Value::Null
}

fn next_request_id() -> Value {
    Value::String(format!(
        "oomu-mcp-{}",
        MCP_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

fn json_rpc_response_result(response: JsonRpcResponse) -> Result<Value, McpClientError> {
    if let Some(error) = response.error {
        let redacted_error = crate::redaction::redacted_argument_summary(&error);
        return Err(McpClientError::protocol(format!(
            "MCP server returned a redacted JSON-RPC error: {redacted_error}"
        )));
    }

    response.result.ok_or_else(|| {
        McpClientError::protocol("MCP JSON-RPC response did not include a result.".to_string())
    })
}

async fn list_tools_for_session(
    session: &Arc<McpClientSession>,
) -> Result<Vec<McpTool>, McpClientError> {
    let response = session
        .send_request(JsonRpcRequest::new(
            "tools/list",
            serde_json::json!({}),
            next_request_id(),
        ))
        .await?;
    let result = json_rpc_response_result(response)?;
    let tools = parse_tools_list(result)?;
    *session.tools_stale.lock().await = false;
    Ok(tools)
}

pub(crate) mod apple_command_execution;
mod catalog_port;
mod error_classification;
pub(crate) mod native_apple_receipts;
mod native_capability_execution;
mod native_public_search_execution;
mod protocol_validation;
mod public_search_session_approval;
mod shutdown;
pub(crate) mod system_mail;
pub(crate) use catalog_port::install_connected_tool_catalog_port;
use protocol_validation::{
    parse_tool_call_result, parse_tools_list, request_id_key, validate_json_rpc_method,
    validate_json_rpc_request,
};
#[cfg(test)]
mod tests;
