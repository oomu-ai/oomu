use crate::db::{
    normalize_channel_platform, trust_usage_day, ChatTurnPersistenceContext, PersistenceEngine,
    SovereignTrustGrant, SovereignTrustPermissionLevel, SovereignTrustToolCategory,
};
use crate::foundation::{
    clock::{unix_time_ms_i64, unix_time_ms_u128, unix_time_ms_u64},
    digest::sha256_hex,
};
use crate::gateway::GatewayIncomingMessage;
use crate::security::sandbox::SandboxRoot;
use crate::sovereign_identity::{NativeFileAuthorityEnvelope, SignatureBlock, SovereignIdentity};
use crate::tools::{ToolError, ToolRegistry};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    ffi::OsString,
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex as StdMutex, OnceLock},
    thread,
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};
use tokio::sync::Mutex as AsyncMutex;
use zeroize::Zeroizing;
mod action_authorization;
mod actuation_continuation;
#[path = "shield_gate/direct_command_turn_guard.rs"]
mod direct_command_turn_guard;
mod external_file_binding;
pub(crate) mod native_file_authority;
#[path = "shield_gate/native_file_receipt.rs"]
mod native_file_receipt;
mod system_action_semantics;
mod terminal_execution;
mod terminal_runtime;
mod unified_diff;
use direct_command_turn_guard::{prepare_receipt, DirectCommandTurnGuard};
use external_file_binding::{
    approved_chat_read_byte_count, approved_chat_read_mime_type, approved_file_identity,
    inspect_approved_chat_read_target, list_bound_external_directory,
    normalize_directory_read_action, open_bound_external_target,
    prepare_approved_external_read_target, prepare_approved_external_write_target,
    prepare_external_filesystem_binding, required_approved_chat_file_binding,
    resolve_external_action_target, write_bound_external_target_atomically,
};
pub(crate) use external_file_binding::{
    bind_approved_external_directory_creation, bind_approved_external_file_read,
    bind_approved_external_file_write, create_bound_approved_external_directory,
    read_bound_approved_external_file_bounded, reviewed_action_class,
    validate_approved_external_write_target, write_bound_approved_external_file_atomically,
    ApprovedExternalDirectoryBinding, ApprovedExternalFileReadBinding,
    ApprovedExternalFileWriteBinding,
};
use native_file_authority::NativeDirectFileAccessRequest;
use terminal_execution::{
    handle_approved_system_execution, terminal_request_for_action as terminal_request,
};
use unified_diff::apply_unified_diff_directive;
#[cfg(test)]
use unified_diff::{apply_unified_diff_hunk, parse_unified_diff, UnifiedDiffHunk};
pub trait AuthorizedActionBoundary: Sized + Send + Sync + 'static {
    fn operation_name(&self) -> &'static str;
}
#[derive(Debug, Deserialize, Serialize)]
pub struct ExecuteCommandRequest {
    pub action: RequestedAction,
    pub logical_certificate: Option<LogicalCertificate>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub generation_token: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub parent_turn_id: Option<String>,
    #[serde(default)]
    pub root_turn_id: Option<String>,
    #[serde(default)]
    pub turn_kind: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub task_run_id: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovedFileReceiptToken {
    pub payload: String,
    pub signature: SignatureBlock,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareApprovedChatFileRequest {
    pub access: NativeDirectFileAccessRequest,
    pub display_message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareApprovedChatFileResponse {
    pub display_name: String,
    pub mime_type: String,
    pub byte_count: usize,
    pub receipt: ApprovedFileReceiptToken,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovedFileReceiptPayload {
    version: u32,
    receipt_id: String,
    session_id: String,
    issued_turn_id: String,
    root_turn_id: String,
    agent_id: String,
    target_identity_hash: String,
    display_name: String,
    mime_type: String,
    byte_count: usize,
    content_sha256: String,
    content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    media_sha256: Option<String>,
    display_message: String,
    issued_at_ms: i64,
    expires_at_ms: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct VerifiedApprovedFileContext {
    pub display_name: String,
    pub mime_type: String,
    pub byte_count: usize,
    pub content: String,
    pub data_base64: Option<String>,
    pub display_message: String,
}
pub const ACTUATION_LEASE_DECAYED_EVENT: &str = "actuation-lease-decayed";
pub const ACTUATION_LEASE_UPDATED_EVENT: &str = "actuation-lease-updated";
const DEFAULT_SCOPE_TRUST_DURATION_MS: u64 = 15 * 60 * 1_000;
const MAX_APPROVAL_DIFF_BYTES: usize = 16 * 1024;
const MAX_APPROVAL_DIFF_LINES_PER_SIDE: usize = 140;
const MAX_REMOTE_MCP_APPROVAL_PREVIEW_CHARS: usize = 8 * 1024;
const LEGACY_APPROVED_CHAT_FILE_RECEIPT_VERSION: u32 = 1;
const APPROVED_CHAT_FILE_RECEIPT_VERSION: u32 = 2;
const APPROVED_CHAT_FILE_RECEIPT_TTL_MS: i64 = 24 * 60 * 60 * 1_000;
const MAX_APPROVED_CHAT_FILE_CONTEXT_BYTES: usize = 256 * 1024;
const MAX_APPROVED_CHAT_FILE_MEDIA_BYTES: usize = 8 * 1024 * 1024;
const MAX_APPROVED_CHAT_FILE_MEDIA_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_APPROVED_CHAT_FILE_MEDIA_CACHE_ENTRIES: usize = 16;

#[derive(Debug)]
struct ApprovedChatFileMedia {
    session_id: String,
    root_turn_id: String,
    agent_id: String,
    mime_type: String,
    sha256: String,
    issued_at_ms: i64,
    expires_at_ms: i64,
    bytes: Zeroizing<Vec<u8>>,
}

static APPROVED_CHAT_FILE_MEDIA_CACHE: OnceLock<StdMutex<HashMap<String, ApprovedChatFileMedia>>> =
    OnceLock::new();
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActuationLease {
    pub actor_id: String,
    pub session_id: String,
    pub operation_classes: Vec<String>,
    pub canonical_scopes: Vec<String>,
    pub expires_at_ms: u64,
    pub max_steps: usize,
    pub current_steps: usize,
    pub is_active: bool,
}
#[derive(Debug, Clone)]
pub struct ScopeTrustCache {
    pub authorized_prefix: PathBuf,
    pub allowed_operations: Vec<String>,
    pub expires_at_ms: u64,
    pub principal: String,
    pub grant_id: String,
    pub granted_at_ms: u64,
    pub app_session: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionScopeTrustGrant {
    pub grant_id: String,
    pub principal: String,
    pub canonical_resource: String,
    pub action_class: String,
    pub granted_at_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeSessionScopeTrustRequest {
    pub grant_id: String,
}
#[derive(Debug, Clone)]
pub struct ScopeTrustManager {
    cache: Arc<StdMutex<Vec<ScopeTrustCache>>>,
}
impl Default for ScopeTrustManager {
    fn default() -> Self {
        Self {
            cache: Arc::new(StdMutex::new(Vec::new())),
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeTrustApprovalRequest {
    pub enabled: bool,
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub max_uses: Option<u32>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantActuationLeaseRequest {
    pub session_id: String,
    pub duration_ms: u64,
    pub max_steps: usize,
    pub authority_proof_id: String,
    pub operation_classes: Vec<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeActuationLeaseRequest {
    pub session_id: String,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActuationLeaseStatus {
    pub lease: Option<ActuationLease>,
    pub active: bool,
    pub now_ms: u64,
    pub remaining_ms: u64,
    pub remaining_steps: usize,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActuationLeaseDecayEvent {
    pub status: ActuationLeaseStatus,
    pub reason: String,
    pub operation: Option<String>,
    pub session_id: String,
    pub review_preview: Option<String>,
}
#[derive(Debug, Clone)]
pub struct ActuationLeaseManager {
    lease: Arc<StdMutex<Option<ActuationLease>>>,
}
#[derive(Debug, Clone)]
pub(crate) enum ActuationLeaseOutcome {
    NotRequired,
    Authorized(ActuationLeaseStatus),
    Blocked(ActuationLeaseStatus, String),
}
impl Default for ActuationLeaseManager {
    fn default() -> Self {
        Self {
            lease: Arc::new(StdMutex::new(None)),
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurgicalPatchDirectiveRequest {
    pub diff: String,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RequestedAction {
    pub kind: String,
    pub principal: Option<String>,
    pub path: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigureChannelRequest {
    pub platform: String,
    pub credentials_json: String,
    pub owner_id: String,
    pub is_active: bool,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogicalCertificate {
    pub premises: Vec<String>,
    pub execution_path: Vec<String>,
    pub formal_conclusion: String,
    pub signature: Option<SignatureBlock>,
}
pub const REMOTE_LEVEL_THREE_BLOCK_MESSAGE: &str =
    "Action blocked: System-level and shell operations are restricted to direct desktop terminal execution only.";
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayFirewallDecision {
    pub allowed: bool,
    pub reason: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteExecutionTrustLevel {
    AutoApproved,
    RequiresInteractiveConfirmation,
    Blocked,
}
impl RemoteExecutionTrustLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::AutoApproved => "level_1_auto_approved",
            Self::RequiresInteractiveConfirmation => "level_2_requires_confirmation",
            Self::Blocked => "level_3_blocked",
        }
    }
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBlockedAction {
    pub action: RequestedAction,
    pub reason: String,
    pub trust_level: String,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteActionFilterResponse {
    pub auto_approved_actions: Vec<RequestedAction>,
    pub confirmation_required_actions: Vec<RequestedAction>,
    pub blocked_actions: Vec<RemoteBlockedAction>,
    pub response_message: Option<String>,
}
#[derive(Default)]
pub struct ShieldApprovalManager {
    pending: AsyncMutex<HashMap<String, PendingShieldApproval>>,
    prompt_lock: AsyncMutex<()>,
    decisions: crate::authority::shield_decision::NativeShieldDecisionStore,
}

struct PendingShieldApproval {
    request: ShieldApprovalRequest,
    frozen: crate::authority::shield_decision::FrozenShieldRequest,
    display_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShieldApprovalRequest {
    pub approval_token: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub generation_token: Option<String>,
    pub action_type: String,
    pub action_label: String,
    pub target_path: Option<String>,
    pub principal: Option<String>,
    pub risk_tier: String,
    pub reason: String,
    pub estimated_token_costs: Option<usize>,
    pub requested_at_ms: u64,
    pub preview: String,
    pub semantic_summary: String,
    pub semantic_detail: String,
    pub approval_tier: String,
    pub approval_mode: String,
    pub diff_preview: Option<String>,
    pub scope_trust_available: bool,
    pub scope_trust_prefix: Option<String>,
    pub scope_trust_duration_ms: u64,
    pub project_id: Option<String>,
    pub task_run_id: Option<String>,
    pub action_class: String,
    pub argument_class: String,
    pub canonical_resource: Option<String>,
    pub mandatory_reconfirm: bool,
    pub approval_scope_kinds: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShieldApprovalStatus {
    pub display_id: String,
    pub session_id: Option<String>,
    pub action_label: String,
    pub semantic_summary: String,
    pub requested_at_ms: u64,
    pub pending: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShieldApprovalDecision {
    Approve,
    Deny,
}

#[derive(Debug)]
pub enum AuthorizedActions {
    GetSystemMetrics(SystemMetricsRequest),
    FileRead(FileReadRequest),
    ApprovedExternalFileRead(ApprovedExternalReadRequest),
    FileWrite(FileWriteRequest),
    CodebasePatch(CodebasePatchRequest),
    CodebaseCompile(CodebaseCompileRequest),
    ApprovedExternalFileWrite(ApprovedExternalWriteRequest),
    TelemetryArchive(TelemetryArchiveRequest),
    ApprovedFileDelete(FileDeleteRequest),
    FileList(FileListRequest),
    ApprovedExternalFileList(ApprovedExternalReadRequest),
    ApprovedSystemExecution(SystemExecutionRequest),
    SystemAudit(SystemAuditRequest),
    WebFetch,
    DocumentIndex,
    AskLocalDocumentIndex,
    SovereignDuckDuckGoSearch(SovereignDuckDuckGoSearchRequest),
    RegisteredTaskTool(crate::tools::task_tool_runtime::ValidatedTaskToolRequest),
    AirlockExport(AirlockExportRequest),
}

impl AuthorizedActionBoundary for AuthorizedActions {
    fn operation_name(&self) -> &'static str {
        match self {
            AuthorizedActions::GetSystemMetrics(_) => "get_system_metrics",
            AuthorizedActions::FileRead(_) | AuthorizedActions::ApprovedExternalFileRead(_) => {
                "file_read"
            }
            AuthorizedActions::FileWrite(_) | AuthorizedActions::ApprovedExternalFileWrite(_) => {
                "file_write"
            }
            AuthorizedActions::TelemetryArchive(_) => "telemetry_archive",
            AuthorizedActions::ApprovedFileDelete(_) => "delete_file",
            AuthorizedActions::CodebasePatch(_) => "codebase_patch",
            AuthorizedActions::CodebaseCompile(_) => "codebase_compile",
            AuthorizedActions::FileList(_) | AuthorizedActions::ApprovedExternalFileList(_) => {
                "file_list"
            }
            AuthorizedActions::ApprovedSystemExecution(_) => "terminal_execute",
            AuthorizedActions::SystemAudit(_) => "system_audit",
            AuthorizedActions::WebFetch => "web_fetch",
            AuthorizedActions::DocumentIndex => "document_index",
            AuthorizedActions::AskLocalDocumentIndex => "ask_local_document_index",
            AuthorizedActions::SovereignDuckDuckGoSearch(_) => "sovereign_duckduckgo_search",
            AuthorizedActions::RegisteredTaskTool(request) => request.operation,
            AuthorizedActions::AirlockExport(_) => "airlock_export",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SystemMetricsRequest {
    pub principal: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileReadRequest {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileWriteRequest {
    pub path: String,
    pub content: String,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct ApprovedFileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Debug)]
pub struct ApprovedExternalReadRequest {
    path: String,
    expected_identity: ApprovedFileIdentity,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ApprovedExternalWriteRequest {
    path: String,
    content: String,
    anchor_path: PathBuf,
    anchor_identity: ApprovedFileIdentity,
    missing_components: Vec<OsString>,
    expected_target_identity: Option<ApprovedFileIdentity>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TelemetryArchiveRequest {
    pub output_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileDeleteRequest {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodebasePatchRequest {
    pub target_file_path: String,
    pub search_pattern: String,
    pub replacement_content: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodebaseCompileTarget {
    Backend,
    Frontend,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodebaseCompileRequest {
    pub target: CodebaseCompileTarget,
}

impl CodebaseCompileTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Frontend => "frontend",
        }
    }

    fn parse(value: &str) -> Result<Self, ShieldGateError> {
        match value.trim().replace('-', "_").to_ascii_lowercase().as_str() {
            "backend" => Ok(Self::Backend),
            "frontend" => Ok(Self::Frontend),
            _ => Err(security_boundary_violation(format!(
                "codebase_compile target must be backend or frontend, got '{value}'."
            ))),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileListRequest {
    pub path: String,
}

pub type SystemExecutionRequest = crate::tools::terminal_contract::NativeTerminalRequest;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SystemAuditRequest {
    pub scope: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SovereignDuckDuckGoSearchRequest {
    pub query: String,
    pub max_results: usize,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AirlockExportRequest {
    pub artifact_path: String,
    pub mount_path: String,
    pub mission_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExecuteCommandResponse {
    pub operation: String,
    pub status: CommandStatus,
    pub message: String,
    pub metrics: Option<SystemMetrics>,
    pub claims: Vec<String>,
    pub verified: bool,
    pub model_used: Option<ModelMetadata>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelMetadata {
    pub name: String,
    pub version: String,
    pub provider: String,
    pub locality: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Completed,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SystemMetrics {
    pub os: String,
    pub arch: String,
    pub logical_cpus: usize,
    pub unix_time_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct ShieldGateError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
}

impl From<crate::authority::NativeAuthorityError> for ShieldGateError {
    fn from(error: crate::authority::NativeAuthorityError) -> Self {
        Self {
            code: error.code,
            boundary: error.boundary,
            message: error.message,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VisualWorkflowNode {
    pub id: String,
    pub dependencies: Vec<String>,
    pub action_kind: String,
    pub path: Option<String>,
}

pub struct TrustPolicy;

impl TrustPolicy {
    pub fn allows_low_risk_plan<'a>(risk_levels: impl IntoIterator<Item = &'a str>) -> bool {
        risk_levels
            .into_iter()
            .all(|risk| risk.eq_ignore_ascii_case("low"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShieldToolApprovalTier {
    BackgroundAutoApproval,
    VisualConsent,
    ExplicitConfirmation,
}

impl ShieldToolApprovalTier {
    fn requires_user_confirmation(self) -> bool {
        !matches!(self, Self::BackgroundAutoApproval)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::BackgroundAutoApproval => "background_auto_approval",
            Self::VisualConsent => "visual_consent",
            Self::ExplicitConfirmation => "explicit_confirmation",
        }
    }

    fn approval_mode(self) -> &'static str {
        match self {
            Self::BackgroundAutoApproval => "background",
            Self::VisualConsent => "visual",
            Self::ExplicitConfirmation => "explicit",
        }
    }

    fn risk_label(self) -> &'static str {
        match self {
            Self::BackgroundAutoApproval => "Low Risk",
            Self::VisualConsent => "Medium Risk",
            Self::ExplicitConfirmation => "High Risk",
        }
    }
}

fn classify_registered_system_tool(action_kind: &str) -> Option<ShieldToolApprovalTier> {
    let normalized = normalize_action_kind(action_kind);
    let built_in = match normalized.as_str() {
        "get_system_metrics"
        | "file_read"
        | "file_list"
        | "system_audit"
        | "ask_local_document_index"
        | "sovereign_duckduckgo_search"
        | "duckduckgo_search"
        | "codebase_compile" => Some(ShieldToolApprovalTier::BackgroundAutoApproval),
        "create_spreadsheet" | "app_control" => {
            Some(ShieldToolApprovalTier::BackgroundAutoApproval)
        }
        "file_write" | "codebase_patch" | "document_index" => {
            Some(ShieldToolApprovalTier::VisualConsent)
        }
        "delete_file"
        | "trash"
        | "trash_file"
        | "terminal_execute"
        | "shell_command"
        | "execute_command"
        | "web_fetch"
        | "network_request"
        | "airlock_export"
        | "telemetry_archive"
        | "mcp_connect_server"
        | "mcp_execute_remote_tool" => Some(ShieldToolApprovalTier::ExplicitConfirmation),
        _ => None,
    };
    built_in.or_else(|| {
        use crate::tools::task_tool_runtime::TaskToolApprovalTier;
        crate::tools::task_tool_runtime::approval_tier(&normalized).map(|tier| match tier {
            TaskToolApprovalTier::Background => ShieldToolApprovalTier::BackgroundAutoApproval,
            TaskToolApprovalTier::Visual => ShieldToolApprovalTier::VisualConsent,
            TaskToolApprovalTier::Explicit => ShieldToolApprovalTier::ExplicitConfirmation,
        })
    })
}

fn normalize_action_kind(action_kind: &str) -> String {
    action_kind.trim().replace('-', "_").to_ascii_lowercase()
}

#[derive(Debug, Clone)]
struct ShieldAuthorizationContext {
    shield_approved: bool,
    trusted_working_directory: Option<String>,
}

impl ShieldAuthorizationContext {
    fn one_time(shield_approved: bool) -> Self {
        Self {
            shield_approved,
            trusted_working_directory: None,
        }
    }
}

#[derive(Debug, Clone)]
struct TrustedActionGrant {
    grant: SovereignTrustGrant,
    estimated_token_cost: i64,
    estimated_cpu_seconds_reservation: f64,
}

#[derive(Debug, Clone)]
enum SovereignTrustDecision {
    PromptRequired,
    Trusted(TrustedActionGrant),
}

impl ScopeTrustManager {
    fn session_grants(&self) -> Result<Vec<SessionScopeTrustGrant>, ShieldGateError> {
        let now_ms = unix_time_ms_u64();
        let mut cache = self.cache.lock().map_err(|_| ShieldGateError {
            code: "scope_trust_lock_failed",
            boundary: "ScopeTrustCache",
            message: "Unable to inspect application-session folder access.".to_string(),
        })?;
        cache.retain(|entry| entry.expires_at_ms > now_ms);
        let mut grants = cache
            .iter()
            .filter(|entry| entry.app_session)
            .flat_map(|entry| {
                entry
                    .allowed_operations
                    .iter()
                    .map(|action_class| SessionScopeTrustGrant {
                        grant_id: entry.grant_id.clone(),
                        principal: entry.principal.clone(),
                        canonical_resource: entry.authorized_prefix.display().to_string(),
                        action_class: action_class.clone(),
                        granted_at_ms: entry.granted_at_ms,
                    })
            })
            .collect::<Vec<_>>();
        grants.sort_by_key(|grant| grant.granted_at_ms);
        grants.reverse();
        Ok(grants)
    }

    fn revoke_session_grant(&self, grant_id: &str) -> Result<bool, ShieldGateError> {
        let mut cache = self.cache.lock().map_err(|_| ShieldGateError {
            code: "scope_trust_lock_failed",
            boundary: "ScopeTrustCache",
            message: "Unable to update application-session folder access.".to_string(),
        })?;
        let before = cache.len();
        cache.retain(|entry| entry.grant_id != grant_id);
        Ok(cache.len() != before)
    }

    #[cfg(test)]
    pub(crate) fn allows_action(&self, action: &RequestedAction) -> Result<bool, ShieldGateError> {
        let principal = action
            .principal
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("local_principal");
        self.allows_action_for_principal(action, principal)
    }

    pub(crate) fn allows_action_for_principal(
        &self,
        action: &RequestedAction,
        principal: &str,
    ) -> Result<bool, ShieldGateError> {
        let Some((operation, target_path)) = scope_trust_action_scope(action)? else {
            return Ok(false);
        };
        let now_ms = unix_time_ms_u64();
        let mut cache = self.cache.lock().map_err(|_| ShieldGateError {
            code: "scope_trust_lock_failed",
            boundary: "ScopeTrustCache",
            message: "Unable to inspect temporary scope trust cache.".to_string(),
        })?;
        cache.retain(|entry| entry.expires_at_ms > now_ms);
        Ok(cache.iter().any(|entry| {
            entry.principal == principal
                && entry
                    .allowed_operations
                    .iter()
                    .any(|allowed| allowed == &operation)
                && target_path.starts_with(&entry.authorized_prefix)
        }))
    }

    #[cfg(test)]
    fn grant_from_approval(
        &self,
        approval: &ShieldApprovalRequest,
        scope_trust: Option<&ScopeTrustApprovalRequest>,
    ) -> Result<bool, ShieldGateError> {
        Ok(self
            .grant_from_approval_with_id(approval, scope_trust)?
            .is_some())
    }

    fn grant_from_approval_with_id(
        &self,
        approval: &ShieldApprovalRequest,
        scope_trust: Option<&ScopeTrustApprovalRequest>,
    ) -> Result<Option<String>, ShieldGateError> {
        let Some(scope_trust) = scope_trust else {
            return Ok(None);
        };
        if !scope_trust.enabled {
            return Ok(None);
        }
        if approval.mandatory_reconfirm
            || (approval.action_class != "filesystem_read"
                && crate::approval_scopes::mandatory_reconfirmation(&approval.action_type))
        {
            return Err(ShieldGateError {
                code: "scope_trust_unavailable",
                boundary: "ScopeTrustCache",
                message: "This action always requires a new approval.".to_string(),
            });
        }
        let app_session_is_offered = approval
            .approval_scope_kinds
            .iter()
            .any(|allowed| allowed == "app_session");
        match scope_trust.kind.as_deref() {
            Some("app_session") if app_session_is_offered => {}
            // Older renderers omitted `kind` for their temporary folder grant.
            // Retain that compatibility only for the filesystem request shape
            // that now explicitly offers application-session authority.
            None if app_session_is_offered => {}
            _ => {
                return Err(ShieldGateError {
                    code: "scope_trust_kind_invalid",
                    boundary: "ScopeTrustCache",
                    message: "That permission duration is not available for this action."
                        .to_string(),
                });
            }
        }
        if !approval.scope_trust_available {
            return Err(ShieldGateError {
                code: "scope_trust_unavailable",
                boundary: "ScopeTrustCache",
                message: "Temporary folder trust is not available for this action.".to_string(),
            });
        }
        let Some(prefix) = approval.scope_trust_prefix.as_deref() else {
            return Err(ShieldGateError {
                code: "scope_trust_missing_prefix",
                boundary: "ScopeTrustCache",
                message: "Temporary folder trust requires an approved folder prefix.".to_string(),
            });
        };
        let app_session = scope_trust.kind.as_deref() == Some("app_session");
        let duration_ms = scope_trust
            .duration_ms
            .unwrap_or(DEFAULT_SCOPE_TRUST_DURATION_MS)
            .clamp(1_000, 60 * 60 * 1_000);
        // Application-session grants live only in this in-memory manager. They
        // disappear when OOMU quits and never become durable policy.
        let expires_at_ms = if app_session {
            u64::MAX
        } else {
            unix_time_ms_u64().saturating_add(duration_ms)
        };
        let mut cache = self.cache.lock().map_err(|_| ShieldGateError {
            code: "scope_trust_lock_failed",
            boundary: "ScopeTrustCache",
            message: "Unable to update temporary scope trust cache.".to_string(),
        })?;
        let granted_at_ms = unix_time_ms_u64();
        let grant_id = format!("sessiongrant_{}", new_approval_token());
        cache.push(ScopeTrustCache {
            authorized_prefix: PathBuf::from(prefix),
            allowed_operations: vec![reviewed_action_class(&approval.action_type)],
            expires_at_ms,
            principal: approval
                .principal
                .clone()
                .unwrap_or_else(|| "local_principal".to_string()),
            grant_id: grant_id.clone(),
            granted_at_ms,
            app_session,
        });
        Ok(Some(grant_id))
    }
}

impl ActuationLeaseManager {
    pub fn grant(
        &self,
        actor_id: String,
        session_id: &str,
        operation_classes: Vec<String>,
        canonical_scopes: Vec<String>,
        duration_ms: u64,
        max_steps: usize,
    ) -> Result<ActuationLeaseStatus, ShieldGateError> {
        let session_id = required_actuation_session_id(Some(session_id))?;
        if duration_ms == 0 {
            return Err(ShieldGateError {
                code: "actuation_lease_invalid_duration",
                boundary: "ActuationLeaseManager",
                message: "Actuation lease duration must be greater than zero.".to_string(),
            });
        }
        if duration_ms > 15 * 60 * 1_000 {
            return Err(ShieldGateError {
                code: "actuation_lease_invalid_duration",
                boundary: "ActuationLeaseManager",
                message: "Actuation access cannot exceed 15 minutes.".to_string(),
            });
        }
        if !(1..=50).contains(&max_steps) {
            return Err(ShieldGateError {
                code: "actuation_lease_invalid_steps",
                boundary: "ActuationLeaseManager",
                message: "Actuation access must allow between 1 and 50 actions.".to_string(),
            });
        }

        let now_ms = unix_time_ms_u64();
        let lease = ActuationLease {
            actor_id,
            session_id,
            operation_classes,
            canonical_scopes,
            expires_at_ms: now_ms.saturating_add(duration_ms),
            max_steps,
            current_steps: 0,
            is_active: true,
        };
        let mut active_lease = self
            .lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *active_lease = Some(lease);
        Ok(status_from_lease(active_lease.as_ref(), now_ms, None))
    }

    pub fn snapshot(&self) -> ActuationLeaseStatus {
        let now_ms = unix_time_ms_u64();
        let mut active_lease = self
            .lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let reason = active_lease.as_mut().and_then(|lease| {
            if lease.is_active && now_ms > lease.expires_at_ms {
                lease.is_active = false;
                Some("expired".to_string())
            } else {
                None
            }
        });
        status_from_lease(active_lease.as_ref(), now_ms, reason)
    }

    pub(crate) fn evaluate_autonomous_action(
        &self,
        app: Option<&tauri::AppHandle>,
        actor_id: Option<&str>,
        session_id: Option<&str>,
        action: &AuthorizedActions,
    ) -> Result<ActuationLeaseOutcome, ShieldGateError> {
        if !is_mutating_action(action) {
            return Ok(ActuationLeaseOutcome::NotRequired);
        }

        let requested_session_id = required_actuation_session_id(session_id)?;
        let requested_actor_id = actor_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ShieldGateError {
                code: "actuation_actor_required",
                boundary: "ActuationLeaseManager",
                message: "The active local identity is required for this action.".to_string(),
            })?;
        let operation = action.operation_name().to_string();
        let now_ms = unix_time_ms_u64();
        let mut active_lease = self
            .lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let Some(lease) = active_lease.as_mut() else {
            let status = status_from_lease(None, now_ms, Some("missing".to_string()));
            emit_actuation_lease_decayed(
                app,
                &status,
                "missing",
                Some(operation),
                requested_session_id.clone(),
                None,
            );
            return Ok(ActuationLeaseOutcome::Blocked(
                status,
                "No active actuation lease is available for autonomous mutating actions."
                    .to_string(),
            ));
        };

        if lease.session_id != requested_session_id {
            let status =
                status_from_lease(Some(lease), now_ms, Some("session_mismatch".to_string()));
            emit_actuation_lease_decayed(
                app,
                &status,
                "session_mismatch",
                Some(operation),
                requested_session_id.clone(),
                None,
            );
            return Ok(ActuationLeaseOutcome::Blocked(
                status,
                format!(
                    "Active actuation lease belongs to session '{}' and cannot authorize session '{}'.",
                    lease.session_id, requested_session_id
                ),
            ));
        }

        let action_class = if matches!(action, AuthorizedActions::RegisteredTaskTool(_)) {
            "registered_task_tool".to_string()
        } else {
            reviewed_action_class(action.operation_name())
        };
        if lease.actor_id != requested_actor_id
            || !lease
                .operation_classes
                .iter()
                .any(|allowed| allowed == &action_class)
            || lease.canonical_scopes != vec![format!("actuation-session:{requested_session_id}")]
        {
            let status =
                status_from_lease(Some(lease), now_ms, Some("authority_mismatch".to_string()));
            emit_actuation_lease_decayed(
                app,
                &status,
                "authority_mismatch",
                Some(operation),
                requested_session_id,
                None,
            );
            return Ok(ActuationLeaseOutcome::Blocked(
                status,
                "Active access does not cover this identity, action, and session scope."
                    .to_string(),
            ));
        }

        let allowed =
            validate_and_decrement_lease(lease, action).map_err(|message| ShieldGateError {
                code: "actuation_lease_clock_error",
                boundary: "ActuationLeaseManager",
                message,
            })?;
        let status = status_from_lease(
            Some(lease),
            now_ms,
            if allowed {
                None
            } else {
                Some("decayed".to_string())
            },
        );

        if allowed {
            Ok(ActuationLeaseOutcome::Authorized(status))
        } else {
            emit_actuation_lease_decayed(
                app,
                &status,
                "decayed",
                Some(operation),
                requested_session_id.clone(),
                None,
            );
            Ok(ActuationLeaseOutcome::Blocked(
                status,
                "Actuation lease expired or exhausted its step budget.".to_string(),
            ))
        }
    }

    pub(crate) fn enforce_autonomous_action(
        &self,
        app: Option<&tauri::AppHandle>,
        actor_id: Option<&str>,
        session_id: Option<&str>,
        action: &AuthorizedActions,
    ) -> Result<(), ShieldGateError> {
        match self.evaluate_autonomous_action(app, actor_id, session_id, action)? {
            ActuationLeaseOutcome::NotRequired => Ok(()),
            ActuationLeaseOutcome::Authorized(status) => {
                let _remaining_steps = status.remaining_steps;
                Ok(())
            }
            ActuationLeaseOutcome::Blocked(status, reason) => Err(ShieldGateError {
                code: "actuation_lease_required",
                boundary: "ActuationLeaseManager",
                message: format!(
                    "{reason} Manual approval is required before execution can continue. Lease active={} remaining_steps={}.",
                    status.active, status.remaining_steps
                ),
            }),
        }
    }

    pub(crate) fn terminate_for_review(
        &self,
        app: Option<&tauri::AppHandle>,
        session_id: Option<&str>,
        reason: &str,
        operation: Option<String>,
        review_preview: Option<String>,
    ) -> ActuationLeaseStatus {
        let requested_session_id = session_id.map(str::trim).unwrap_or_default().to_string();
        let now_ms = unix_time_ms_u64();
        let mut active_lease = self
            .lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(lease) = active_lease.as_mut() {
            if !requested_session_id.is_empty() && lease.session_id == requested_session_id {
                lease.is_active = false;
            }
        }
        let status = status_from_lease(active_lease.as_ref(), now_ms, Some(reason.to_string()));
        emit_actuation_lease_decayed(
            app,
            &status,
            reason,
            operation,
            requested_session_id,
            review_preview,
        );
        status
    }

    pub(crate) fn finish_session(
        &self,
        app: Option<&tauri::AppHandle>,
        session_id: Option<&str>,
        reason: &str,
    ) -> Option<ActuationLeaseStatus> {
        let requested_session_id = session_id.map(str::trim).unwrap_or_default().to_string();
        let now_ms = unix_time_ms_u64();
        let mut active_lease = self
            .lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let lease = active_lease.as_mut()?;
        if lease.session_id != requested_session_id || !lease.is_active {
            return None;
        }
        lease.is_active = false;
        let status = status_from_lease(active_lease.as_ref(), now_ms, Some(reason.to_string()));
        emit_actuation_lease_decayed(app, &status, reason, None, requested_session_id, None);
        Some(status)
    }
}

pub fn validate_and_decrement_lease(
    lease: &mut ActuationLease,
    action: &AuthorizedActions,
) -> Result<bool, String> {
    if !is_mutating_action(action) {
        return Ok(true);
    }

    let now = unix_time_ms_u64();

    if !lease.is_active || now > lease.expires_at_ms || lease.current_steps >= lease.max_steps {
        lease.is_active = false;
        return Ok(false);
    }

    lease.current_steps += 1;
    Ok(true)
}

pub fn is_mutating_action(action: &AuthorizedActions) -> bool {
    if let AuthorizedActions::RegisteredTaskTool(request) = action {
        return request.potentially_effectful();
    }
    if let AuthorizedActions::ApprovedSystemExecution(request) = action {
        return request.classification().requires_human_approval();
    }
    matches!(
        action,
        AuthorizedActions::FileWrite(_)
            | AuthorizedActions::CodebasePatch(_)
            | AuthorizedActions::CodebaseCompile(_)
            | AuthorizedActions::ApprovedExternalFileWrite(_)
            | AuthorizedActions::TelemetryArchive(_)
            | AuthorizedActions::ApprovedFileDelete(_)
            | AuthorizedActions::DocumentIndex
            | AuthorizedActions::AirlockExport(_)
    )
}

fn required_actuation_session_id(session_id: Option<&str>) -> Result<String, ShieldGateError> {
    session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| ShieldGateError {
            code: "actuation_session_required",
            boundary: "ActuationLeaseManager",
            message: "Start this access from a specific chat or task session.".to_string(),
        })
}

fn status_from_lease(
    lease: Option<&ActuationLease>,
    now_ms: u64,
    reason: Option<String>,
) -> ActuationLeaseStatus {
    let active = lease
        .map(|lease| lease.is_active && now_ms <= lease.expires_at_ms)
        .unwrap_or(false);
    let remaining_ms = lease
        .map(|lease| lease.expires_at_ms.saturating_sub(now_ms))
        .unwrap_or_default();
    let remaining_steps = lease
        .map(|lease| lease.max_steps.saturating_sub(lease.current_steps))
        .unwrap_or_default();

    ActuationLeaseStatus {
        lease: lease.cloned(),
        active,
        now_ms,
        remaining_ms,
        remaining_steps,
        reason,
    }
}

fn emit_actuation_lease_decayed(
    app: Option<&tauri::AppHandle>,
    status: &ActuationLeaseStatus,
    reason: &str,
    operation: Option<String>,
    session_id: String,
    review_preview: Option<String>,
) {
    let Some(app) = app else {
        return;
    };
    let event = ActuationLeaseDecayEvent {
        status: status.clone(),
        reason: reason.to_string(),
        operation,
        session_id,
        review_preview,
    };
    let _ = app.emit(ACTUATION_LEASE_DECAYED_EVENT, event);
}

impl ShieldApprovalManager {
    async fn pending_requests(&self) -> Vec<ShieldApprovalStatus> {
        let pending = self.pending.lock().await;
        let mut requests = pending
            .values()
            .map(|entry| shield_approval_status(entry, true))
            .collect::<Vec<_>>();
        requests.sort_by_key(|request| request.requested_at_ms);
        requests
    }

    async fn request_approval(
        &self,
        app: &tauri::AppHandle,
        request: ShieldApprovalRequest,
    ) -> Result<ShieldApprovalDecision, ShieldGateError> {
        let frozen = crate::authority::shield_decision::freeze_request(&request)?;
        let token = frozen.approval_token.clone();
        let display_id = format!("shieldstatus_{}", new_approval_token());
        {
            let mut pending = self.pending.lock().await;
            if pending.contains_key(&token) {
                return Err(ShieldGateError {
                    code: "shield_approval_duplicate",
                    boundary: "ShieldApprovalManager",
                    message: "That Shield Gate action is already awaiting a decision.".to_string(),
                });
            }
            pending.insert(
                token.clone(),
                PendingShieldApproval {
                    request: request.clone(),
                    frozen: frozen.clone(),
                    display_id,
                },
            );
        }

        let pending_status = {
            let pending = self.pending.lock().await;
            pending
                .get(&token)
                .map(|entry| shield_approval_status(entry, true))
        };
        if let Some(status) = pending_status.as_ref() {
            if let Err(error) = app.emit("shield-approval-status-changed", status) {
                let mut pending = self.pending.lock().await;
                pending.remove(&token);
                return Err(ShieldGateError {
                    code: "shield_approval_event_failed",
                    boundary: "ShieldApprovalManager",
                    message: format!("Unable to publish Shield Gate status: {error}"),
                });
            }
        }

        let result = async {
            let _prompt_guard = self.prompt_lock.lock().await;
            let identity = app.state::<SovereignIdentity>();
            let actor_id = crate::authority::current_actor_id(identity.inner())?;
            let persistence = app.state::<PersistenceEngine>();
            let locale = crate::settings::locale_state_for_engine(persistence.inner(), None)
                .map(|state| state.active_locale)
                .unwrap_or_else(|_| "en-US".to_string());
            let selection =
                system_action_semantics::scenario_one_native_selection(app, &request, &locale)
                    .await?;
            let decision_id =
                self.decisions
                    .issue_after_native_presence(&frozen, actor_id.clone(), selection)?;
            let decision = self.decisions.consume(&decision_id, &frozen, &actor_id)?;
            if decision.decision_id != decision_id || decision.nonce.is_empty() {
                return Err(ShieldGateError {
                    code: "shield_decision_identity_invalid",
                    boundary: "ShieldApprovalManager",
                    message: "The native Shield decision could not be verified.".to_string(),
                });
            }
            if decision.decision == ShieldApprovalDecision::Approve {
                apply_native_scope_selection(app, &request, &decision.scope_kind)?;
            }
            eprintln!(
                "SHIELD_NATIVE_DECISION request_sha256={} decision={} scope={} consumed=true",
                decision.request_sha256,
                match decision.decision {
                    ShieldApprovalDecision::Approve => "approved",
                    ShieldApprovalDecision::Deny => "denied",
                },
                decision.scope_kind,
            );
            Ok(decision.decision)
        }
        .await;

        let resolved_status = {
            let mut pending = self.pending.lock().await;
            pending
                .remove(&token)
                .map(|entry| shield_approval_status(&entry, false))
        };
        if let Some(status) = resolved_status {
            let _ = app.emit("shield-approval-status-changed", status);
        }
        result
    }
}

fn shield_approval_status(
    pending: &PendingShieldApproval,
    is_pending: bool,
) -> ShieldApprovalStatus {
    debug_assert_eq!(
        pending.frozen.approval_token,
        pending.request.approval_token
    );
    ShieldApprovalStatus {
        display_id: pending.display_id.clone(),
        session_id: pending.request.session_id.clone(),
        action_label: pending.request.action_label.clone(),
        semantic_summary: pending.request.semantic_summary.clone(),
        requested_at_ms: pending.request.requested_at_ms,
        pending: is_pending,
    }
}

fn apply_native_scope_selection(
    app: &tauri::AppHandle,
    request: &ShieldApprovalRequest,
    scope_kind: &str,
) -> Result<(), ShieldGateError> {
    if scope_kind == "once" {
        return Ok(());
    }
    let selection = ScopeTrustApprovalRequest {
        enabled: true,
        duration_ms: Some(request.scope_trust_duration_ms),
        kind: Some(scope_kind.to_string()),
        max_uses: None,
    };
    if scope_kind == "app_session" {
        app.state::<ScopeTrustManager>()
            .grant_from_approval_with_id(request, Some(&selection))?;
        return Ok(());
    }
    crate::approval_scopes::grant(
        app.state::<PersistenceEngine>().inner(),
        request,
        &selection,
    )
    .map(|_| ())
    .map_err(|message| ShieldGateError {
        code: "approval_scope_invalid",
        boundary: "ReviewedApprovalScope",
        message,
    })
}

#[tauri::command]
pub async fn list_pending_shield_approvals(
    approvals: tauri::State<'_, ShieldApprovalManager>,
) -> Result<Vec<ShieldApprovalStatus>, ShieldGateError> {
    Ok(approvals.pending_requests().await)
}

#[tauri::command]
pub fn list_session_scope_trust_grants(
    scope_trust: tauri::State<'_, ScopeTrustManager>,
) -> Result<Vec<SessionScopeTrustGrant>, ShieldGateError> {
    scope_trust.session_grants()
}

#[tauri::command]
pub fn revoke_session_scope_trust_grant(
    request: RevokeSessionScopeTrustRequest,
    scope_trust: tauri::State<'_, ScopeTrustManager>,
) -> Result<bool, ShieldGateError> {
    scope_trust.revoke_session_grant(request.grant_id.trim())
}

pub async fn request_user_approval(
    app: &tauri::AppHandle,
    approvals: &ShieldApprovalManager,
    request: ShieldApprovalRequest,
) -> Result<(), ShieldGateError> {
    match approvals.request_approval(app, request).await? {
        ShieldApprovalDecision::Approve => Ok(()),
        ShieldApprovalDecision::Deny => Err(ShieldGateError {
            code: "shield_approval_denied",
            boundary: "ShieldApprovalManager",
            message: "Access denied by the Principal at the Shield Gate approval prompt."
                .to_string(),
        }),
    }
}

async fn request_direct_command_approval(
    app: &tauri::AppHandle,
    approvals: &ShieldApprovalManager,
    action: &RequestedAction,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    generation_token: Option<&str>,
    project_id: Option<&str>,
    task_run_id: Option<&str>,
    agent_id: Option<&str>,
) -> Result<bool, ShieldGateError> {
    let Some(mut approval_request) = build_shield_approval_request(action) else {
        return Ok(false);
    };
    approval_request.session_id = clean_approval_context_value(session_id);
    approval_request.turn_id = clean_approval_context_value(turn_id);
    approval_request.generation_token = clean_approval_context_value(generation_token);
    approval_request.project_id = clean_approval_context_value(project_id);
    approval_request.task_run_id = clean_approval_context_value(task_run_id);
    if approval_request.principal.is_none() {
        approval_request.principal =
            clean_approval_context_value(agent_id).or_else(|| Some("local_principal".to_string()));
    }
    approval_request.approval_scope_kinds = available_scope_kinds(&approval_request);

    match approvals.request_approval(app, approval_request).await? {
        ShieldApprovalDecision::Approve => Ok(true),
        ShieldApprovalDecision::Deny => Err(ShieldGateError {
            code: "shield_approval_denied",
            boundary: "ShieldApprovalManager",
            message: "Access denied by the Principal at the Shield Gate approval prompt."
                .to_string(),
        }),
    }
}

fn clean_approval_context_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn reviewed_principal<'a>(action: &'a RequestedAction, agent_id: Option<&'a str>) -> &'a str {
    action
        .principal
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| agent_id.map(str::trim).filter(|value| !value.is_empty()))
        .unwrap_or("local_principal")
}

#[tauri::command]
pub fn get_actuation_lease_status(
    leases: tauri::State<'_, ActuationLeaseManager>,
) -> ActuationLeaseStatus {
    leases.snapshot()
}

#[tauri::command]
pub fn grant_actuation_lease(
    request: GrantActuationLeaseRequest,
    leases: tauri::State<'_, ActuationLeaseManager>,
    authority: tauri::State<'_, crate::authority::NativeAuthorityManager>,
    identity: tauri::State<'_, SovereignIdentity>,
    app: tauri::AppHandle,
) -> Result<ActuationLeaseStatus, ShieldGateError> {
    let actor_id = crate::authority::current_actor_id(identity.inner())?;
    let session_id = required_actuation_session_id(Some(&request.session_id))?;
    let canonical_scope = format!("actuation-session:{session_id}");
    authority.consume(
        &request.authority_proof_id,
        crate::authority::NativeAuthorityExpectation {
            actor_id: actor_id.clone(),
            session_id: session_id.clone(),
            operation_classes: request.operation_classes.clone(),
            canonical_scopes: vec![canonical_scope.clone()],
            max_steps: request.max_steps,
            allowed_persistences: vec!["one_time".to_string(), "session_gated".to_string()],
        },
    )?;
    let status = leases.grant(
        actor_id,
        &session_id,
        request.operation_classes,
        vec![canonical_scope],
        request.duration_ms,
        request.max_steps,
    )?;
    if let Err(error) = app.emit(ACTUATION_LEASE_UPDATED_EVENT, &status) {
        leases.finish_session(None, Some(&session_id), "actuation_lease_event_failed");
        return Err(ShieldGateError {
            code: "actuation_lease_event_failed",
            boundary: "ActuationLeaseManager",
            message: format!("Unable to emit actuation lease update: {error}"),
        });
    }
    Ok(status)
}

#[tauri::command]
pub fn revoke_actuation_lease(
    request: RevokeActuationLeaseRequest,
    leases: tauri::State<'_, ActuationLeaseManager>,
    app: tauri::AppHandle,
) -> ActuationLeaseStatus {
    leases.terminate_for_review(
        Some(&app),
        Some(&request.session_id),
        request.reason.as_deref().unwrap_or("manual_revocation"),
        None,
        None,
    )
}

#[tauri::command]
pub async fn apply_surgical_patch_directive(
    request: SurgicalPatchDirectiveRequest,
) -> Result<ExecuteCommandResponse, ShieldGateError> {
    let diff = request.diff.trim().to_string();
    if diff.is_empty() {
        return Err(ShieldGateError {
            code: "shield_gate_invalid_input",
            boundary: "SurgicalPatchDirective",
            message: "Surgical patch directive requires a non-empty unified diff.".to_string(),
        });
    }
    if diff.len() > 256 * 1024 {
        return Err(ShieldGateError {
            code: "shield_gate_invalid_input",
            boundary: "SurgicalPatchDirective",
            message: "Surgical patch directive exceeds the 256KB safety limit.".to_string(),
        });
    }

    tauri::async_runtime::spawn_blocking(move || apply_unified_diff_directive(&diff))
        .await
        .map_err(|error| ShieldGateError {
            code: "shield_gate_execution_failed",
            boundary: "SurgicalPatchDirective",
            message: error.to_string(),
        })?
}

fn approved_chat_file_error(message: impl Into<String>) -> ShieldGateError {
    ShieldGateError {
        code: "approved_file_unavailable",
        boundary: "ApprovedChatFileReceipt",
        message: message.into(),
    }
}

fn approved_chat_file_media_cache() -> &'static StdMutex<HashMap<String, ApprovedChatFileMedia>> {
    APPROVED_CHAT_FILE_MEDIA_CACHE.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn prune_approved_chat_file_media_cache(
    cache: &mut HashMap<String, ApprovedChatFileMedia>,
    now_ms: i64,
) {
    cache.retain(|_, item| item.expires_at_ms >= now_ms);
    loop {
        let total_bytes = cache.values().map(|item| item.bytes.len()).sum::<usize>();
        if cache.len() <= MAX_APPROVED_CHAT_FILE_MEDIA_CACHE_ENTRIES
            && total_bytes <= MAX_APPROVED_CHAT_FILE_MEDIA_CACHE_BYTES
        {
            break;
        }
        let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, item)| item.issued_at_ms)
            .map(|(receipt_id, _)| receipt_id.clone())
        else {
            break;
        };
        cache.remove(&oldest);
    }
}

fn cache_approved_chat_file_media(
    receipt_id: String,
    media: ApprovedChatFileMedia,
) -> Result<(), ShieldGateError> {
    let mut cache = approved_chat_file_media_cache()
        .lock()
        .map_err(|_| approved_chat_file_error("The approved image could not be held securely."))?;
    let now_ms = unix_time_ms_i64();
    prune_approved_chat_file_media_cache(&mut cache, now_ms);
    cache.insert(receipt_id, media);
    prune_approved_chat_file_media_cache(&mut cache, now_ms);
    Ok(())
}

fn verified_approved_chat_file_media(
    payload: &ApprovedFileReceiptPayload,
) -> Result<Option<String>, String> {
    let Some(expected_sha256) = payload.media_sha256.as_deref() else {
        return Ok(None);
    };
    let now_ms = unix_time_ms_i64();
    let mut cache = approved_chat_file_media_cache()
        .lock()
        .map_err(|_| "receipt_media_unavailable".to_string())?;
    prune_approved_chat_file_media_cache(&mut cache, now_ms);
    let media = cache
        .get(&payload.receipt_id)
        .ok_or_else(|| "receipt_media_unavailable".to_string())?;
    if media.session_id != payload.session_id
        || media.root_turn_id != payload.root_turn_id
        || media.agent_id != payload.agent_id
        || media.mime_type != payload.mime_type
        || media.bytes.len() != payload.byte_count
        || media.sha256 != expected_sha256
        || media.expires_at_ms != payload.expires_at_ms
        || sha256_hex(media.bytes.as_slice()) != expected_sha256
    {
        return Err("receipt_media_invalid".to_string());
    }
    Ok(Some(
        base64::engine::general_purpose::STANDARD.encode(media.bytes.as_slice()),
    ))
}

pub(crate) fn verify_approved_file_receipt(
    receipt: &ApprovedFileReceiptToken,
    identity: &SovereignIdentity,
    session_id: &str,
    root_turn_id: &str,
    agent_id: &str,
) -> Result<VerifiedApprovedFileContext, String> {
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(receipt.payload.as_bytes())
        .map_err(|_| "receipt_payload_invalid".to_string())?;
    if payload_bytes.len() > MAX_APPROVED_CHAT_FILE_CONTEXT_BYTES.saturating_add(16 * 1024) {
        return Err("receipt_payload_too_large".to_string());
    }
    let payload_json =
        std::str::from_utf8(&payload_bytes).map_err(|_| "receipt_payload_invalid".to_string())?;
    identity
        .verify_payload(payload_json, &receipt.signature)
        .map_err(|_| "receipt_signature_invalid".to_string())?;
    let payload: ApprovedFileReceiptPayload =
        serde_json::from_str(payload_json).map_err(|_| "receipt_payload_invalid".to_string())?;
    if payload.version != LEGACY_APPROVED_CHAT_FILE_RECEIPT_VERSION
        && payload.version != APPROVED_CHAT_FILE_RECEIPT_VERSION
    {
        return Err("receipt_version_invalid".to_string());
    }
    let now_ms = unix_time_ms_i64();
    if payload.expires_at_ms < now_ms || payload.issued_at_ms > now_ms.saturating_add(60_000) {
        return Err("receipt_expired".to_string());
    }
    if payload.session_id != session_id
        || payload.root_turn_id != root_turn_id
        || payload.agent_id != agent_id
    {
        return Err("receipt_turn_binding_invalid".to_string());
    }
    let base_content_invalid = payload.display_name.trim().is_empty()
        || payload.mime_type.trim().is_empty()
        || payload.byte_count == 0
        || payload.content.trim().is_empty()
        || payload.content.len() > MAX_APPROVED_CHAT_FILE_CONTEXT_BYTES
        || sha256_hex(payload.content.as_bytes()) != payload.content_sha256
        || payload.target_identity_hash.len() != 64
        || payload.receipt_id.len() != 48;
    let version_content_invalid = if payload.version == LEGACY_APPROVED_CHAT_FILE_RECEIPT_VERSION {
        payload.mime_type != "text/plain"
            || payload.content.len() != payload.byte_count
            || payload.media_sha256.is_some()
    } else if payload.mime_type.starts_with("image/") {
        payload.byte_count > MAX_APPROVED_CHAT_FILE_MEDIA_BYTES
            || payload
                .media_sha256
                .as_deref()
                .is_none_or(|digest| digest.len() != 64)
    } else {
        payload.media_sha256.is_some()
    };
    if base_content_invalid || version_content_invalid {
        return Err("receipt_content_invalid".to_string());
    }
    let data_base64 = verified_approved_chat_file_media(&payload)?;
    Ok(VerifiedApprovedFileContext {
        display_name: payload.display_name,
        mime_type: payload.mime_type,
        byte_count: payload.byte_count,
        content: payload.content,
        data_base64,
        display_message: payload.display_message,
    })
}

fn require_durable_direct_command(persistence: &PersistenceEngine) -> Result<(), ShieldGateError> {
    persistence
        .require_durable_store("direct Shield Gate command execution")
        .map_err(|message| ShieldGateError {
            code: "volatile_persistence_command_blocked",
            boundary: "PersistentStateEngine",
            message,
        })
}

fn action_observability_summary(action: &RequestedAction) -> String {
    serde_json::json!({
        "kind": crate::redaction::redacted_log_text(&action.kind),
        "principalSupplied": action.principal.as_ref().is_some_and(|value| !value.trim().is_empty()),
        "pathSupplied": action.path.as_ref().is_some_and(|value| !value.trim().is_empty()),
        "contentBytes": action.content.as_ref().map_or(0, |value| value.len()),
    })
    .to_string()
}

fn shield_error_observability(error: &ShieldGateError) -> String {
    serde_json::json!({
        "code": error.code,
        "boundary": error.boundary,
    })
    .to_string()
}

fn action_output_observability_summary(output: &ExecuteCommandResponse) -> String {
    serde_json::json!({
        "operation": crate::redaction::redacted_log_text(&output.operation),
        "status": output.status.as_str(),
        "verified": output.verified,
        "claimCount": output.claims.len(),
        "messageBytes": output.message.len(),
    })
    .to_string()
}

#[tauri::command]
pub async fn execute_command(
    request: ExecuteCommandRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    scope_trust: tauri::State<'_, ScopeTrustManager>,
    leases: tauri::State<'_, ActuationLeaseManager>,
    app: tauri::AppHandle,
) -> Result<ExecuteCommandResponse, ShieldGateError> {
    if native_file_authority::is_native_file_access_kind(&request.action.kind) {
        native_file_authority::reject_renderer_certificate(request.logical_certificate.as_ref())?;
        return Err(security_boundary_violation(
            "Use the native read-only file access boundary for this operation.".to_string(),
        ));
    }
    execute_command_with_native_authority(
        request,
        None,
        persistence,
        identity,
        approvals,
        scope_trust,
        leases,
        app,
    )
    .await
}

#[tauri::command]
pub async fn execute_native_file_access(
    request: NativeDirectFileAccessRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    scope_trust: tauri::State<'_, ScopeTrustManager>,
    leases: tauri::State<'_, ActuationLeaseManager>,
    app: tauri::AppHandle,
) -> Result<ExecuteCommandResponse, ShieldGateError> {
    let command = request.into_observed_command(persistence.inner())?;
    execute_native_file_access_command(
        command,
        persistence,
        identity,
        approvals,
        scope_trust,
        leases,
        app,
    )
    .await
}

pub(crate) async fn execute_native_file_access_command(
    mut request: ExecuteCommandRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    scope_trust: tauri::State<'_, ScopeTrustManager>,
    leases: tauri::State<'_, ActuationLeaseManager>,
    app: tauri::AppHandle,
) -> Result<ExecuteCommandResponse, ShieldGateError> {
    require_durable_direct_command(persistence.inner())?;
    DirectCommandTurnGuard::validate_accepted(persistence.inner(), &request)?;
    let authority = issue_native_file_authority(&mut request, identity.inner())?;
    execute_command_with_native_authority(
        request,
        Some(authority),
        persistence,
        identity,
        approvals,
        scope_trust,
        leases,
        app,
    )
    .await
}

fn issue_native_file_authority(
    request: &mut ExecuteCommandRequest,
    identity: &SovereignIdentity,
) -> Result<NativeFileAuthorityEnvelope, ShieldGateError> {
    request.action = normalize_directory_read_action(request.action.clone());
    native_file_authority::issue(request, identity)
}

async fn record_direct_command_failure(
    persistence: &PersistenceEngine,
    operation: &str,
    input: &str,
    error: &ShieldGateError,
) {
    let summary = shield_error_observability(error);
    let _ = persistence
        .save_action_result(
            "direct-command".to_string(),
            operation.to_string(),
            input.to_string(),
            Some(summary.clone()),
            "failed".to_string(),
        )
        .await;
    log_certificate(operation, input, &summary);
}

async fn execute_command_with_native_authority(
    mut request: ExecuteCommandRequest,
    native_authority: Option<NativeFileAuthorityEnvelope>,
    persistence: tauri::State<'_, PersistenceEngine>,
    identity: tauri::State<'_, SovereignIdentity>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    scope_trust: tauri::State<'_, ScopeTrustManager>,
    leases: tauri::State<'_, ActuationLeaseManager>,
    app: tauri::AppHandle,
) -> Result<ExecuteCommandResponse, ShieldGateError> {
    require_durable_direct_command(persistence.inner())?;
    let mut turn_guard = terminal_runtime::begin(persistence.inner(), &mut request)?;
    let operation = request.action.kind.clone();
    let input = action_observability_summary(&request.action);
    let authority_result = if native_file_authority::is_native_file_access_kind(&operation) {
        native_authority
            .as_ref()
            .ok_or_else(|| {
                security_boundary_violation(
                    "Native file authority was not issued for this operation.".to_string(),
                )
            })
            .and_then(|authority| {
                native_file_authority::consume(&request, authority, identity.inner())
            })
    } else {
        validate_host_access_certificate(
            request.action.kind.as_str(),
            request.logical_certificate.as_ref(),
            identity.inner(),
        )
    };
    if let Err(error) = authority_result {
        record_direct_command_failure(persistence.inner(), &operation, &input, &error).await;
        return Err(error);
    }

    let (mut prepared_external_action, bound_action) =
        match prepare_external_filesystem_binding(&request.action)? {
            Some((prepared, bound_action)) => (Some(prepared), bound_action),
            None => (None, request.action.clone()),
        };
    let action_class = reviewed_action_class(&bound_action.kind);
    let mandatory_reconfirm = crate::approval_scopes::mandatory_reconfirmation(&action_class);
    let reviewed_resource = reviewed_resource_for_action(&bound_action, &action_class)?;
    let reviewed_argument_class =
        crate::approval_scopes::argument_class(&action_class, &approval_preview(&bound_action));
    let mut reviewed_scope_authorized = crate::approval_scopes::authorize(
        persistence.inner(),
        reviewed_principal(&bound_action, request.agent_id.as_deref()),
        request.project_id.as_deref(),
        request.task_run_id.as_deref(),
        &action_class,
        &reviewed_resource,
        &reviewed_argument_class,
        1,
    )
    .map_err(|message| ShieldGateError {
        code: "approval_scope_persistence_failed",
        boundary: "ReviewedApprovalScope",
        message,
    })?;
    // Honor grants created before filesystem operations shared permission classes.
    if !reviewed_scope_authorized && action_class == "filesystem_write" {
        let legacy_action_class = normalize_action_kind(&bound_action.kind);
        let legacy_argument_class = crate::approval_scopes::argument_class(
            &legacy_action_class,
            &approval_preview(&bound_action),
        );
        reviewed_scope_authorized = crate::approval_scopes::authorize(
            persistence.inner(),
            reviewed_principal(&bound_action, request.agent_id.as_deref()),
            request.project_id.as_deref(),
            request.task_run_id.as_deref(),
            &legacy_action_class,
            &reviewed_resource,
            &legacy_argument_class,
            1,
        )
        .map_err(|message| ShieldGateError {
            code: "approval_scope_persistence_failed",
            boundary: "ReviewedApprovalScope",
            message,
        })?;
    }
    let trust_decision = match if mandatory_reconfirm {
        Ok(SovereignTrustDecision::PromptRequired)
    } else {
        evaluate_sovereign_trust_for_action(
            persistence.inner(),
            &bound_action,
            request.session_id.as_deref(),
        )
    } {
        Ok(decision) => decision,
        Err(error) => {
            record_direct_command_failure(persistence.inner(), &operation, &input, &error).await;
            return Err(error);
        }
    };

    let mut trusted_execution = match trust_decision {
        SovereignTrustDecision::Trusted(trusted) => Some(trusted),
        SovereignTrustDecision::PromptRequired => None,
    };
    let mut authorization_context = if reviewed_scope_authorized {
        ShieldAuthorizationContext::one_time(true)
    } else if let Some(trusted) = trusted_execution.as_ref() {
        ShieldAuthorizationContext {
            shield_approved: true,
            trusted_working_directory: Some(trusted.grant.canonical_directory_path.clone()),
        }
    } else if !mandatory_reconfirm
        && scope_trust.allows_action_for_principal(
            &bound_action,
            reviewed_principal(&bound_action, request.agent_id.as_deref()),
        )?
    {
        ShieldAuthorizationContext::one_time(true)
    } else {
        let shield_approved = match request_direct_command_approval(
            &app,
            approvals.inner(),
            &bound_action,
            request.session_id.as_deref(),
            request.turn_id.as_deref(),
            request.generation_token.as_deref(),
            request.project_id.as_deref(),
            request.task_run_id.as_deref(),
            request.agent_id.as_deref(),
        )
        .await
        {
            Ok(approved) => approved,
            Err(error) => {
                record_direct_command_failure(persistence.inner(), &operation, &input, &error)
                    .await;
                return Err(error);
            }
        };
        ShieldAuthorizationContext::one_time(shield_approved)
    };

    let authorized_action_result = match prepared_external_action.take() {
        Some(action) => Ok(action),
        None => authorize_action_for_execution_with_context(
            bound_action.clone(),
            &authorization_context,
        ),
    };
    let mut authorized_action = match authorized_action_result {
        Ok(action) => action,
        Err(error) => {
            record_direct_command_failure(persistence.inner(), &operation, &input, &error).await;
            return Err(error);
        }
    };

    if trusted_execution.is_some() {
        let active_actor_id = crate::authority::current_actor_id(identity.inner())?;
        match leases.evaluate_autonomous_action(
            Some(&app),
            Some(&active_actor_id),
            request.session_id.as_deref(),
            &authorized_action,
        ) {
            Ok(ActuationLeaseOutcome::NotRequired | ActuationLeaseOutcome::Authorized(_)) => {}
            Ok(ActuationLeaseOutcome::Blocked(_, _)) => {
                let shield_approved = match request_direct_command_approval(
                    &app,
                    approvals.inner(),
                    &bound_action,
                    request.session_id.as_deref(),
                    request.turn_id.as_deref(),
                    request.generation_token.as_deref(),
                    request.project_id.as_deref(),
                    request.task_run_id.as_deref(),
                    request.agent_id.as_deref(),
                )
                .await
                {
                    Ok(approved) => approved,
                    Err(error) => {
                        record_direct_command_failure(
                            persistence.inner(),
                            &operation,
                            &input,
                            &error,
                        )
                        .await;
                        return Err(error);
                    }
                };
                authorization_context = ShieldAuthorizationContext::one_time(shield_approved);
                trusted_execution = None;
                if !matches!(
                    authorized_action,
                    AuthorizedActions::ApprovedExternalFileRead(_)
                        | AuthorizedActions::ApprovedExternalFileList(_)
                        | AuthorizedActions::ApprovedExternalFileWrite(_)
                ) {
                    authorized_action = match authorize_action_for_execution_with_context(
                        bound_action.clone(),
                        &authorization_context,
                    ) {
                        Ok(action) => action,
                        Err(error) => {
                            record_direct_command_failure(
                                persistence.inner(),
                                &operation,
                                &input,
                                &error,
                            )
                            .await;
                            return Err(error);
                        }
                    };
                }
            }
            Err(error) => {
                record_direct_command_failure(persistence.inner(), &operation, &input, &error)
                    .await;
                return Err(error);
            }
        };
    }
    let operation = authorized_action.operation_name();

    let action_id = persistence
        .save_action_result(
            "direct-command".to_string(),
            operation.to_string(),
            input.clone(),
            None,
            "running".to_string(),
        )
        .await
        .map_err(|error| ShieldGateError {
            code: "persistence_error",
            boundary: "PersistentStateEngine",
            message: error,
        })?;
    let trusted_started = trusted_execution.as_ref().map(|_| Instant::now());
    let native_file_attempt =
        prepare_receipt(turn_guard.as_ref(), operation, &bound_action).await?;
    let mut output = match authorized_action {
        AuthorizedActions::AirlockExport(request) => crate::airlock::Airlock::new(project_root())
            .export_sync(request, identity.inner())
            .unwrap_or_else(|message| {
                ExecuteCommandResponse::from_tool_error(ToolError {
                    operation: "airlock_export".to_string(),
                    message,
                })
            }),
        AuthorizedActions::CodebaseCompile(request) => {
            crate::native_runtime::execute_codebase_compile(&app, request).await
        }
        AuthorizedActions::ApprovedExternalFileRead(request) => {
            tokio::task::spawn_blocking(move || handle_approved_external_file_read(request))
                .await
                .unwrap_or_else(|_| {
                    ExecuteCommandResponse::from_tool_error(ToolError {
                        operation: "file_read".to_string(),
                        message: "The approved file could not be viewed safely.".to_string(),
                    })
                })
        }
        action => handle_authorized_action(action),
    };
    native_file_receipt::finish(native_file_attempt, &output).await;
    if let Some(trusted) = trusted_execution {
        let observed_elapsed_wall_seconds = trusted_started
            .map(|started| started.elapsed().as_secs_f64())
            .unwrap_or_default();
        let reserved_cpu_seconds =
            observed_elapsed_wall_seconds.max(trusted.estimated_cpu_seconds_reservation);
        persistence
            .record_sovereign_trust_usage(
                &trusted.grant,
                trusted.estimated_token_cost,
                reserved_cpu_seconds,
                unix_time_ms_i64(),
            )
            .map_err(|error| ShieldGateError {
                code: "persistence_error",
                boundary: "SovereignTrustEngine",
                message: error.to_string(),
            })?;
        output.claims.push(sovereign_trust_reservation_claim(
            trusted.grant.permission_level.as_str(),
            &trusted.grant.canonical_directory_path,
            &trusted.grant.directory_path,
            trusted.estimated_token_cost,
            reserved_cpu_seconds,
            observed_elapsed_wall_seconds,
        ));
    }
    if output.verified
        && output.status.as_str() == "completed"
        && matches!(operation, "file_read" | "file_list" | "file_write")
    {
        if let (Some(guard), Some(path)) = (turn_guard.as_ref(), bound_action.path.as_deref()) {
            let target_kind = if operation == "file_list" {
                "directory"
            } else {
                "file"
            };
            if let Err(error) = persistence.record_verified_filesystem_context(
                &guard.context,
                operation,
                path,
                target_kind,
                identity.inner(),
            ) {
                eprintln!(
                    "VERIFIED_FILESYSTEM_CONTEXT_SKIPPED operation={} reason={}",
                    operation,
                    crate::redaction::redacted_log_text(&error)
                );
            }
        }
    }
    let output_json = action_output_observability_summary(&output);
    persistence
        .update_action_result(
            action_id,
            Some(output_json),
            output.status.as_str().to_string(),
        )
        .await
        .map_err(|error| ShieldGateError {
            code: "persistence_error",
            boundary: "PersistentStateEngine",
            message: error,
        })?;
    log_certificate(
        operation,
        &input,
        &action_output_observability_summary(&output),
    );

    if let Some(guard) = turn_guard.as_mut() {
        guard.finalize_output(&output)?;
    }

    Ok(output)
}

fn reviewed_resource_for_action(
    action: &RequestedAction,
    action_class: &str,
) -> Result<String, ShieldGateError> {
    let normalized_kind = normalize_action_kind(&action.kind);
    let resolved_external_target = resolve_external_action_target(action, &normalized_kind);
    match resolved_external_target {
        Some(result) => result.map(|path| path.display().to_string()),
        None => Ok(crate::approval_scopes::canonical_resource(
            action.path.as_deref(),
            action_class,
        )),
    }
}

pub(crate) fn verify_gateway_message_allowlist(
    persistence: &PersistenceEngine,
    message: &GatewayIncomingMessage,
) -> Result<GatewayFirewallDecision, ShieldGateError> {
    let platform =
        normalize_channel_platform(&message.platform).map_err(|error| ShieldGateError {
            code: "gateway_invalid_platform",
            boundary: "GatewayShieldGate",
            message: error.to_string(),
        })?;
    let config = persistence
        .select_channel_config(&platform)
        .map_err(|error| ShieldGateError {
            code: "persistence_error",
            boundary: "GatewayShieldGate",
            message: error.to_string(),
        })?;

    let Some(config) = config else {
        eprintln!("SOVEREIGN_GATEWAY_SECURITY_WARNING platform={platform} reason=missing_config");
        return Ok(GatewayFirewallDecision {
            allowed: false,
            reason: "missing_config".to_string(),
        });
    };
    if !config.is_active {
        eprintln!("SOVEREIGN_GATEWAY_SECURITY_WARNING platform={platform} reason=inactive_channel");
        return Ok(GatewayFirewallDecision {
            allowed: false,
            reason: "inactive_channel".to_string(),
        });
    }
    let owner_id = config
        .owner_id
        .and_then(|owner| clean_gateway_owner_id(Some(owner.as_str())))
        .unwrap_or_default();
    if owner_id.trim().is_empty() {
        eprintln!("SOVEREIGN_GATEWAY_SECURITY_WARNING platform={platform} reason=owner_unset");
        return Ok(GatewayFirewallDecision {
            allowed: false,
            reason: "owner_unset".to_string(),
        });
    }
    if owner_id.trim() != message.sender_id.trim() {
        eprintln!(
            "SOVEREIGN_GATEWAY_SECURITY_WARNING platform={platform} reason=unauthorized_sender"
        );
        return Ok(GatewayFirewallDecision {
            allowed: false,
            reason: "unauthorized_sender".to_string(),
        });
    }

    Ok(GatewayFirewallDecision {
        allowed: true,
        reason: "authorized_owner".to_string(),
    })
}

fn clean_gateway_owner_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn filter_gateway_remote_actions(
    actions: &[RequestedAction],
) -> RemoteActionFilterResponse {
    let mut auto_approved_actions = Vec::new();
    let mut confirmation_required_actions = Vec::new();
    let mut blocked_actions = Vec::new();

    for action in actions {
        match classify_remote_action_trust_level(action) {
            RemoteExecutionTrustLevel::AutoApproved => {
                auto_approved_actions.push(action.clone());
            }
            RemoteExecutionTrustLevel::RequiresInteractiveConfirmation => {
                confirmation_required_actions.push(action.clone());
            }
            RemoteExecutionTrustLevel::Blocked => {
                blocked_actions.push(RemoteBlockedAction {
                    action: action.clone(),
                    reason: REMOTE_LEVEL_THREE_BLOCK_MESSAGE.to_string(),
                    trust_level: RemoteExecutionTrustLevel::Blocked.as_str().to_string(),
                });
            }
        }
    }

    RemoteActionFilterResponse {
        auto_approved_actions,
        confirmation_required_actions,
        response_message: if blocked_actions.is_empty() {
            None
        } else {
            Some(REMOTE_LEVEL_THREE_BLOCK_MESSAGE.to_string())
        },
        blocked_actions,
    }
}

fn classify_remote_action_trust_level(action: &RequestedAction) -> RemoteExecutionTrustLevel {
    match normalize_action_kind(&action.kind).as_str() {
        "get_system_metrics" | "system_audit" => RemoteExecutionTrustLevel::AutoApproved,
        "file_read" | "file_list" if remote_planning_read_allowed(action) => {
            RemoteExecutionTrustLevel::AutoApproved
        }
        "file_write" if remote_workspace_write_requires_confirmation(action) => {
            RemoteExecutionTrustLevel::RequiresInteractiveConfirmation
        }
        "codebase_patch" if remote_codebase_patch_requires_confirmation(action) => {
            RemoteExecutionTrustLevel::RequiresInteractiveConfirmation
        }
        "terminal_execute" | "shell_command" | "execute_command" | "codebase_compile"
        | "delete_file" | "trash" | "trash_file" | "telemetry_archive" | "airlock_export" => {
            RemoteExecutionTrustLevel::Blocked
        }
        _ => RemoteExecutionTrustLevel::Blocked,
    }
}

fn remote_planning_read_allowed(action: &RequestedAction) -> bool {
    let Some(path) = action.path.as_deref() else {
        return false;
    };
    resolve_diagnostic_read_path(&action.kind, path)
        .map(|resolved| path_contains_component(&resolved, "planning"))
        .unwrap_or(false)
}

fn remote_workspace_write_requires_confirmation(action: &RequestedAction) -> bool {
    action
        .path
        .as_deref()
        .is_some_and(|path| validate_project_quarantine(path, "file_write").is_ok())
}

fn remote_codebase_patch_requires_confirmation(action: &RequestedAction) -> bool {
    action
        .path
        .as_deref()
        .is_some_and(|path| validate_codebase_patch_target(path).is_ok())
}

fn path_contains_component(path: &Path, expected: &str) -> bool {
    path.components().any(|component| {
        let Component::Normal(part) = component else {
            return false;
        };
        part.to_string_lossy().eq_ignore_ascii_case(expected)
    })
}

pub fn authorize_action(action: RequestedAction) -> Result<AuthorizedActions, ShieldGateError> {
    authorize_action_for_execution(action, false)
}

pub(crate) fn authorize_action_for_approved_plan(
    action: RequestedAction,
) -> Result<AuthorizedActions, ShieldGateError> {
    authorize_action_for_execution(action, true)
}

fn authorize_action_for_execution(
    action: RequestedAction,
    shield_approved: bool,
) -> Result<AuthorizedActions, ShieldGateError> {
    authorize_action_for_execution_with_context(
        action,
        &ShieldAuthorizationContext::one_time(shield_approved),
    )
}

fn authorize_action_for_execution_with_context(
    action: RequestedAction,
    context: &ShieldAuthorizationContext,
) -> Result<AuthorizedActions, ShieldGateError> {
    action_authorization::authorize(action, context)
}

pub fn verify_visual_workflow_integrity(
    nodes: &[VisualWorkflowNode],
) -> Result<(), ShieldGateError> {
    use std::collections::{HashMap, HashSet};

    if nodes.is_empty() {
        return Err(ShieldGateError {
            code: "shield_gate_invalid_input",
            boundary: "VisualWorkflowIntegrity",
            message: "Visual workflow requires at least one block.".to_string(),
        });
    }

    let mut ids = HashSet::new();
    for node in nodes {
        if !ids.insert(node.id.as_str()) {
            return Err(security_boundary_violation(format!(
                "Visual workflow contains duplicate block id {}.",
                node.id
            )));
        }
        if matches!(
            node.action_kind.as_str(),
            "file_read" | "file_list" | "file_write"
        ) {
            let path = node.path.as_deref().ok_or_else(|| {
                security_boundary_violation(format!(
                    "{} visual block {} is missing a quarantined path.",
                    node.action_kind, node.id
                ))
            })?;
            validate_project_quarantine(path, &node.action_kind)?;
        }
    }

    for node in nodes {
        for dependency in &node.dependencies {
            if !ids.contains(dependency.as_str()) {
                return Err(security_boundary_violation(format!(
                    "Visual workflow block {} references unknown dependency {}.",
                    node.id, dependency
                )));
            }
        }
    }

    let graph = nodes
        .iter()
        .map(|node| (node.id.as_str(), node.dependencies.as_slice()))
        .collect::<HashMap<_, _>>();
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();

    for node in nodes {
        detect_visual_cycle(node.id.as_str(), &graph, &mut visiting, &mut visited)?;
    }

    Ok(())
}

fn detect_visual_cycle<'a>(
    node_id: &'a str,
    graph: &std::collections::HashMap<&'a str, &'a [String]>,
    visiting: &mut std::collections::HashSet<&'a str>,
    visited: &mut std::collections::HashSet<&'a str>,
) -> Result<(), ShieldGateError> {
    if visited.contains(node_id) {
        return Ok(());
    }
    if !visiting.insert(node_id) {
        return Err(security_boundary_violation(format!(
            "Visual workflow contains a circular dependency at block {node_id}."
        )));
    }

    if let Some(dependencies) = graph.get(node_id) {
        for dependency in *dependencies {
            detect_visual_cycle(dependency.as_str(), graph, visiting, visited)?;
        }
    }

    visiting.remove(node_id);
    visited.insert(node_id);
    Ok(())
}

pub(crate) fn build_shield_approval_request(
    action: &RequestedAction,
) -> Option<ShieldApprovalRequest> {
    let action = normalize_directory_read_action(action.clone());
    let normalized_kind = normalize_action_kind(&action.kind);
    let mut approval_tier = classify_registered_system_tool(&normalized_kind)?;
    let term_target = if matches!(
        normalized_kind.as_str(),
        "terminal_execute" | "shell_command" | "execute_command"
    ) {
        let request = terminal_request(&action).ok()?;
        let project_root = development_repo_root()
            .canonicalize()
            .unwrap_or_else(|_| development_repo_root());
        if request.prompt_free_in_project(&project_root) {
            approval_tier = ShieldToolApprovalTier::BackgroundAutoApproval;
            None
        } else if let Some(target) = request.external_read_target(&project_root) {
            approval_tier = ShieldToolApprovalTier::VisualConsent;
            Some(target)
        } else {
            approval_tier = ShieldToolApprovalTier::ExplicitConfirmation;
            None
        }
    } else {
        None
    };
    if matches!(normalized_kind.as_str(), "file_read" | "file_list")
        && action
            .path
            .as_deref()
            .is_some_and(|path| resolve_read_only_action_path(&normalized_kind, path).is_err())
    {
        approval_tier = ShieldToolApprovalTier::VisualConsent;
    }
    if !approval_tier.requires_user_confirmation() {
        return None;
    }

    let mut semantics = semantic_action_description(&action, &normalized_kind, approval_tier)?;
    let resolved_external_target = resolve_external_action_target(&action, &normalized_kind)
        .transpose()
        .ok()?;
    if let Some(resolved) = term_target.or(resolved_external_target) {
        semantics.target_path = Some(resolved.display().to_string());
    }
    let scope_trust_prefix = if approval_tier == ShieldToolApprovalTier::VisualConsent {
        scope_trust_prefix_for_action(&action)
            .ok()
            .flatten()
            .map(|path| path.display().to_string())
    } else {
        None
    };
    let scope_trust_available = scope_trust_prefix.is_some();

    let action_class = if approval_tier == ShieldToolApprovalTier::VisualConsent
        && matches!(
            normalized_kind.as_str(),
            "terminal_execute" | "shell_command" | "execute_command"
        ) {
        "filesystem_read".to_string()
    } else {
        reviewed_action_class(&normalized_kind)
    };
    let canonical_resource = semantics
        .target_path
        .as_deref()
        .map(|path| crate::approval_scopes::canonical_resource(Some(path), &action_class));
    let argument_class =
        crate::approval_scopes::argument_class(&action_class, &approval_preview(&action));
    let mandatory_reconfirm = !(approval_tier == ShieldToolApprovalTier::VisualConsent
        && matches!(
            normalized_kind.as_str(),
            "terminal_execute" | "shell_command" | "execute_command"
        ))
        && crate::approval_scopes::mandatory_reconfirmation(&normalized_kind);
    let mut request = ShieldApprovalRequest {
        approval_token: new_approval_token(),
        session_id: None,
        turn_id: None,
        generation_token: None,
        action_type: action.kind.clone(),
        action_label: semantics.action_label,
        target_path: semantics.target_path,
        principal: action.principal.clone(),
        risk_tier: approval_tier.risk_label().to_string(),
        reason: semantics.reason,
        estimated_token_costs: Some(estimate_action_token_costs(&action)),
        requested_at_ms: unix_time_ms_u64(),
        preview: approval_preview(&action),
        semantic_summary: semantics.summary,
        semantic_detail: semantics.detail,
        approval_tier: approval_tier.as_str().to_string(),
        approval_mode: approval_tier.approval_mode().to_string(),
        diff_preview: visual_diff_preview(&action),
        scope_trust_available,
        scope_trust_prefix,
        scope_trust_duration_ms: DEFAULT_SCOPE_TRUST_DURATION_MS,
        project_id: None,
        task_run_id: None,
        action_class,
        argument_class,
        canonical_resource,
        mandatory_reconfirm,
        approval_scope_kinds: vec!["once".to_string()],
    };
    request.approval_scope_kinds = available_scope_kinds(&request);
    Some(request)
}

fn available_scope_kinds(request: &ShieldApprovalRequest) -> Vec<String> {
    let mut kinds = vec!["once".to_string()];
    if request.mandatory_reconfirm || !request.scope_trust_available {
        return kinds;
    }
    if matches!(
        request.action_class.as_str(),
        "filesystem_read" | "filesystem_write"
    ) {
        kinds.push("app_session".to_string());
        if request
            .scope_trust_prefix
            .as_deref()
            .is_some_and(folder_scope_allows_persistent_access)
        {
            kinds.push("persistent".to_string());
        }
        return kinds;
    }
    if request.task_run_id.is_some() {
        kinds.push("task".to_string());
    }
    if request.project_id.is_some() && request.canonical_resource.is_some() {
        kinds.push("project_path".to_string());
    }
    if request.canonical_resource.is_some() {
        kinds.push("persistent".to_string());
    }
    kinds
}

fn folder_scope_allows_persistent_access(path: &str) -> bool {
    let Ok(canonical) = fs::canonicalize(path) else {
        return false;
    };
    if canonical.parent().is_none() {
        return false;
    }

    // Permanent authority is for user-managed working folders, never operating
    // system locations. External volumes remain eligible, but their mount root
    // itself does not.
    for protected_root in [
        "/System",
        "/Library",
        "/Applications",
        "/bin",
        "/sbin",
        "/usr",
        "/etc",
        "/private",
        "/dev",
        "/cores",
        "/opt",
        "/Network",
    ] {
        let protected_root = Path::new(protected_root);
        if canonical == protected_root || canonical.starts_with(protected_root) {
            return false;
        }
    }
    if matches!(canonical.to_str(), Some("/Users" | "/Volumes")) {
        return false;
    }

    if let Some(home) = env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|path| fs::canonicalize(path).ok())
    {
        if canonical == home {
            return false;
        }
        for sensitive in [
            ".ssh",
            ".gnupg",
            ".aws",
            ".azure",
            ".kube",
            ".config/gcloud",
            ".local/share/keyrings",
            "Library/Keychains",
            "Library/Safari",
            "Library/Mail",
            "Library/Messages",
            "Library/Application Support/Google/Chrome",
            "Library/Application Support/Firefox",
            "Library/Application Support/OOMU",
            "Library/Application Support/OOMU",
            "Library/Application Support/OOMU",
            "Library/Application Support/ai.eldris.oomu.gpd",
            "Library/Containers/ai.eldris.oomu.gpd",
        ] {
            let sensitive = home.join(sensitive);
            if canonical == sensitive
                || canonical.starts_with(&sensitive)
                || sensitive.starts_with(&canonical)
            {
                return false;
            }
        }
    }
    true
}

struct ShieldActionSemantics {
    action_label: String,
    summary: String,
    detail: String,
    reason: String,
    target_path: Option<String>,
}

fn semantic_action_description(
    action: &RequestedAction,
    normalized_kind: &str,
    approval_tier: ShieldToolApprovalTier,
) -> Option<ShieldActionSemantics> {
    let target_path = approval_target_path(action, normalized_kind);
    let target_name = target_path
        .as_deref()
        .map(friendly_path_name)
        .unwrap_or_else(|| "the requested action".to_string());
    let risk_suffix = approval_tier.risk_label();
    if let Some(semantics) = system_action_semantics::delegated(action, normalized_kind) {
        return semantics;
    }
    let semantics = match normalized_kind {
        "file_read" => ShieldActionSemantics {
            action_label: "View a local file".to_string(),
            summary: format!("Use {target_name} to finish this request."),
            detail: "OOMU will view this file. It will not change or delete it."
                .to_string(),
            reason: "This file is outside the folders OOMU can currently use."
                .to_string(),
            target_path,
        },
        "file_list" => ShieldActionSemantics {
            action_label: "View a local folder".to_string(),
            summary: format!("Use {target_name} to finish this request."),
            detail: "OOMU will view the names of files and folders here. Nothing will be changed."
                .to_string(),
            reason: "This folder is outside the folders OOMU can currently use."
                .to_string(),
            target_path,
        },
        "file_write" => ShieldActionSemantics {
            action_label: "Write a local file".to_string(),
            summary: format!("Save proposed changes to {target_name} ({risk_suffix})."),
            detail: "OOMU will update local file content. Review the changed lines before approving."
                .to_string(),
            reason: "This changes a local file, so OOMU is asking for a quick visual check."
                .to_string(),
            target_path,
        },
        "configure_channel" => system_action_semantics::configure_channel(action),
        "create_system_calendar_event" => system_action_semantics::calendar_event(action)?,
        "create_conflict_free_calendar_event" => {
            system_action_semantics::conflict_free_calendar_event(action)?
        }
        "create_decision_pack" => system_action_semantics::decision_pack(action)?,
        "create_file"
        | "prepare_background_agent_comparison"
        | "prepare_milestone_constraint_recovery_plan" => ShieldActionSemantics {
            action_label: "Create a local file".to_string(),
            summary: format!("Create {target_name}."),
            detail: "OOMU will create this file. It will not replace an existing file."
                .to_string(),
            reason: "This location is outside the folders OOMU can currently use."
                .to_string(),
            target_path,
        },
        "codebase_patch" => ShieldActionSemantics {
            action_label: "Patch repository source".to_string(),
            summary: format!("Apply a source patch to {target_name} ({risk_suffix})."),
            detail: "OOMU will replace a matched block in the active repository. The diff shows the proposed edit."
                .to_string(),
            reason: "This changes source code in the active project.".to_string(),
            target_path,
        },
        "document_index" => ShieldActionSemantics {
            action_label: "Update the local document index".to_string(),
            summary: format!("Refresh the document index for {target_name} ({risk_suffix})."),
            detail: "OOMU will update local index data used for document search.".to_string(),
            reason: "This updates local application state.".to_string(),
            target_path,
        },
        "delete_file" | "trash" | "trash_file" => ShieldActionSemantics {
            action_label: "Remove a local file".to_string(),
            summary: format!("Remove {target_name} from local storage ({risk_suffix})."),
            detail: "OOMU will remove the selected file after approval.".to_string(),
            reason: "This removes local content and needs explicit confirmation.".to_string(),
            target_path,
        },
        "telemetry_archive" => ShieldActionSemantics {
            action_label: "Package a diagnostics archive".to_string(),
            summary: format!("Write a diagnostics archive to {target_name} ({risk_suffix})."),
            detail: "OOMU will package local diagnostics into the selected output path.".to_string(),
            reason: "This writes a local archive outside the regular project flow.".to_string(),
            target_path,
        },
        "airlock_export" => ShieldActionSemantics {
            action_label: "Export a project artifact".to_string(),
            summary: format!("Export an artifact to {target_name} ({risk_suffix})."),
            detail: "OOMU will copy a prepared artifact to the selected mount path.".to_string(),
            reason: "This writes content to an external destination.".to_string(),
            target_path,
        },
        "terminal_execute" | "shell_command" | "execute_command" => ShieldActionSemantics {
            action_label: "Run a local command".to_string(),
            summary: format!("Run a local command ({risk_suffix})."),
            detail: "OOMU will run the shown command in the local runtime.".to_string(),
            reason: "Local commands can change files or process state, so this needs explicit confirmation."
                .to_string(),
            target_path,
        },
        "web_fetch" | "network_request" => ShieldActionSemantics {
            action_label: "Access a network destination".to_string(),
            summary: format!("Access {target_name} over the network ({risk_suffix})."),
            detail: "OOMU will contact the shown network destination after approval.".to_string(),
            reason: "This can send request data outside the local runtime.".to_string(),
            target_path,
        },
        "mcp_connect_server" => ShieldActionSemantics {
            action_label: "Connect a tool provider".to_string(),
            summary: format!("Connect {target_name} as a tool provider ({risk_suffix})."),
            detail: "OOMU will attach the selected provider to the local tool runtime.".to_string(),
            reason: "New tool providers can expand what the runtime can do.".to_string(),
            target_path,
        },
        "mcp_execute_remote_tool" => ShieldActionSemantics {
            action_label: "Run a remote tool".to_string(),
            summary: format!("Send the reviewed call to {target_name} ({risk_suffix})."),
            detail: "OOMU will send the shown redacted arguments to exactly one approved remote MCP tool call. Destination, server identity, certificate, schema, arguments, and audit receipt are bound to the one-use authority."
                .to_string(),
            reason: "Remote tools can send data outside the local runtime and always require native confirmation."
                .to_string(),
            target_path,
        },
        _ => return None,
    };
    Some(semantics)
}

fn approval_target_path(action: &RequestedAction, normalized_kind: &str) -> Option<String> {
    match normalized_kind {
        "terminal_execute" | "shell_command" | "execute_command" => terminal_request(action)
            .ok()
            .and_then(|request| {
                let root = development_repo_root()
                    .canonicalize()
                    .unwrap_or_else(|_| development_repo_root());
                request.external_read_target(&root)
            })
            .map(|path| path.display().to_string()),
        "telemetry_archive" => action
            .path
            .clone()
            .or_else(|| action.content.clone())
            .or_else(|| action.principal.clone()),
        "delete_file" | "trash" | "trash_file" => action
            .path
            .clone()
            .or_else(|| action.content.clone())
            .or_else(|| action.principal.clone()),
        "codebase_compile" => action
            .path
            .clone()
            .or_else(|| action.principal.clone())
            .or_else(|| action.content.clone()),
        _ => action
            .path
            .clone()
            .or_else(|| action.principal.clone())
            .or_else(|| action.content.clone()),
    }
}

fn friendly_path_name(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "the selected target".to_string();
    }
    Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| trimmed.to_string())
}

fn new_approval_token() -> String {
    let mut bytes = [0_u8; 24];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn estimate_action_token_costs(action: &RequestedAction) -> usize {
    let byte_count = action.kind.len()
        + action.principal.as_deref().unwrap_or_default().len()
        + action.path.as_deref().unwrap_or_default().len()
        + action.content.as_deref().unwrap_or_default().len();
    (byte_count / 4).max(1)
}

fn approval_preview(action: &RequestedAction) -> String {
    if let Some(preview) = system_action_semantics::direct_preview(action) {
        return preview;
    }
    let preview = match action.kind.as_str() {
        "terminal_execute" | "shell_command" | "execute_command" => {
            return terminal_request(action)
                .map(|request| truncate_for_receipt(&request.display_command(), 700))
                .unwrap_or_else(|_| "Run the selected terminal command".to_string())
        }
        "mcp_connect_server" | "mcp_execute_remote_tool" => action
            .content
            .as_deref()
            .or(action.path.as_deref())
            .or(action.principal.as_deref())
            .unwrap_or("mcp_connect_server"),
        "file_write" => action.content.as_deref().unwrap_or("file write"),
        "configure_channel" => {
            return system_action_semantics::configure_channel_preview(action)
                .map(|value| value.to_string())
                .unwrap_or_else(|| {
                    "{\"platform\":\"messaging\",\"ownerId\":\"unknown\",\"isActive\":false}"
                        .to_string()
                });
        }
        "create_decision_pack" => {
            return system_action_semantics::decision_pack_preview(action)
                .map(|value| value.to_string())
                .unwrap_or_default()
        }
        _ => action.path.as_deref().unwrap_or(action.kind.as_str()),
    };
    let limit = if normalize_action_kind(&action.kind) == "mcp_execute_remote_tool" {
        MAX_REMOTE_MCP_APPROVAL_PREVIEW_CHARS
    } else {
        700
    };
    truncate_for_receipt(preview, limit)
}

fn visual_diff_preview(action: &RequestedAction) -> Option<String> {
    match normalize_action_kind(&action.kind).as_str() {
        "file_write" => {
            let path = action.path.as_deref()?;
            let new_content = action.content.as_deref().unwrap_or_default();
            // Preparing a permission prompt must never read an external file
            // before the user has granted access to it.
            let old_content = if validate_project_quarantine(path, "file_write").is_ok() {
                scope_target_for_path(path, "file_write", &project_root())
                    .ok()
                    .flatten()
                    .filter(|target| target.is_file())
                    .and_then(|target| fs::read_to_string(target).ok())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            Some(build_approval_unified_diff(path, &old_content, new_content))
        }
        "codebase_patch" => {
            let path = action.path.as_deref()?;
            let search_pattern = action.principal.as_deref()?;
            let replacement_content = action.content.as_deref().unwrap_or_default();
            let target = scope_target_for_path(path, "codebase_patch", &development_repo_root())
                .ok()
                .flatten()?;
            let old_content = fs::read_to_string(target).ok()?;
            let new_content = if old_content.contains(search_pattern) {
                old_content.replacen(search_pattern, replacement_content, 1)
            } else {
                replacement_content.to_string()
            };
            Some(build_approval_unified_diff(
                path,
                &old_content,
                &new_content,
            ))
        }
        "delete_file" | "trash" | "trash_file" => {
            let path = action
                .path
                .as_deref()
                .or(action.content.as_deref())
                .or(action.principal.as_deref())?;
            let target = scope_target_for_path(path, "delete_file", &project_root())
                .ok()
                .flatten()?;
            let old_content = fs::read_to_string(target).ok()?;
            Some(build_approval_unified_diff(path, &old_content, ""))
        }
        _ => None,
    }
}

fn build_approval_unified_diff(label: &str, old_content: &str, new_content: &str) -> String {
    let mut diff = String::new();
    diff.push_str(&format!("--- a/{label}\n"));
    diff.push_str(&format!("+++ b/{label}\n"));
    diff.push_str("@@\n");

    if old_content == new_content {
        diff.push_str(" No text changes detected.\n");
        return diff;
    }

    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();
    for line in old_lines.iter().take(MAX_APPROVAL_DIFF_LINES_PER_SIDE) {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    if old_lines.len() > MAX_APPROVAL_DIFF_LINES_PER_SIDE {
        diff.push_str("-...\n");
    }
    for line in new_lines.iter().take(MAX_APPROVAL_DIFF_LINES_PER_SIDE) {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    if new_lines.len() > MAX_APPROVAL_DIFF_LINES_PER_SIDE {
        diff.push_str("+...\n");
    }

    truncate_for_receipt(&diff, MAX_APPROVAL_DIFF_BYTES)
}

fn scope_trust_prefix_for_action(
    action: &RequestedAction,
) -> Result<Option<PathBuf>, ShieldGateError> {
    let Some((_, target_path)) = scope_trust_action_scope(action)? else {
        return Ok(None);
    };
    let prefix = if target_path.is_dir() {
        target_path
    } else if let Some(parent) = target_path.parent() {
        parent.to_path_buf()
    } else {
        return Ok(None);
    };
    if prefix.parent().is_none() {
        return Ok(None);
    }
    Ok(Some(prefix))
}

fn scope_trust_action_scope(
    action: &RequestedAction,
) -> Result<Option<(String, PathBuf)>, ShieldGateError> {
    match normalize_action_kind(&action.kind).as_str() {
        "file_read" | "file_list" => {
            let Some(path) = action.path.as_deref() else {
                return Ok(None);
            };
            Ok(
                scope_target_for_path(path, "filesystem_read", &project_root())?
                    .map(|target| ("filesystem_read".to_string(), target)),
            )
        }
        kind if external_file_binding::is_project_file_write_action(kind) => {
            let Some(path) = action.path.as_deref() else {
                return Ok(None);
            };
            Ok(
                scope_target_for_path(path, "filesystem_write", &project_root())?
                    .map(|target| ("filesystem_write".to_string(), target)),
            )
        }
        "codebase_patch" => {
            let Some(path) = action.path.as_deref() else {
                return Ok(None);
            };
            Ok(
                scope_target_for_path(path, "codebase_patch", &development_repo_root())?
                    .map(|target| ("codebase_patch".to_string(), target)),
            )
        }
        "document_index" => {
            let Some(path) = action.path.as_deref() else {
                return Ok(None);
            };
            Ok(
                scope_target_for_path(path, "document_index", &project_root())?
                    .map(|target| ("document_index".to_string(), target)),
            )
        }
        "terminal_execute" | "shell_command" | "execute_command" => {
            let request = terminal_request(action)?;
            let root = development_repo_root()
                .canonicalize()
                .unwrap_or_else(|_| development_repo_root());
            Ok(request
                .external_read_target(&root)
                .map(|target| ("filesystem_read".to_string(), target)))
        }
        _ => Ok(None),
    }
}

fn scope_target_for_path(
    path: &str,
    operation: &str,
    relative_base: &Path,
) -> Result<Option<PathBuf>, ShieldGateError> {
    let requested = expand_shield_home_path(path, operation)?;
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(security_boundary_violation(format!(
            "{operation} rejected path traversal while preparing temporary folder trust."
        )));
    }
    let candidate = if requested.is_absolute() {
        requested
    } else {
        relative_base.join(requested)
    };
    if candidate.exists() {
        return fs::canonicalize(&candidate).map(Some).map_err(|_| {
            security_boundary_violation(format!(
                "{operation} target could not be resolved for approval preview."
            ))
        });
    }
    let Some(parent) = candidate.parent() else {
        return Ok(None);
    };
    let Ok(parent_real) = fs::canonicalize(parent) else {
        return Ok(None);
    };
    let Some(file_name) = candidate.file_name() else {
        return Ok(Some(parent_real));
    };
    Ok(Some(parent_real.join(file_name)))
}

fn evaluate_sovereign_trust_for_action(
    persistence: &PersistenceEngine,
    action: &RequestedAction,
    session_id: Option<&str>,
) -> Result<SovereignTrustDecision, ShieldGateError> {
    let Some((tool_category, target_path)) = trust_scope_for_action(action)? else {
        return Ok(SovereignTrustDecision::PromptRequired);
    };
    let now_ms = unix_time_ms_i64();
    let Some(grant) = persistence
        .select_matching_sovereign_trust_grant(session_id, &target_path, tool_category, now_ms)
        .map_err(|error| ShieldGateError {
            code: "persistence_error",
            boundary: "SovereignTrustEngine",
            message: error.to_string(),
        })?
    else {
        return Ok(SovereignTrustDecision::PromptRequired);
    };

    if grant
        .expires_at_ms
        .is_some_and(|expires_at| expires_at <= now_ms)
    {
        return Ok(SovereignTrustDecision::PromptRequired);
    }
    if !matches!(
        grant.permission_level,
        SovereignTrustPermissionLevel::SessionGated | SovereignTrustPermissionLevel::GlobalTrust
    ) {
        return Ok(SovereignTrustDecision::PromptRequired);
    }

    let estimated_token_cost = estimate_action_token_costs(action) as i64;
    let estimated_cpu_seconds_reservation = estimate_action_cpu_seconds(action);
    enforce_sovereign_trust_resource_limits(
        &grant,
        estimated_token_cost,
        estimated_cpu_seconds_reservation,
        now_ms,
    )?;
    Ok(SovereignTrustDecision::Trusted(TrustedActionGrant {
        grant,
        estimated_token_cost,
        estimated_cpu_seconds_reservation,
    }))
}

fn trust_scope_for_action(
    action: &RequestedAction,
) -> Result<Option<(SovereignTrustToolCategory, PathBuf)>, ShieldGateError> {
    match action.kind.as_str() {
        "file_write" | "create_file" => {
            let Some(path) = action.path.as_deref() else {
                return Ok(None);
            };
            if action.kind == "file_write"
                && validate_project_quarantine(path, "file_write").is_ok()
            {
                return Ok(None);
            }
            let target_path = validate_approved_external_write_target(path)?;
            Ok(Some((
                SovereignTrustToolCategory::ExternalWrites,
                target_path,
            )))
        }
        "telemetry_archive" => {
            let Some(path) = action.path.as_deref() else {
                return Ok(None);
            };
            let target_path = validate_approved_external_write_target(path)?;
            Ok(Some((
                SovereignTrustToolCategory::ExternalWrites,
                target_path,
            )))
        }
        "terminal_execute" | "shell_command" | "execute_command" => Ok(Some((
            SovereignTrustToolCategory::ShellCommands,
            shell_command_trust_scope(),
        ))),
        _ => Ok(None),
    }
}

fn shell_command_trust_scope() -> PathBuf {
    let repo_root = development_repo_root();
    if repo_root.exists() {
        repo_root
    } else {
        project_root()
    }
}

fn enforce_sovereign_trust_resource_limits(
    grant: &SovereignTrustGrant,
    estimated_token_cost: i64,
    estimated_cpu_seconds_reservation: f64,
    now_ms: i64,
) -> Result<(), ShieldGateError> {
    let today = trust_usage_day(now_ms);
    let token_cost_used_today = if grant.usage_day == today {
        grant.token_cost_used_today
    } else {
        0
    };
    let cpu_seconds_used_today = if grant.usage_day == today {
        grant.cpu_seconds_used_today
    } else {
        0.0
    };

    if grant.daily_token_cost_limit > 0
        && token_cost_used_today.saturating_add(estimated_token_cost.max(0))
            > grant.daily_token_cost_limit
    {
        return Err(sovereign_trust_limit_error(format!(
            "Trusted action exceeds the daily estimated token-cost reservation limit for {}: {} reserved + {} requested estimate > {}.",
            grant.canonical_directory_path,
            token_cost_used_today,
            estimated_token_cost,
            grant.daily_token_cost_limit
        )));
    }

    if grant.daily_cpu_seconds_limit > 0.0
        && cpu_seconds_used_today + estimated_cpu_seconds_reservation.max(0.0)
            > grant.daily_cpu_seconds_limit
    {
        return Err(sovereign_trust_limit_error(format!(
            "Trusted action exceeds the daily CPU-seconds reservation limit for {}: {:.3}s reserved + {:.3}s requested estimate > {:.3}s.",
            grant.canonical_directory_path,
            cpu_seconds_used_today,
            estimated_cpu_seconds_reservation,
            grant.daily_cpu_seconds_limit
        )));
    }

    Ok(())
}

fn estimate_action_cpu_seconds(action: &RequestedAction) -> f64 {
    match action.kind.as_str() {
        "terminal_execute" | "shell_command" | "execute_command" => 30.0,
        "file_write" => {
            let bytes = action.content.as_deref().unwrap_or_default().len() as f64;
            (bytes / 1_000_000.0).clamp(0.05, 2.0)
        }
        _ => 0.05,
    }
}

fn sovereign_trust_reservation_claim(
    tier: &str,
    canonical_scope: &str,
    requested_scope: &str,
    estimated_token_cost: i64,
    reserved_cpu_seconds: f64,
    observed_elapsed_wall_seconds: f64,
) -> String {
    format!(
        "CLAIM sovereign_trust_auto_approved tier={tier} scope={canonical_scope} requested_scope={requested_scope} estimated_token_cost={estimated_token_cost} reserved_cpu_seconds={reserved_cpu_seconds:.3} observed_elapsed_wall_seconds={observed_elapsed_wall_seconds:.3}"
    )
}

fn sovereign_trust_limit_error(message: String) -> ShieldGateError {
    ShieldGateError {
        code: "sovereign_trust_resource_limit_exceeded",
        boundary: "SovereignTrustEngine",
        message,
    }
}

fn validate_host_access_certificate(
    action_kind: &str,
    certificate: Option<&LogicalCertificate>,
    identity: &SovereignIdentity,
) -> Result<(), ShieldGateError> {
    if !requires_logical_certificate(action_kind) {
        return Ok(());
    }

    let certificate = certificate.ok_or_else(|| security_boundary_violation(format!(
        "Security Boundary Violation: {action_kind} requires a logical_certificate before Shield Gate authorization."
    )))?;

    certificate
        .validate_for_action_kind(action_kind)
        .map_err(security_boundary_violation)?;
    let signature = certificate.signature.as_ref().ok_or_else(|| {
        security_boundary_violation(
            "Security Boundary Violation: logical_certificate.signature is required.".to_string(),
        )
    })?;
    identity
        .verify_authority_certificate_parts(
            &certificate.premises,
            &certificate.execution_path,
            &certificate.formal_conclusion,
            signature,
        )
        .map_err(|error| ShieldGateError {
            code: error.code,
            boundary: error.boundary,
            message: error.message,
        })
}

pub fn validate_logical_certificate_for_host_access(
    action_kind: &str,
    certificate: Option<&LogicalCertificate>,
    identity: &SovereignIdentity,
) -> Result<(), ShieldGateError> {
    validate_host_access_certificate(action_kind, certificate, identity)
}

fn requires_logical_certificate(action_kind: &str) -> bool {
    matches!(
        action_kind,
        "file_read"
            | "codebase_patch"
            | "codebase_compile"
            | "file_list"
            | "system_audit"
            | "ask_local_document_index"
            | "network_request"
            | "web_fetch"
            | "sovereign_duckduckgo_search"
            | "document_index"
            | "memory_commit"
            | "remote_command"
            | "remote_inference"
            | "airlock_export"
            | "action_plan"
    )
}

fn security_boundary_violation(message: String) -> ShieldGateError {
    ShieldGateError {
        code: "security_boundary_violation",
        boundary: "ShieldGate",
        message,
    }
}

const SHIELD_GATE_DIAGNOSTIC_READ_FILES: &[&str] = &["soul.md", "user.md", "oomu_settings.json"];
const SHIELD_GATE_DIAGNOSTIC_READ_DIRS: &[&str] = &["soul_manifest"];

#[cfg(test)]
pub fn is_diagnostic_query_permitted(action_type: &str, target_path: &str) -> bool {
    resolve_diagnostic_read_path(action_type, target_path).is_ok()
}

pub(crate) fn resolve_diagnostic_read_path(
    action_type: &str,
    target_path: &str,
) -> Result<PathBuf, ShieldGateError> {
    let normalized_action = normalize_action_kind(action_type);
    if !matches!(
        normalized_action.as_str(),
        "file_list" | "file_read" | "system_audit"
    ) {
        return Err(security_boundary_violation(format!(
            "{action_type} is not eligible for the read-only diagnostic allowlist."
        )));
    }

    let requested = expand_shield_home_path(target_path, &normalized_action)?;
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(security_boundary_violation(format!(
            "{normalized_action} diagnostic query rejected path traversal."
        )));
    }

    if let Some(path) = resolve_diagnostic_config_read_path(&requested)? {
        return Ok(path);
    }

    Err(security_boundary_violation(format!(
        "{normalized_action} target is not in the Shield Gate diagnostic read allowlist."
    )))
}

fn resolve_read_only_action_path(
    action_type: &str,
    target_path: &str,
) -> Result<String, ShieldGateError> {
    if let Ok(path) = resolve_diagnostic_read_path(action_type, target_path) {
        log_shield_gate_bypass(action_type, target_path, &path);
        return Ok(path.display().to_string());
    }

    validate_project_quarantine(target_path, action_type)?;
    Ok(target_path.to_string())
}

fn resolve_diagnostic_config_read_path(
    requested: &Path,
) -> Result<Option<PathBuf>, ShieldGateError> {
    if !has_diagnostic_config_component(requested) {
        return Ok(None);
    }

    if requested.is_absolute() {
        for root in diagnostic_config_roots()? {
            if path_has_case_aware_prefix(requested, &root) {
                return Ok(Some(guard_existing_diagnostic_path(requested, &root)?));
            }
        }
        return Ok(None);
    }

    let root = project_root();
    let path = root.join(requested);
    Ok(Some(guard_existing_diagnostic_path(&path, &root)?))
}

fn has_diagnostic_config_component(path: &Path) -> bool {
    path.components().any(|component| {
        let Component::Normal(part) = component else {
            return false;
        };
        let normalized = part.to_string_lossy().to_ascii_lowercase();
        SHIELD_GATE_DIAGNOSTIC_READ_FILES
            .iter()
            .any(|allowed| normalized == *allowed)
            || SHIELD_GATE_DIAGNOSTIC_READ_DIRS
                .iter()
                .any(|allowed| normalized == *allowed)
    })
}

fn diagnostic_config_roots() -> Result<Vec<PathBuf>, ShieldGateError> {
    let mut roots = vec![project_root(), development_repo_root()];
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join(".oomu"));
    }
    Ok(roots)
}

fn guard_existing_diagnostic_path(
    candidate: &Path,
    root: &Path,
) -> Result<PathBuf, ShieldGateError> {
    if let Ok(real_candidate) = fs::canonicalize(candidate) {
        let real_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        if path_has_case_aware_prefix(&real_candidate, &real_root) {
            return Ok(real_candidate);
        }
        return Err(security_boundary_violation(
            "Diagnostic read target resolves outside its safe root.".to_string(),
        ));
    }

    if path_has_case_aware_prefix(candidate, root) {
        return Ok(candidate.to_path_buf());
    }

    Err(security_boundary_violation(
        "Diagnostic read target is outside its safe root.".to_string(),
    ))
}

fn path_has_case_aware_prefix(path: &Path, prefix: &Path) -> bool {
    let path = comparable_path_components(path);
    let prefix = comparable_path_components(prefix);
    path.len() >= prefix.len()
        && path
            .iter()
            .zip(prefix.iter())
            .all(|(left, right)| left == right)
}

fn comparable_path_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| {
            let value = component.as_os_str().to_string_lossy();
            if cfg!(any(target_os = "macos", windows)) {
                value.to_ascii_lowercase()
            } else {
                value.to_string()
            }
        })
        .collect()
}

fn log_shield_gate_bypass(action_type: &str, target_path: &str, resolved_path: &Path) {
    let _ = (action_type, target_path, resolved_path);
    #[cfg(debug_assertions)]
    eprintln!(
        "SHIELD_GATE_DIAGNOSTIC_ALLOWLIST action={} target=opaque_path resolved=verified",
        crate::redaction::redacted_log_text(action_type)
    );
}

fn validate_project_quarantine(path: &str, operation: &str) -> Result<(), ShieldGateError> {
    let root = project_root();
    let requested = expand_shield_home_path(path, operation)?;
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(security_boundary_violation(format!(
            "{operation} rejected path traversal outside project quarantine."
        )));
    }

    let candidate = if requested.is_absolute() {
        requested
    } else {
        root.join(&requested)
    };

    if !candidate.starts_with(&root) {
        return Err(security_boundary_violation(format!(
            "{operation} rejected path outside project quarantine."
        )));
    }

    Ok(())
}

fn validate_codebase_patch_target(path: &str) -> Result<(), ShieldGateError> {
    let requested = Path::new(path);
    if path.trim().is_empty() {
        return Err(security_boundary_violation(
            "codebase_patch requires a non-empty target_file_path.".to_string(),
        ));
    }
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(security_boundary_violation(
            "codebase_patch rejected path traversal outside the active development repository."
                .to_string(),
        ));
    }

    let root = development_repo_root();
    let root_real = fs::canonicalize(&root).map_err(|_| {
        security_boundary_violation(
            "codebase_patch development repository root is unavailable.".to_string(),
        )
    })?;
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let candidate_real = fs::canonicalize(&candidate).map_err(|_| {
        security_boundary_violation(
            "codebase_patch target must be an existing file inside the active development repository."
                .to_string(),
        )
    })?;

    if !candidate_real.starts_with(&root_real) {
        return Err(security_boundary_violation(
            "codebase_patch rejected a system or host path outside the active development repository."
                .to_string(),
        ));
    }
    if !candidate_real.is_file() {
        return Err(security_boundary_violation(
            "codebase_patch target is not a regular file.".to_string(),
        ));
    }

    Ok(())
}

fn validate_codebase_compile_root() -> Result<PathBuf, ShieldGateError> {
    let root = development_repo_root();
    let root_real = fs::canonicalize(&root).map_err(|_| {
        security_boundary_violation(
            "codebase_compile development repository root is unavailable.".to_string(),
        )
    })?;
    if !root_real.is_dir() {
        return Err(security_boundary_violation(
            "codebase_compile root is not a directory.".to_string(),
        ));
    }
    if !root_real.join("package.json").is_file()
        || !root_real.join("src-tauri").join("Cargo.toml").is_file()
    {
        return Err(security_boundary_violation(
            "codebase_compile root does not look like the active development repository."
                .to_string(),
        ));
    }
    Ok(root_real)
}

fn expand_shield_home_path(path: &str, operation: &str) -> Result<PathBuf, ShieldGateError> {
    let normalized = normalize_shell_escaped_path(path);
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return Err(security_boundary_violation(format!(
            "{operation} requires a non-empty path."
        )));
    }
    if trimmed == "~" {
        return env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            security_boundary_violation("Unable to resolve the user home directory.".to_string())
        });
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        let home = env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            security_boundary_violation("Unable to resolve the user home directory.".to_string())
        })?;
        return Ok(home.join(rest));
    }
    Ok(PathBuf::from(trimmed))
}

fn normalize_shell_escaped_path(raw_path: &str) -> String {
    let trimmed = raw_path.trim();
    let mut normalized = String::with_capacity(trimmed.len());
    let mut characters = trimmed.chars();

    while let Some(character) = characters.next() {
        if character == '\\' {
            if let Some(escaped) = characters.next() {
                normalized.push(escaped);
            } else {
                normalized.push(character);
            }
            continue;
        }
        normalized.push(character);
    }

    normalized
}

fn shield_file_delete_safe_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|path| fs::canonicalize(path).ok())
    {
        roots.push(home);
    }
    if let Ok(temp) = fs::canonicalize(env::temp_dir()) {
        roots.push(temp);
    }
    if let Ok(root) = fs::canonicalize(project_root()) {
        roots.push(root);
    }
    if let Ok(root) = fs::canonicalize(development_repo_root()) {
        if !roots.iter().any(|existing| existing == &root) {
            roots.push(root);
        }
    }
    roots
}

fn validate_web_url(url: &str) -> Result<(), ShieldGateError> {
    if url.starts_with("https://") || url.starts_with("http://") {
        return Ok(());
    }

    Err(security_boundary_violation(
        "Web fetch rejected a non-HTTP source URI.".to_string(),
    ))
}

#[cfg(test)]
fn mask_pii(value: &str) -> String {
    let mut entities = Vec::new();

    let patterns = [
        r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}",
        r"\b\d{3}-\d{2}-\d{4}\b",
        r"(?i)\bssn[:\s-]*[a-zA-Z0-9_-]+\b",
        r"\b(?:\+?1[-. ]?)?\(?\d{3}\)?[-. ]?\d{3}[-. ]\d{4}\b",
        r"(?i)\b(?:dob|birth|date of birth|birthdate)[:\s-]*\d{2,4}[-/.]\d{2}[-/.]\d{2,4}\b",
        r"\b(?:19|20)\d{2}[-/.]\d{2}[-/.]\d{2}\b",
        r"(?i)\b(?:mrn|patient|physician|doctor|dr|patient_id)[:\s-]*[a-zA-Z0-9_-]+\b",
        r"\bMRN-\d{5,}\b",
        r"\b(?:\d{1,3}\.){3}\d{1,3}\b",
        r"\b(?:\d[ -]*?){13,16}\b",
        r"\b\d{1,5}\s+[A-Za-z0-9\s,.]+?\s+(?:Street|St|Avenue|Ave|Road|Rd|Boulevard|Blvd|Drive|Dr|Lane|Ln|Way|Court|Ct|ZIP|zip|Zip)\b",
        r"\b\d{5}(?:-\d{4})?\b",
        r"/[Uu]sers/[a-zA-Z0-9_-]+/",
        r"(?i)oomu/[a-zA-Z0-9_-]+/",
    ];

    for re in patterns
        .iter()
        .filter_map(|pattern| regex::Regex::new(pattern).ok())
    {
        for mat in re.find_iter(value) {
            let ent = mat.as_str().trim().to_string();
            if !ent.is_empty() {
                entities.push(ent);
            }
        }
    }

    for token in value.split_whitespace() {
        let trimmed = token.trim_matches(|character: char| {
            matches!(
                character,
                ',' | '.' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\''
            )
        });
        let lower = trimmed.to_lowercase();
        let digit_count = trimmed
            .chars()
            .filter(|character| character.is_ascii_digit())
            .count();
        if lower.contains('@')
            || trimmed.contains("/Users/")
            || lower.contains("oomu/")
            || (digit_count >= 9 && trimmed.chars().any(|character| "-().".contains(character)))
            || lower.starts_with("ssn:")
            || lower.starts_with("dob:")
            || lower.starts_with("patient:")
        {
            entities.push(trimmed.to_string());
        }
    }

    let mut unique = Vec::new();
    for entity in entities {
        if !unique.contains(&entity) {
            unique.push(entity);
        }
    }

    unique.sort_by(|a, b| b.len().cmp(&a.len()));

    let mut sanitized = value.to_string();
    for entity in unique {
        sanitized = sanitized.replace(&entity, "{{PII_MASKED}}");
    }
    sanitized
}

pub fn handle_authorized_action(action: AuthorizedActions) -> ExecuteCommandResponse {
    match action {
        AuthorizedActions::ApprovedExternalFileRead(request) => {
            handle_approved_external_file_read(request)
        }
        AuthorizedActions::ApprovedExternalFileList(request) => {
            handle_approved_external_file_list(request)
        }
        AuthorizedActions::ApprovedExternalFileWrite(request) => {
            handle_approved_external_file_write(request)
        }
        AuthorizedActions::TelemetryArchive(action) => ToolRegistry::new()
            .execute(AuthorizedActions::TelemetryArchive(action))
            .unwrap_or_else(ExecuteCommandResponse::from_tool_error),
        AuthorizedActions::ApprovedFileDelete(request) => handle_approved_file_delete(request),
        AuthorizedActions::ApprovedSystemExecution(request) => {
            handle_approved_system_execution(request)
        }
        action => ToolRegistry::new()
            .execute(action)
            .unwrap_or_else(ExecuteCommandResponse::from_tool_error),
    }
}

const MAX_APPROVED_EXTERNAL_FILE_READ_BYTES: u64 = 8 * 1024 * 1024;
const MAX_APPROVED_EXTERNAL_TEXT_CONTEXT_BYTES: u64 = 96 * 1024;
const MAX_APPROVED_EXTERNAL_DIRECTORY_ENTRIES: usize = 10_000;

fn approved_external_file_context(
    path: &Path,
    byte_count: u64,
    mut file: fs::File,
) -> Result<(String, String, bool), String> {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("pdf"))
    {
        let extraction = crate::pdf_containment::extract_pdf_from_open_file(file)
            .map_err(|error| error.message)?;
        let text = if extraction.text.trim().is_empty() {
            "No readable text was found in the approved PDF.".to_string()
        } else {
            extraction.text
        };
        return Ok(("application/pdf".to_string(), text, extraction.truncated));
    }
    if crate::tools::vision::is_supported_visual_artifact_path(path) {
        let mut bytes = Vec::with_capacity(byte_count as usize);
        std::io::Read::by_ref(&mut file)
            .take(MAX_APPROVED_EXTERNAL_FILE_READ_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "approved_file_read_failed".to_string())?;
        if bytes.len() as u64 > MAX_APPROVED_EXTERNAL_FILE_READ_BYTES {
            return Err("approved_file_too_large".to_string());
        }
        let visual = crate::tools::vision::analyze_visual_bytes_for_context(path, bytes)?;
        return Ok((visual.mime_type, visual.text, visual.truncated));
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_APPROVED_EXTERNAL_TEXT_CONTEXT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "approved_file_read_failed".to_string())?;
    let truncated = bytes.len() as u64 > MAX_APPROVED_EXTERNAL_TEXT_CONTEXT_BYTES;
    if truncated {
        bytes.truncate(MAX_APPROVED_EXTERNAL_TEXT_CONTEXT_BYTES as usize);
    }
    if bytes.iter().any(|byte| *byte == 0) {
        return Err("approved_file_format_unsupported".to_string());
    }
    let text =
        String::from_utf8(bytes).map_err(|_| "approved_file_format_unsupported".to_string())?;
    Ok(("text/plain".to_string(), text, truncated))
}

fn handle_approved_external_file_read(
    request: ApprovedExternalReadRequest,
) -> ExecuteCommandResponse {
    let path = PathBuf::from(&request.path);
    let file = match open_bound_external_target(&path, request.expected_identity, false) {
        Ok(file) => file,
        Err(message) => {
            return ExecuteCommandResponse::from_tool_error(ToolError {
                operation: "file_read".to_string(),
                message,
            });
        }
    };
    let metadata = match file.metadata() {
        Ok(metadata) if metadata.is_file() => metadata,
        _ => {
            return ExecuteCommandResponse::from_tool_error(ToolError {
                operation: "file_read".to_string(),
                message: "The approved file is no longer available.".to_string(),
            });
        }
    };
    if metadata.len() > MAX_APPROVED_EXTERNAL_FILE_READ_BYTES {
        return ExecuteCommandResponse::from_tool_error(ToolError {
            operation: "file_read".to_string(),
            message: "The approved file is too large to read safely in one step.".to_string(),
        });
    }
    match approved_external_file_context(&path, metadata.len(), file) {
        Ok((mime_type, content, truncated)) => {
            let bytes = metadata.len();
            let truncation_note = if truncated {
                "\n\nOOMU showed the portion that fits safely in this chat."
            } else {
                ""
            };
            ExecuteCommandResponse {
                operation: "file_read".to_string(),
                status: CommandStatus::Completed,
                message: format!("{content}{truncation_note}"),
                metrics: None,
                claims: vec![
                    format!(
                        "CLAIM shield_gate_approved_external_read path={} min_bytes={bytes}",
                        path.display()
                    ),
                    format!("CLAIM local_context_parser mime_type={mime_type}"),
                ],
                verified: true,
                model_used: None,
            }
        }
        Err(_) => ExecuteCommandResponse::from_tool_error(ToolError {
            operation: "file_read".to_string(),
            message: "The approved file could not be viewed safely.".to_string(),
        }),
    }
}

fn handle_approved_external_file_list(
    request: ApprovedExternalReadRequest,
) -> ExecuteCommandResponse {
    let path = PathBuf::from(&request.path);
    let directory = match open_bound_external_target(&path, request.expected_identity, true) {
        Ok(directory) => directory,
        Err(message) => {
            return ExecuteCommandResponse::from_tool_error(ToolError {
                operation: "file_list".to_string(),
                message,
            });
        }
    };
    let mut entries = match list_bound_external_directory(&directory) {
        Ok(entries) => entries,
        Err(message) => {
            return ExecuteCommandResponse::from_tool_error(ToolError {
                operation: "file_list".to_string(),
                message,
            });
        }
    };
    entries.sort();
    let entry_count = entries.len();
    ExecuteCommandResponse {
        operation: "file_list".to_string(),
        status: CommandStatus::Completed,
        message: if entries.is_empty() {
            "(directory is empty)".to_string()
        } else {
            entries.join("\n")
        },
        metrics: None,
        claims: vec![
            format!(
                "CLAIM shield_gate_approved_external_directory path={}",
                path.display()
            ),
            format!(
                "CLAIM directory_entries path={} count={entry_count}",
                path.display()
            ),
        ],
        verified: true,
        model_used: None,
    }
}

fn handle_approved_file_delete(request: FileDeleteRequest) -> ExecuteCommandResponse {
    let path = PathBuf::from(&request.path);
    match fs::remove_file(&path) {
        Ok(()) => {
            if path.exists() {
                return ExecuteCommandResponse::from_tool_error(ToolError {
                    operation: "delete_file".to_string(),
                    message: "Unable to verify deletion of the approved file.".to_string(),
                });
            }
            ExecuteCommandResponse {
                operation: "delete_file".to_string(),
                status: CommandStatus::Completed,
                message: format!("Deleted file: {}", path.display()),
                metrics: None,
                claims: vec![format!("CLAIM file_deleted path={}", path.display())],
                verified: true,
                model_used: None,
            }
        }
        Err(_) => ExecuteCommandResponse::from_tool_error(ToolError {
            operation: "delete_file".to_string(),
            message: "The approved file delete failed.".to_string(),
        }),
    }
}

fn handle_approved_external_file_write(
    request: ApprovedExternalWriteRequest,
) -> ExecuteCommandResponse {
    let path = PathBuf::from(&request.path);
    let bytes = match write_bound_external_target_atomically(&request) {
        Ok(bytes) => bytes,
        Err(message) => {
            return ExecuteCommandResponse::from_tool_error(ToolError {
                operation: "file_write".to_string(),
                message,
            });
        }
    };
    ExecuteCommandResponse {
        operation: "file_write".to_string(),
        status: CommandStatus::Completed,
        message: format!(
            "Shield Gate approved and wrote {bytes} byte(s) to {}.",
            path.display()
        ),
        metrics: None,
        claims: vec![format!(
            "CLAIM shield_gate_approved_external_write path={} min_bytes={bytes}",
            path.display()
        )],
        verified: true,
        model_used: None,
    }
}

fn truncate_for_receipt(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (index, character) in value.chars().enumerate() {
        if index >= max_chars {
            output.push_str("\n...[truncated]");
            break;
        }
        output.push(character);
    }
    output
}

pub fn get_system_metrics(request: SystemMetricsRequest) -> ExecuteCommandResponse {
    let logical_cpus = match std::thread::available_parallelism() {
        Ok(logical_cpus) => usize::from(logical_cpus),
        Err(error) => {
            return ExecuteCommandResponse::from_tool_error(ToolError {
                operation: "get_system_metrics".to_string(),
                message: format!(
                    "Unable to observe the operating system's available CPU parallelism: {error}"
                ),
            });
        }
    };
    let unix_time_ms = unix_time_ms_u128();

    let metrics = SystemMetrics {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        logical_cpus,
        unix_time_ms,
    };

    ExecuteCommandResponse {
        operation: "get_system_metrics".to_string(),
        status: CommandStatus::Completed,
        message: format!(
            "Diagnostics complete for {}: {} on {} with {} logical CPU(s).",
            request.principal, metrics.os, metrics.arch, metrics.logical_cpus
        ),
        metrics: Some(metrics),
        claims: vec!["CLAIM operation=get_system_metrics status=completed".to_string()],
        verified: false,
        model_used: None,
    }
}

impl ExecuteCommandResponse {
    pub fn from_tool_error(error: ToolError) -> Self {
        Self {
            operation: error.operation,
            status: CommandStatus::Failed,
            message: error.message,
            metrics: None,
            claims: vec!["CLAIM operation=tool_error status=failed".to_string()],
            verified: false,
            model_used: None,
        }
    }
}

impl ModelMetadata {
    pub fn local_gemma() -> Self {
        Self {
            name: "Gemma 4 2B".to_string(),
            version: "llama.cpp resident context".to_string(),
            provider: "Local".to_string(),
            locality: "local".to_string(),
        }
    }

    pub fn gemini_pro() -> Self {
        Self {
            name: "Gemini 3.1 Pro".to_string(),
            version: "API bridge".to_string(),
            provider: "Gemini".to_string(),
            locality: "remote".to_string(),
        }
    }

    pub fn chatgpt() -> Self {
        Self {
            name: "ChatGPT".to_string(),
            version: "API bridge".to_string(),
            provider: "OpenAI".to_string(),
            locality: "remote".to_string(),
        }
    }
}

#[cfg(debug_assertions)]
fn log_certificate(operation: &str, input: &str, output: &str) {
    eprintln!(
        "MLC_LOG operation={} input_bytes={} output_bytes={}",
        crate::redaction::redacted_log_text(operation),
        input.len(),
        output.len()
    );
}

#[cfg(not(debug_assertions))]
fn log_certificate(_operation: &str, _input: &str, _output: &str) {}

fn project_root() -> PathBuf {
    crate::settings::app_data_root()
}

pub(crate) fn development_repo_root() -> PathBuf {
    if let Some(manifest_dir) = crate::development_manifest_dir() {
        let manifest_dir = PathBuf::from(manifest_dir);
        if manifest_dir.ends_with("src-tauri") {
            if let Some(root) = manifest_dir.parent() {
                return root.to_path_buf();
            }
        }
        return manifest_dir;
    }

    env::current_dir().unwrap_or_else(|_| project_root())
}

impl LogicalCertificate {
    pub fn unsigned(
        premises: Vec<String>,
        execution_path: Vec<String>,
        formal_conclusion: String,
    ) -> Self {
        Self {
            premises,
            execution_path,
            formal_conclusion,
            signature: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_for_action_kind("generic")
    }

    fn validate_for_action_kind(&self, action_kind: &str) -> Result<(), String> {
        if self.premises.is_empty()
            || self
                .premises
                .iter()
                .any(|premise| premise.trim().is_empty())
        {
            return Err(
                "Security Boundary Violation: logical_certificate.premises must contain non-empty entries."
                    .to_string(),
            );
        }

        if self.execution_path.is_empty()
            || self
                .execution_path
                .iter()
                .any(|entry| entry.trim().is_empty())
        {
            return Err(
                "Security Boundary Violation: logical_certificate.execution_path must contain non-empty entries."
                    .to_string(),
            );
        }

        if self.formal_conclusion.trim().is_empty() {
            return Err(
                "Security Boundary Violation: logical_certificate.formal_conclusion is required."
                    .to_string(),
            );
        }

        let min_execution_path_entries = if action_kind == "action_plan" { 1 } else { 2 };
        if strict_mlc_mode()
            && (self.execution_path.len() < min_execution_path_entries
                || self.formal_conclusion.trim().len() < 12)
        {
            return Err(
                "Security Boundary Violation: strict MLC mode requires explicit reasoning."
                    .to_string(),
            );
        }

        Ok(())
    }
}

fn strict_mlc_mode() -> bool {
    option_env!("OOMU_MLC_STRICT_MODE").unwrap_or("true") == "true"
}

#[cfg(test)]
pub(crate) mod tests;
