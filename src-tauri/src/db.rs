use crate::agentic_loop::{ActionPlan, AgenticLoopError};
use crate::foundation::{
    clock::{unix_time_ms_i64 as unix_time_ms, unix_time_secs_i64 as unix_time_seconds},
    digest::{sha256_chunks, sha256_hex},
};
use crate::p0_contracts::ProjectId;
use crate::persistence_health::BackingStoreClass;
use crate::security::firewall::{default_workspace_id, normalize_workspace_id};
use crate::settings;
use crate::workflow_ir::{
    BlueprintCompilationStatus, CompiledInstruction, CompiledWorkflow, CompilerTarget,
    ExecutionInstance, ExecutionStatus, InputNode, McpToolNode, OutputNode, WorkflowBlueprint,
    WorkflowCompletionKind, WorkflowEdge, WorkflowIr, WorkflowNode, WorkflowNodeKind,
    WORKFLOW_COMPILER_MODEL, WORKFLOW_IR_SCHEMA_VERSION,
};
#[cfg(all(not(test), feature = "insecure-fallback-danger-do-not-use"))]
compile_error!(
    "COMPILE ERROR: Insecure database fallbacks must never be enabled in release builds."
);
pub(crate) mod accepted_turn_checkpoint;
mod agent_execution;
mod agent_execution_calendar_recovery;
mod agent_execution_lifecycle;
mod agent_execution_recovery_state;
mod agent_execution_restart;
mod assistant_content;
#[cfg(test)]
mod assistant_content_tests;
mod auto_route;
mod auto_route_identity;
mod auto_route_reconciliation;
pub(crate) mod auto_route_validation;
mod canonical_json;
mod canonical_model_authority;
mod ch_migration;
mod chat_completion_attention;
mod chat_completion_attention_migration;
mod chat_session_attention_migration;
mod chat_turn_acceptance;
mod commands;
mod connector_scope_migration;
mod database_key;
mod filesystem_context;
#[cfg(test)]
mod ledger_tests;
mod local_inference_audit;
mod migration_integrity;
mod migration_lock;
pub(crate) mod permission_turn_continuation;
mod queued_messages;
mod recoverable_chat_deletion;
#[cfg(test)]
mod recovery_integration_tests;
mod recovery_merge;
mod response_claim;
#[cfg(test)]
mod response_claim_tests;
mod route_policy_records;
mod routing_persistence;
mod scheduled_execution;
mod session_context_policy;
mod session_context_policy_migration;
mod slack_pkce_loopback_migration;
mod static_migrations;
mod terminal;
pub use accepted_turn_checkpoint::{
    record_accepted_chat_turn_checkpoint, AcceptedChatTurnCheckpointKind,
    AcceptedChatTurnCheckpointReceipt, RecordAcceptedChatTurnCheckpointRequest,
};
pub use agent_execution::PlanExecutionCheckpoint;
pub(crate) use auto_route_identity::VerifiedAutoRouteBaseline;
pub use auto_route_identity::{
    AutoRouteActivationReceipt, AutoRouteActivationResponse, AutoRouteProvenance,
    AutoRouteSessionBaselineRequest, CanonicalModelId, ProviderConfigurationId, ProviderTypeId,
    RouteGeneration,
};
use canonical_json::canonicalize_json;
pub(crate) use chat_turn_acceptance::CompleteClaimedChatTurnRequest;
pub use chat_turn_acceptance::{
    AbandonAcceptedChatTurnRequest, AcceptChatTurnRequest, AcceptedChatTurn,
    FinalizeAcceptedChatTurnRequest,
};
pub use commands::*;
#[cfg(test)]
use database_key::derive_legacy_bound_database_key;
use database_key::{clear_cached_database_key, get_legacy_database_key_for_migration};
pub use database_key::{database_key_error, get_current_encryption_state, get_database_key};
#[cfg(any(test, debug_assertions))]
use database_key::{
    derive_memory_hard_database_key, install_database_key_for_integration_test,
    resolve_database_secret_with_keychain_mode,
};
pub use filesystem_context::{
    AssistantContentReference, ContextualFileActionPreparation, PreparedContextualFileAction,
    VerifiedFilesystemContext,
};
use migration_integrity::{accepts_legacy_runner_checksum, verify_agent_execution_origin_index};
use migration_lock::MigrationFileLock;
use rand_core::{OsRng, RngCore};
pub(crate) use response_claim::{is_chat_turn_response_claim_conflict, AUTO_TURN_KIND};
use response_claim::{validate_chat_turn_context_fields, validate_chat_turn_parent};
use route_policy_records::session_config_from_row;
pub use route_policy_records::{
    AutoRouteTurnPolicyRecord, ChatSessionRoutePolicyRecord, QueuedAutoRouteIdentityRecord,
    SessionConfigRecord,
};
use rusqlite::{params, params_from_iter, Connection, OpenFlags, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub use session_context_policy::{
    CompactChatSessionRequest, ContextCompactionResult, ContextHorizonStatus,
    SaveSessionContextPolicyRequest, SessionContextPolicyRecord,
};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
pub use terminal::execute_terminal_db_audit;
use terminal::{terminal_empty_placeholder, terminal_optional_bool, terminal_preview};
use zeroize::Zeroize;
const PRIVATE_PERSISTENCE_STORE_ID: &str = "private://persistence";
const DB_FILE: &str = "oomu_state.sqlite";
const OPS_DB_FILE: &str = "oomu_ops.db";
const MOD_VECTOR_DB_FILE: &str = "mod_vector.db";
pub(crate) const SQLITE_MAINTENANCE_INTERVAL_MS: i64 = 604_800_000;
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const SQLITE_MAINTENANCE_LAST_RUN_KEY: &str = "sqlite_maintenance.last_run_ms";
const GATEWAY_MESSAGE_PROCESSING_LEASE_MS: i64 = 5 * 60 * 1_000;
const GATEWAY_MESSAGE_RECEIPT_RETENTION_MS: i64 = 90 * 24 * 60 * 60 * 1_000;
const DEFAULT_LOCAL_CONTEXT_TOKENS: usize = settings::DEFAULT_CONTEXT_BUDGET;
const DEFAULT_CLOUD_CONTEXT_TOKENS: usize = settings::DEFAULT_CLOUD_CONTEXT_BUDGET;
const LEDGER_AVOIDED_API_INPUT_USD_PER_MILLION: f64 = 1.25;
const LEDGER_AVOIDED_API_OUTPUT_USD_PER_MILLION: f64 = 5.00;
const LEDGER_BYTES_PER_TOKEN_ESTIMATE: f64 = 4.0;
const LEDGER_RESET_AT_KEY: &str = "sovereign_ledger.reset_at_ms";
const WORKFLOW_EXECUTION_MIGRATION: &str =
    include_str!("../migrations/0002_workflow_execution.sql");
const WORKFLOW_COMPILATION_STATUS_MIGRATION: &str =
    include_str!("../migrations/0003_workflow_compilation_status.sql");
const WORKFLOW_APPROVAL_GATEWAY_MIGRATION: &str =
    include_str!("../migrations/0004_workflow_approval_gateway.sql");
const WORKFLOW_SCHEDULES_MIGRATION: &str =
    include_str!("../migrations/0005_workflow_schedules.sql");
const CORE_SCHEMA_MIGRATION_SOURCE: &str = include_str!("../migrations/0001_core_schema.sql");
const CHAT_CONTEXT_MIGRATION_SOURCE: &str = include_str!("../migrations/0006_chat_context.sql");
const SOVEREIGN_TRUST_MIGRATION_SOURCE: &str =
    include_str!("../migrations/0007_sovereign_trust.sql");
const PROJECT_WORKSPACES_MIGRATION: &str =
    include_str!("../migrations/0010_project_workspaces.sql");
const TASK_CONTROL_PLANE_MIGRATION: &str =
    include_str!("../migrations/0011_unified_task_control_plane.sql");
const CAPABILITY_CONNECTORS_MIGRATION: &str =
    include_str!("../migrations/0012_capability_connectors_onboarding.sql");
const ROUTINES_BACKGROUND_MIGRATION: &str =
    include_str!("../migrations/0013_routines_background_delivery.sql");
const GUARDED_BROWSER_AUTOMATION_MIGRATION: &str =
    include_str!("../migrations/0014_guarded_browser_automation.sql");
const VERIFIED_ARTIFACT_PIPELINE_MIGRATION: &str =
    include_str!("../migrations/0015_verified_artifact_pipeline.sql");
const CONSTRAINED_DELEGATION_TRUST_UX_MIGRATION: &str =
    include_str!("../migrations/0016_constrained_delegation_and_trust_ux.sql");
const MICROSOFT_CONNECTOR_METADATA_MIGRATION: &str =
    include_str!("../migrations/0017_microsoft_connector_metadata.sql");
const VERIFIED_WORKBOOK_PIPELINE_MIGRATION: &str =
    include_str!("../migrations/0018_verified_workbook_pipeline.sql");
const VERIFIED_PRESENTATION_PIPELINE_MIGRATION: &str =
    include_str!("../migrations/0019_verified_presentation_pipeline.sql");
const MULTIMODAL_MEDIA_MIGRATION: &str = include_str!("../migrations/0020_multimodal_media.sql");
const SECURE_REMOTE_DISPATCH_MIGRATION: &str =
    include_str!("../migrations/0021_secure_remote_dispatch.sql");
const CAPABILITY_BUNDLES_MIGRATION: &str =
    include_str!("../migrations/0022_capability_bundles.sql");
const ADAPTIVE_LEARNING_MIGRATION: &str = include_str!("../migrations/0023_adaptive_learning.sql");
const SCALED_WORK_GRAPHS_ANALYSIS_MIGRATION: &str =
    include_str!("../migrations/0024_scaled_work_graphs_and_analysis.sql");
const CHAT_TURN_RESPONSE_CLAIM_MIGRATION: &str =
    include_str!("../migrations/0025_chat_turn_response_claim.sql");
const PRIVATE_DATA_EGRESS_RECEIPTS_MIGRATION: &str =
    include_str!("../migrations/0026_private_data_egress_receipts.sql");
const REMOTE_RECEIPT_ATOMICITY_MIGRATION: &str =
    include_str!("../migrations/0027_remote_receipt_atomicity.sql");
const REMOTE_ARTIFACT_TRUTH_MIGRATION: &str =
    include_str!("../migrations/0028_remote_artifact_truth.sql");
const VERIFIED_FILESYSTEM_CONTEXT_MIGRATION: &str =
    include_str!("../migrations/0029_verified_filesystem_context.sql");
const AGENT_EXECUTION_ORIGIN_UNIQUENESS_MIGRATION: &str =
    include_str!("../migrations/0030_agent_execution_origin_uniqueness.sql");
const RECOVERABLE_CHAT_DELETION_MIGRATION: &str =
    include_str!("../migrations/0031_recoverable_chat_deletion.sql");
// Preserve only the ledger fingerprints needed to recognize databases that ran
// these retired migrations. Their executable schema-creation code is gone.
const RETIRED_MIGRATION_0008_CHECKSUM: &str =
    "f918af8f0eb52b19124dc09fe9a6a48dae70ec8439790886576a2f158e15453d";
const RETIRED_MIGRATION_0009_CHECKSUM: &str =
    "be8464f522b8f56ce1d583116888e69e737226e64bde3189c702a13277e934d3";
#[derive(Debug, Clone, Copy)]
struct MigrationDescriptor {
    sequence: i64,
    id: &'static str,
    source: MigrationSource,
    destructive: bool,
}
const fn sql(sequence: i64, id: &'static str, source: &'static str) -> MigrationDescriptor {
    MigrationDescriptor {
        sequence,
        id,
        source: MigrationSource::Sql(source),
        destructive: false,
    }
}
#[derive(Debug, Clone, Copy)]
enum MigrationSource {
    Sql(&'static str),
    RustImplementation {
        contract: &'static str,
        implementation_ids: &'static [&'static str],
    },
    HistoricalChecksum(&'static str),
}

const MIGRATIONS: [MigrationDescriptor; 42] = [
    MigrationDescriptor {
        sequence: 1,
        id: "0001_core_schema",
        source: MigrationSource::RustImplementation {
            contract: CORE_SCHEMA_MIGRATION_SOURCE,
            implementation_ids: &["0001_core_schema", "0001_seed_channel_configs"],
        },
        destructive: false,
    },
    sql(2, "0002_workflow_execution", WORKFLOW_EXECUTION_MIGRATION),
    MigrationDescriptor {
        sequence: 3,
        id: "0003_workflow_compilation_status",
        source: MigrationSource::RustImplementation {
            contract: WORKFLOW_COMPILATION_STATUS_MIGRATION,
            implementation_ids: &["0003_workflow_compilation_status", "shared_schema_probes"],
        },
        destructive: false,
    },
    MigrationDescriptor {
        sequence: 4,
        id: "0004_workflow_approval_gateway",
        source: MigrationSource::RustImplementation {
            contract: WORKFLOW_APPROVAL_GATEWAY_MIGRATION,
            implementation_ids: &["0004_workflow_approval_gateway", "shared_schema_probes"],
        },
        destructive: true,
    },
    sql(5, "0005_workflow_schedules", WORKFLOW_SCHEDULES_MIGRATION),
    MigrationDescriptor {
        sequence: 6,
        id: "0006_chat_context",
        source: MigrationSource::RustImplementation {
            contract: CHAT_CONTEXT_MIGRATION_SOURCE,
            implementation_ids: &["0006_chat_context", "shared_schema_probes"],
        },
        destructive: false,
    },
    MigrationDescriptor {
        sequence: 7,
        id: "0007_sovereign_trust",
        source: MigrationSource::RustImplementation {
            contract: SOVEREIGN_TRUST_MIGRATION_SOURCE,
            implementation_ids: &["0007_sovereign_trust", "shared_schema_probes"],
        },
        destructive: false,
    },
    MigrationDescriptor {
        sequence: 8,
        id: "0008_license_compliance",
        source: MigrationSource::HistoricalChecksum(RETIRED_MIGRATION_0008_CHECKSUM),
        destructive: false,
    },
    MigrationDescriptor {
        sequence: 9,
        id: "0009_retire_license_compliance",
        source: MigrationSource::HistoricalChecksum(RETIRED_MIGRATION_0009_CHECKSUM),
        destructive: true,
    },
    sql(10, "0010_project_workspaces", PROJECT_WORKSPACES_MIGRATION),
    sql(
        11,
        "0011_unified_task_control_plane",
        TASK_CONTROL_PLANE_MIGRATION,
    ),
    sql(
        12,
        "0012_capability_connectors_onboarding",
        CAPABILITY_CONNECTORS_MIGRATION,
    ),
    sql(
        13,
        "0013_routines_background_delivery",
        ROUTINES_BACKGROUND_MIGRATION,
    ),
    sql(
        14,
        "0014_guarded_browser_automation",
        GUARDED_BROWSER_AUTOMATION_MIGRATION,
    ),
    sql(
        15,
        "0015_verified_artifact_pipeline",
        VERIFIED_ARTIFACT_PIPELINE_MIGRATION,
    ),
    sql(
        16,
        "0016_constrained_delegation_and_trust_ux",
        CONSTRAINED_DELEGATION_TRUST_UX_MIGRATION,
    ),
    sql(
        17,
        "0017_microsoft_connector_metadata",
        MICROSOFT_CONNECTOR_METADATA_MIGRATION,
    ),
    sql(
        18,
        "0018_verified_workbook_pipeline",
        VERIFIED_WORKBOOK_PIPELINE_MIGRATION,
    ),
    sql(
        19,
        "0019_verified_presentation_pipeline",
        VERIFIED_PRESENTATION_PIPELINE_MIGRATION,
    ),
    sql(20, "0020_multimodal_media", MULTIMODAL_MEDIA_MIGRATION),
    sql(
        21,
        "0021_secure_remote_dispatch",
        SECURE_REMOTE_DISPATCH_MIGRATION,
    ),
    sql(22, "0022_capability_bundles", CAPABILITY_BUNDLES_MIGRATION),
    sql(23, "0023_adaptive_learning", ADAPTIVE_LEARNING_MIGRATION),
    sql(
        24,
        "0024_scaled_work_graphs_and_analysis",
        SCALED_WORK_GRAPHS_ANALYSIS_MIGRATION,
    ),
    sql(
        25,
        "0025_chat_turn_response_claim",
        CHAT_TURN_RESPONSE_CLAIM_MIGRATION,
    ),
    sql(
        26,
        "0026_private_data_egress_receipts",
        PRIVATE_DATA_EGRESS_RECEIPTS_MIGRATION,
    ),
    sql(
        27,
        "0027_remote_receipt_atomicity",
        REMOTE_RECEIPT_ATOMICITY_MIGRATION,
    ),
    sql(
        28,
        "0028_remote_artifact_truth",
        REMOTE_ARTIFACT_TRUTH_MIGRATION,
    ),
    sql(
        29,
        "0029_verified_filesystem_context",
        VERIFIED_FILESYSTEM_CONTEXT_MIGRATION,
    ),
    sql(
        30,
        "0030_agent_execution_origin_uniqueness",
        AGENT_EXECUTION_ORIGIN_UNIQUENESS_MIGRATION,
    ),
    sql(
        31,
        "0031_recoverable_chat_deletion",
        RECOVERABLE_CHAT_DELETION_MIGRATION,
    ),
    connector_scope_migration::DESCRIPTOR,
    ch_migration::DESCRIPTOR,
    chat_session_attention_migration::DESCRIPTOR,
    slack_pkce_loopback_migration::DESCRIPTOR,
    chat_completion_attention_migration::DESCRIPTOR,
    session_context_policy_migration::DESCRIPTOR,
    static_migrations::PRIVATE_EGRESS_CONFIRMATION_DESCRIPTOR,
    static_migrations::AUTO_ROUTE_MODEL_IDENTITY_DESCRIPTOR,
    static_migrations::EXECUTION_TRANSCRIPT_CONTINUITY_DESCRIPTOR,
    static_migrations::AUTO_ROUTE_PROVIDER_IDENTITY_DESCRIPTOR,
    static_migrations::TRUTHFUL_BACKGROUND_RUNTIME_DESCRIPTOR,
];
pub const WORKFLOW_APPROVAL_TTL_SECONDS: i64 = 900;
pub(crate) const SOVEREIGN_TRUST_SESSION_DURATION_MS: i64 = 86_400_000;
pub(crate) const DEFAULT_SOVEREIGN_TRUST_DAILY_TOKEN_LIMIT: i64 = 100_000;
pub(crate) const DEFAULT_SOVEREIGN_TRUST_DAILY_CPU_SECONDS_LIMIT: f64 = 3_600.0;
pub(crate) const COMMUNITY_CHANNEL_PLATFORMS: [&str; 3] = ["telegram", "discord", "slack"];
#[derive(Clone)]
pub struct PersistenceEngine {
    db_path: Arc<RwLock<PathBuf>>,
    write_lock: Arc<Mutex<()>>,
    workspace_id: String,
    storage_class: Arc<RwLock<BackingStoreClass>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistenceRecoveryReport {
    pub recovered_records: usize,
    pub skipped_records: usize,
    pub conflicting_records: usize,
    pub failed_records: usize,
    pub durable_probe_verified: bool,
    pub requires_confirmation: bool,
    pub backup_created: bool,
}
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum DatabaseError {
    #[error("Unable to resolve OOMU application support directory: {0}")]
    AppSupportUnavailable(String),
    #[error("Invalid mod database namespace: {0}")]
    InvalidModId(String),
    #[error("Mod database not found at {0}")]
    NotFound(String),
    #[error("Mod database connection failed: {0}")]
    ConnectionFailed(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseDatabaseTablePurge {
    pub table: String,
    pub rows_deleted: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseDatabaseSanitationReport {
    pub path: PathBuf,
    pub purged_tables: Vec<ReleaseDatabaseTablePurge>,
}
#[derive(Debug, Clone, Serialize)]
pub struct AgenticState {
    pub intents: Vec<IntentRecord>,
    pub actions: Vec<ActionRecord>,
    pub certificates: Vec<CertificateRecord>,
    pub plan_generation_states: Vec<PlanGenerationStateRecord>,
    pub recoverable_actions: Vec<RecoverableAction>,
}
#[derive(Debug, Clone)]
pub struct ProjectInferenceContext {
    pub project_id: String,
    pub instructions: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct IntentRecord {
    pub id: i64,
    pub plan_id: String,
    pub prompt: String,
    pub metadata: String,
    pub timestamp_ms: i64,
}
#[derive(Debug, Clone, Serialize)]
pub struct ActionRecord {
    pub id: i64,
    pub plan_id: String,
    pub tool: String,
    pub input: String,
    pub output: Option<String>,
    pub status: String,
    pub timestamp_ms: i64,
}
#[derive(Debug, Clone, Serialize)]
pub struct CertificateRecord {
    pub id: i64,
    pub plan_id: String,
    pub action_id: Option<i64>,
    pub mlc_path: String,
    pub mlc_content: String,
    pub timestamp_ms: i64,
}
#[derive(Debug, Clone, Serialize)]
pub struct PlanGenerationStateRecord {
    pub id: i64,
    pub plan_id: String,
    pub plan_json: String,
    pub current_step_index: i64,
    pub status: String,
    pub generated_text: String,
    pub timestamp_ms: i64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionLogRecord {
    pub id: i64,
    pub execution_id: String,
    pub plan_id: String,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub level: String,
    pub phase: String,
    pub message: String,
    pub payload_json: Option<String>,
    pub created_at_ms: i64,
}
impl AgentExecutionLogRecord {
    pub fn is_terminal(&self) -> bool {
        agent_execution_restart::is_terminal_phase(&self.phase)
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionLogBatch {
    pub execution_id: String,
    pub logs: Vec<AgentExecutionLogRecord>,
    pub terminal: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecoverableAction {
    pub action_id: i64,
    pub plan_id: String,
    pub tool: String,
    pub input: String,
    pub status: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SovereignTrustPermissionLevel {
    OneTime,
    SessionGated,
    GlobalTrust,
}

impl SovereignTrustPermissionLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::OneTime => "one_time",
            Self::SessionGated => "session_gated",
            Self::GlobalTrust => "global_trust",
        }
    }

    fn parse(value: &str) -> rusqlite::Result<Self> {
        match value {
            "one_time" => Ok(Self::OneTime),
            "session_gated" => Ok(Self::SessionGated),
            "global_trust" => Ok(Self::GlobalTrust),
            _ => Err(database_key_error(format!(
                "Unknown sovereign trust permission level: {value}"
            ))),
        }
    }

    fn from_request(value: &str) -> Result<Self, String> {
        match value.trim().replace('-', "_").to_ascii_lowercase().as_str() {
            "one_time" => Ok(Self::OneTime),
            "session_gated" => Ok(Self::SessionGated),
            "global_trust" => Ok(Self::GlobalTrust),
            _ => Err(format!(
                "Sovereign trust permission_level must be one_time, session_gated, or global_trust, got '{value}'."
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SovereignTrustToolCategory {
    ShellCommands,
    ExternalWrites,
}

impl SovereignTrustToolCategory {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ShellCommands => "shell_commands",
            Self::ExternalWrites => "external_writes",
        }
    }

    fn from_request(value: &str) -> Result<Self, String> {
        match value.trim().replace('-', "_").to_ascii_lowercase().as_str() {
            "shell_commands" | "shell_command" | "shell" => Ok(Self::ShellCommands),
            "external_writes" | "external_write" | "file_write" | "file_writes" => {
                Ok(Self::ExternalWrites)
            }
            _ => Err(format!(
                "Sovereign trust tool category must be shell_commands or external_writes, got '{value}'."
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SovereignTrustGrantSource {
    Policy(i64),
    Session(String),
}

#[derive(Debug, Clone)]
pub(crate) struct SovereignTrustGrant {
    pub source: SovereignTrustGrantSource,
    pub directory_path: String,
    pub canonical_directory_path: String,
    pub allowed_tool_categories: String,
    pub permission_level: SovereignTrustPermissionLevel,
    pub expires_at_ms: Option<i64>,
    pub daily_token_cost_limit: i64,
    pub daily_cpu_seconds_limit: f64,
    pub token_cost_used_today: i64,
    pub cpu_seconds_used_today: f64,
    pub usage_day: i64,
}

#[derive(Clone)]
pub(crate) struct ChannelConfigRecord {
    pub platform: String,
    pub label: String,
    pub is_active: bool,
    pub credentials_json: String,
    pub owner_id: Option<String>,
    pub updated_at_ms: i64,
}

impl std::fmt::Debug for ChannelConfigRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelConfigRecord")
            .field("platform", &self.platform)
            .field("label", &self.label)
            .field("is_active", &self.is_active)
            .field(
                "credential_configured",
                &(self.credentials_json.trim() != "{}"),
            )
            .field("owner_id", &self.owner_id)
            .field("updated_at_ms", &self.updated_at_ms)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelConfigSummary {
    pub platform: String,
    pub label: String,
    pub is_active: bool,
    pub credential_configured: bool,
    pub owner_id: Option<String>,
    pub updated_at_ms: i64,
}

impl From<&ChannelConfigRecord> for ChannelConfigSummary {
    fn from(config: &ChannelConfigRecord) -> Self {
        Self {
            platform: config.platform.clone(),
            label: config.label.clone(),
            is_active: config.is_active,
            credential_configured: config.credentials_json.trim() != "{}",
            owner_id: config.owner_id.clone(),
            updated_at_ms: config.updated_at_ms,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveChannelConfigRequest {
    pub platform: String,
    pub is_active: bool,
    pub credentials_json: Option<String>,
    pub owner_id: Option<String>,
}

impl std::fmt::Debug for SaveChannelConfigRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SaveChannelConfigRequest")
            .field("platform", &self.platform)
            .field("is_active", &self.is_active)
            .field(
                "credential_update_supplied",
                &self.credentials_json.is_some(),
            )
            .field("owner_id", &self.owner_id)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionRecord {
    pub id: String,
    pub workspace_id: String,
    pub project_id: Option<String>,
    pub agent_id: String,
    pub title: String,
    pub title_source: String,
    pub provider_id: String,
    pub model_id: String,
    pub web_grounding_override: Option<bool>,
    pub dynamic_routing_override: Option<bool>,
    pub unread_completion: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameChatSessionRequest {
    pub session_id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessageRecord {
    pub id: i64,
    pub workspace_id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_json: Option<String>,
    pub is_compacted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compaction_type: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ChatTurnPersistenceContext {
    pub turn_id: String,
    pub generation_token: String,
    pub session_id: String,
    pub agent_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub parent_turn_id: Option<String>,
    pub root_turn_id: String,
    pub turn_kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SovereignLedgerStats {
    pub total_local_turns: u64,
    pub total_cloud_turns: u64,
    pub ratio_on_device: f64,
    pub estimated_api_savings: f64,
    pub data_egress_protected_mb: f64,
    pub protected_input_tokens: u64,
    pub protected_output_tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticCompactionResponse {
    pub compacted_messages: usize,
    pub anchor_message_id: Option<i64>,
}

const RECENT_RAW_CHAT_TURNS_TO_PRESERVE: usize = 6;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelevantChatMemoryBlock {
    pub workspace_id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at_ms: i64,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedWorkflowRecord {
    pub id: String,
    pub name: String,
    pub steps: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedWorkflowProjectionRecord {
    pub id: String,
    pub name: String,
    pub steps: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compilation_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_capabilities: Option<crate::workflow_ir::review::WorkflowReviewCapabilities>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutingPreferenceRecord {
    pub key: String,
    pub value: String,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_route_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_route_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_config_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRoutingPreferenceRecord {
    pub key: String,
    pub primary_route_id: Option<String>,
    pub fallback_route_id: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowScheduleRecord {
    pub id: String,
    pub workflow_id: String,
    #[serde(default)]
    pub workflow_version: Option<u32>,
    pub label: String,
    pub schedule_expression: String,
    pub run_request: Value,
    pub is_active: bool,
    #[serde(default)]
    pub next_run_at_ms: Option<i64>,
    #[serde(default)]
    pub claimed_at_ms: Option<i64>,
    #[serde(default)]
    pub last_started_at_ms: Option<i64>,
    #[serde(default)]
    pub last_completed_at_ms: Option<i64>,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_instance_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default = "default_routine_timezone")]
    pub routine_timezone: String,
    #[serde(default = "default_schedule_kind")]
    pub schedule_kind: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default = "default_missed_run_policy")]
    pub missed_run_policy: String,
    #[serde(default = "default_missed_run_cap")]
    pub missed_run_cap: u8,
    #[serde(default)]
    pub active_window_start_minute: Option<u16>,
    #[serde(default)]
    pub active_window_end_minute: Option<u16>,
    #[serde(default)]
    pub delivery_target: Value,
    #[serde(default)]
    pub authority: Value,
}

fn default_routine_timezone() -> String {
    "UTC".to_string()
}
fn default_schedule_kind() -> String {
    "recurring".to_string()
}
fn default_missed_run_policy() -> String {
    "skip".to_string()
}
fn default_missed_run_cap() -> u8 {
    3
}

#[derive(Debug, Clone)]
pub struct WorkflowScheduleUpsert {
    pub id: String,
    pub workflow_id: String,
    pub workflow_version: Option<u32>,
    pub label: String,
    pub schedule_expression: String,
    pub run_request: Value,
    pub is_active: bool,
    pub next_run_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledKnowledgeSyncRecord {
    pub workflow_id: String,
    pub workflow_version: u32,
    pub schedule_id: String,
    pub path: String,
    pub schedule_expression: String,
    pub next_run_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueueMessageRequest {
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub generation_token: Option<String>,
    #[serde(default)]
    pub parent_turn_id: Option<String>,
    #[serde(default)]
    pub root_turn_id: Option<String>,
    #[serde(default)]
    pub turn_kind: Option<String>,
    pub agent_id: String,
    pub message: String,
    #[serde(default)]
    pub attachments: Vec<crate::inference::ChatAttachment>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default, alias = "contextBudget")]
    pub context_budget: Option<i32>,
    #[serde(default)]
    pub steering: Option<String>,
    #[serde(default)]
    pub automated_web_grounding_enabled: Option<bool>,
    #[serde(default)]
    pub dynamic_routing_override: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedMessageRecord {
    pub id: i64,
    pub turn_id: Option<String>,
    pub generation_token: Option<String>,
    pub parent_turn_id: Option<String>,
    pub root_turn_id: Option<String>,
    pub turn_kind: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: String,
    pub message: String,
    pub attachments: Vec<crate::inference::ChatAttachment>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub reasoning: Option<String>,
    pub context: Option<String>,
    pub steering: Option<String>,
    pub automated_web_grounding_enabled: Option<bool>,
    pub dynamic_routing_override: Option<bool>,
    pub auto_route_identity: Option<QueuedAutoRouteIdentityRecord>,
    pub status: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub executed_at_ms: Option<i64>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateChatSessionRequest {
    pub agent_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub title: Option<String>,
    #[serde(default, alias = "dynamicRoutingOverride")]
    pub dynamic_routing_override: Option<bool>,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaveAgenticStateRequest {
    pub plan: ActionPlan,
}

#[derive(Debug, Serialize)]
pub struct PersistenceResponse {
    pub db_path: String,
    pub message: String,
}

#[cfg(test)]
#[test]
fn persistence_response_store_id_is_opaque() {
    let serialized = serde_json::to_string(&PersistenceResponse {
        db_path: PRIVATE_PERSISTENCE_STORE_ID.to_string(),
        message: "stored".to_string(),
    })
    .unwrap();
    assert!(serialized.contains("private://persistence"));
    if let Some(home) = std::env::var_os("HOME") {
        assert!(!serialized.contains(&home.to_string_lossy().to_string()));
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertSovereignTrustPolicyRequest {
    pub authority_proof_id: String,
    pub session_id: String,
    pub directory_path: String,
    pub allowed_tool_categories: Vec<String>,
    pub permission_level: String,
    pub expires_at_ms: Option<i64>,
    pub daily_token_cost_limit: Option<i64>,
    pub daily_cpu_seconds_limit: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivateSovereignTrustSessionRequest {
    pub authority_proof_id: String,
    pub session_id: String,
    pub directory_path: String,
    pub allowed_tool_categories: Vec<String>,
    pub expires_at_ms: Option<i64>,
    pub daily_token_cost_limit: Option<i64>,
    pub daily_cpu_seconds_limit: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SovereignTrustPolicyResponse {
    pub policy_id: i64,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SovereignTrustSessionResponse {
    pub active_session_id: String,
    pub expires_at_ms: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SovereignTrustDashboardResponse {
    pub policies: Vec<SovereignTrustPolicyRecord>,
    pub active_sessions: Vec<SovereignTrustSessionRecord>,
    pub audit_events: Vec<SovereignTrustAuditEvent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SovereignTrustPolicyRecord {
    pub id: i64,
    pub directory_path: String,
    pub canonical_directory_path: String,
    pub allowed_tool_categories: Vec<String>,
    pub permission_level: String,
    pub expires_at_ms: Option<i64>,
    pub daily_token_cost_limit: i64,
    pub daily_cpu_seconds_limit: f64,
    pub estimated_token_cost_reserved_today: i64,
    pub cpu_seconds_reserved_today: f64,
    pub usage_day: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SovereignTrustSessionRecord {
    pub id: String,
    pub session_id: String,
    pub policy_id: Option<i64>,
    pub directory_path: String,
    pub canonical_directory_path: String,
    pub allowed_tool_categories: Vec<String>,
    pub permission_level: String,
    pub expires_at_ms: i64,
    pub daily_token_cost_limit: i64,
    pub daily_cpu_seconds_limit: f64,
    pub estimated_token_cost_reserved_today: i64,
    pub cpu_seconds_reserved_today: f64,
    pub usage_day: i64,
    pub created_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SovereignTrustAuditEvent {
    pub id: i64,
    pub plan_id: String,
    pub operation: String,
    pub input_kind: Option<String>,
    pub target_path: Option<String>,
    pub status: String,
    pub authorization_mode: String,
    pub trust_tier: Option<String>,
    pub execution_hash: String,
    pub summary: String,
    pub claims: Vec<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SovereignTrustMutationResponse {
    pub affected_rows: usize,
    pub message: String,
}

impl PersistenceEngine {
    pub fn initialize() -> Result<Self, String> {
        let db_path = project_root().join(DB_FILE);
        Self::initialize_at_with_storage_class(db_path, BackingStoreClass::Persistent)
    }

    pub fn initialize_at(db_path: PathBuf) -> Result<Self, String> {
        Self::initialize_at_with_storage_class(db_path, BackingStoreClass::Persistent)
    }

    pub(crate) fn initialize_volatile_at(db_path: PathBuf) -> Result<Self, String> {
        Self::initialize_at_with_storage_class(db_path, BackingStoreClass::Volatile)
    }

    fn initialize_at_with_storage_class(
        db_path: PathBuf,
        storage_class: BackingStoreClass,
    ) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let engine = Self {
            db_path: Arc::new(RwLock::new(db_path)),
            write_lock: Arc::new(Mutex::new(())),
            workspace_id: default_workspace_id(),
            storage_class: Arc::new(RwLock::new(storage_class)),
        };
        engine.run_migrations().map_err(|error| error.to_string())?;
        Ok(engine)
    }

    #[cfg(any(test, debug_assertions))]
    fn initialize_at_with_database_key_loader<F>(
        db_path: PathBuf,
        keychain_loader: F,
        allow_insecure_test_fallback: bool,
    ) -> Result<Self, String>
    where
        F: FnOnce() -> Result<String, String>,
    {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut database_secret = resolve_database_secret_with_keychain_mode(
            keychain_loader,
            allow_insecure_test_fallback,
        )?;
        let database_key = derive_memory_hard_database_key(&database_secret)?;
        database_secret.zeroize();
        let engine = Self {
            db_path: Arc::new(RwLock::new(db_path)),
            write_lock: Arc::new(Mutex::new(())),
            workspace_id: default_workspace_id(),
            storage_class: Arc::new(RwLock::new(BackingStoreClass::Persistent)),
        };
        engine
            .run_migrations_with_database_key(&database_key)
            .map_err(|error| error.to_string())?;
        Ok(engine)
    }

    /// Creates an isolated encrypted store for integration tests without
    /// depending on the interactive OS keychain. This API is absent from
    /// optimized release builds.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn initialize_for_integration_test(db_path: PathBuf) -> Result<Self, String> {
        const TEST_SECRET: &str = "default_secure_test_key";
        let engine = Self::initialize_at_with_database_key_loader(
            db_path,
            || Ok(TEST_SECRET.to_string()),
            false,
        )?;
        install_database_key_for_integration_test(derive_memory_hard_database_key(TEST_SECRET)?);
        Ok(engine)
    }

    pub fn db_path(&self) -> String {
        self.db_path
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .to_string_lossy()
            .to_string()
    }

    pub fn storage_class(&self) -> BackingStoreClass {
        *self
            .storage_class
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn require_durable_store(&self, operation: &str) -> Result<(), String> {
        if self.storage_class() == BackingStoreClass::Persistent {
            Ok(())
        } else {
            Err(format!(
                "Operation '{operation}' is blocked while persistence is {:?}; reconcile and verify durable storage first.",
                self.storage_class()
            ))
        }
    }

    pub fn project_inference_context_for_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ProjectInferenceContext>, String> {
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        let context = connection
            .query_row(
                "SELECT p.project_id, COALESCE(i.instructions, '') FROM chat_sessions c JOIN projects p ON p.project_id=c.project_id LEFT JOIN project_instructions i ON i.project_id=p.project_id WHERE c.id=?1 AND p.archived_at_ms IS NULL",
                params![session_id],
                |row| Ok(ProjectInferenceContext { project_id: row.get(0)?, instructions: row.get(1)? }),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some(mut context) = context else {
            return Ok(None);
        };
        let mut statement = connection.prepare("SELECT v.method_json FROM saved_methods m JOIN saved_method_versions v ON v.method_id=m.method_id AND v.version=m.current_version WHERE m.enabled=1 AND m.deleted_at_ms IS NULL AND (m.project_id=?1 OR m.project_id IS NULL) ORDER BY m.project_id IS NULL, m.updated_at_ms DESC LIMIT 20").map_err(|error|error.to_string())?;
        let methods = statement
            .query_map(params![context.project_id], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        if !methods.is_empty() {
            context.instructions.push_str("\n\nMethods the user explicitly asked OOMU to remember. Use one only when it clearly matches the current request. These methods never grant tools, permissions, connector access, or approval authority:\n");
            for method in methods {
                context.instructions.push_str("- ");
                context.instructions.push_str(&method);
                context.instructions.push('\n');
            }
        }
        Ok(Some(context))
    }

    pub(crate) fn export_encrypted_snapshot(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> Result<(), String> {
        if destination_path.exists() {
            return Err("Recovery snapshot destination already exists.".to_string());
        }
        if self.storage_class() != BackingStoreClass::Persistent
            && self.current_db_path() != source_path
        {
            return Err(
                "Recovery snapshot source does not match the active volatile store.".to_string(),
            );
        }

        let _guard = self.lock_writes();
        let database_key = get_database_key()?;
        let source = open_sqlcipher_database_connection_with_key(source_path, &database_key)
            .map_err(|error| error.to_string())?;
        verify_migration_ledger(&source).map_err(|error| error.to_string())?;
        verify_schema_invariants(&source, MIGRATIONS.len() as i64)
            .map_err(|error| error.to_string())?;
        source
            .execute_batch("PRAGMA wal_checkpoint(FULL);")
            .map_err(|error| error.to_string())?;
        let source_records =
            count_recoverable_records(&source).map_err(|error| error.to_string())?;

        let mut random = [0_u8; 16];
        OsRng.fill_bytes(&mut random);
        let temporary_path =
            destination_path.with_extension(format!("snapshot-{}.partial", hex::encode(random)));
        let result = (|| {
            let mut destination =
                open_sqlcipher_database_connection_with_key(&temporary_path, &database_key)
                    .map_err(|error| error.to_string())?;
            {
                let backup = rusqlite::backup::Backup::new(&source, &mut destination)
                    .map_err(|error| error.to_string())?;
                backup
                    .run_to_completion(128, Duration::from_millis(5), None)
                    .map_err(|error| error.to_string())?;
            }
            verify_migration_ledger(&destination).map_err(|error| error.to_string())?;
            verify_schema_invariants(&destination, MIGRATIONS.len() as i64)
                .map_err(|error| error.to_string())?;
            let destination_records =
                count_recoverable_records(&destination).map_err(|error| error.to_string())?;
            if destination_records != source_records {
                return Err(format!(
                    "Recovery snapshot record-count verification failed: expected {source_records}, found {destination_records}."
                ));
            }
            destination
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .map_err(|error| error.to_string())?;
            drop(destination);
            remove_sqlite_sidecars(&temporary_path);
            set_private_file(&temporary_path).map_err(|error| error.to_string())?;
            if has_plaintext_sqlite_header(&temporary_path) {
                return Err(
                    "Recovery snapshot unexpectedly contains a plaintext SQLite header."
                        .to_string(),
                );
            }
            fs::rename(&temporary_path, destination_path).map_err(|error| error.to_string())?;
            set_private_file(destination_path).map_err(|error| error.to_string())?;
            Ok(())
        })();
        if let Err(error) = result {
            remove_sqlite_sidecars(&temporary_path);
            let _ = fs::remove_file(&temporary_path);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn export_encrypted_operations_snapshot(
        &self,
        source_path: &Path,
        destination_path: &Path,
    ) -> Result<(), String> {
        if destination_path.exists() {
            return Err("Recovery operations snapshot destination already exists.".to_string());
        }
        if self.storage_class() != BackingStoreClass::Persistent
            && self.ops_db_path() != source_path
        {
            return Err(
                "Recovery operations snapshot source does not match the active volatile store."
                    .to_string(),
            );
        }

        let _guard = self.lock_writes();
        let database_key = get_database_key()?;
        let mut random = [0_u8; 16];
        OsRng.fill_bytes(&mut random);
        let temporary_path = destination_path.with_extension(format!(
            "operations-snapshot-{}.partial",
            hex::encode(random)
        ));
        create_verified_operations_copy(source_path, &temporary_path, &database_key)
            .map_err(|error| error.to_string())?;
        if let Err(error) = fs::rename(&temporary_path, destination_path) {
            remove_sqlite_sidecars(&temporary_path);
            let _ = fs::remove_file(&temporary_path);
            return Err(error.to_string());
        }
        set_private_file(destination_path).map_err(|error| error.to_string())
    }

    pub fn reconcile_volatile_store(
        &self,
        confirm_overwrite: bool,
    ) -> Result<PersistenceRecoveryReport, String> {
        self.reconcile_volatile_store_to(project_root().join(DB_FILE), confirm_overwrite)
    }

    fn reconcile_volatile_store_to(
        &self,
        durable_path: PathBuf,
        confirm_overwrite: bool,
    ) -> Result<PersistenceRecoveryReport, String> {
        if self.storage_class() == BackingStoreClass::Persistent {
            self.probe_active_durable_store()?;
            return Ok(PersistenceRecoveryReport {
                recovered_records: 0,
                skipped_records: 0,
                conflicting_records: 0,
                failed_records: 0,
                durable_probe_verified: true,
                requires_confirmation: false,
                backup_created: false,
            });
        }

        let _guard = self.lock_writes();
        let source_path = self.current_db_path();
        if source_path == durable_path {
            return Err("Recovery source and durable destination unexpectedly match.".to_string());
        }
        let key = get_database_key()?;
        let source = open_sqlcipher_database_connection_with_key(&source_path, &key)
            .map_err(|error| error.to_string())?;
        verify_migration_ledger(&source).map_err(|error| error.to_string())?;
        verify_schema_invariants(&source, MIGRATIONS.len() as i64)
            .map_err(|error| error.to_string())?;
        source
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|error| error.to_string())?;
        let source_records =
            count_recoverable_records(&source).map_err(|error| error.to_string())?;
        let source_operations_path = source_path
            .parent()
            .ok_or_else(|| "Volatile recovery state has no parent directory.".to_string())?
            .join(OPS_DB_FILE);
        if !source_operations_path.is_file() {
            return Err(
                "Volatile recovery operations database is missing; export or repair is required."
                    .to_string(),
            );
        }
        let source_operations =
            open_sqlcipher_database_connection_with_key(&source_operations_path, &key)
                .map_err(|error| error.to_string())?;
        verify_operations_database(&source_operations).map_err(|error| error.to_string())?;
        source_operations
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|error| error.to_string())?;
        let source_operations_records =
            count_operations_records(&source_operations).map_err(|error| error.to_string())?;

        if let Some(parent) = durable_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let durable_operations_path = durable_path
            .parent()
            .ok_or_else(|| "Durable recovery state has no parent directory.".to_string())?
            .join(OPS_DB_FILE);
        let durable_operations_file_exists = durable_operations_path.is_file();
        let mut durable_operations_ready = false;
        let mut durable_operations_connection = None;
        if durable_operations_file_exists {
            let durable_operations =
                open_sqlcipher_database_connection_with_key(&durable_operations_path, &key)
                    .map_err(|error| {
                        format!(
                            "Persistent operations-store probe failed; reconciliation refused without repair: {error}"
                        )
                    })?;
            match verify_operations_database(&durable_operations) {
                Ok(()) => {
                    durable_operations_ready = true;
                    durable_operations_connection = Some(durable_operations);
                }
                Err(error) => {
                    if !operations_database_has_no_user_schema(&durable_operations)
                        .map_err(|probe_error| probe_error.to_string())?
                    {
                        return Err(format!(
                            "Persistent operations-store validation failed; reconciliation refused without repair: {error}"
                        ));
                    }
                }
            }
        }
        let mut durable_connection = None;
        if durable_path.exists() {
            let connection = open_sqlcipher_database_connection_with_key(&durable_path, &key)
                .map_err(|error| {
                    format!(
                        "Persistent store probe failed; reconciliation refused without repair: {error}"
                    )
                })?;
            verify_migration_ledger(&connection).map_err(|error| error.to_string())?;
            verify_schema_invariants(&connection, MIGRATIONS.len() as i64)
                .map_err(|error| error.to_string())?;
            durable_read_write_probe(&connection).map_err(|error| error.to_string())?;
            durable_connection = Some(connection);
        }

        let source_total_records = source_records.saturating_add(source_operations_records);
        let state_assessment = durable_connection
            .as_ref()
            .map(|durable| {
                recovery_merge::assess_recovery_records(
                    &source,
                    durable,
                    recovery_merge::STATE_RECOVERY_TABLES,
                )
            })
            .transpose()?
            .unwrap_or(recovery_merge::RecoveryMergeAssessment {
                source_records,
                new_records: source_records,
                identical_records: 0,
                source_newer_records: 0,
                durable_newer_records: 0,
                conflicting_records: 0,
            });
        let operations_assessment = durable_operations_connection
            .as_ref()
            .map(|durable| {
                recovery_merge::assess_recovery_records(
                    &source_operations,
                    durable,
                    recovery_merge::OPERATIONS_RECOVERY_TABLES,
                )
            })
            .transpose()?
            .unwrap_or(recovery_merge::RecoveryMergeAssessment {
                source_records: source_operations_records,
                new_records: source_operations_records,
                identical_records: 0,
                source_newer_records: 0,
                durable_newer_records: 0,
                conflicting_records: 0,
            });
        if state_assessment.source_records != source_records
            || operations_assessment.source_records != source_operations_records
        {
            return Err("Recovery record accounting did not cover every protected table.".into());
        }
        let state_conflict = state_assessment.conflicting_records != 0;
        let operations_conflict = operations_assessment.conflicting_records != 0;
        let has_conflict = state_conflict || operations_conflict;
        let conflicting_records = state_assessment
            .conflicting_records
            .saturating_add(operations_assessment.conflicting_records);
        if has_conflict && !confirm_overwrite {
            *self
                .storage_class
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                BackingStoreClass::RecoveryPending;
            return Ok(PersistenceRecoveryReport {
                recovered_records: 0,
                skipped_records: source_total_records,
                conflicting_records,
                failed_records: 0,
                durable_probe_verified: true,
                requires_confirmation: true,
                backup_created: false,
            });
        }

        let replace_state = durable_connection.is_none() || (confirm_overwrite && state_conflict);
        let replace_operations =
            !durable_operations_ready || (confirm_overwrite && operations_conflict);
        let merge_state = !replace_state
            && (state_assessment.new_records != 0 || state_assessment.source_newer_records != 0);
        let merge_operations = !replace_operations
            && (operations_assessment.new_records != 0
                || operations_assessment.source_newer_records != 0);
        let state_backup_path = if replace_state || merge_state {
            durable_connection
                .as_ref()
                .map(|connection| {
                    create_verified_migration_backup(
                        connection,
                        &durable_path,
                        &key,
                        "volatile-reconciliation",
                    )
                    .map_err(|error| error.to_string())
                })
                .transpose()?
        } else {
            None
        };
        let operations_backup_path =
            if (replace_operations || merge_operations) && durable_operations_ready {
                Some(
                    create_verified_operations_backup(
                        &durable_operations_path,
                        &key,
                        "volatile-reconciliation-operations",
                    )
                    .map_err(|error| error.to_string())?,
                )
            } else {
                None
            };

        let mut recovered_records = 0usize;
        let mut skipped_records = 0usize;
        if merge_state {
            let report = recovery_merge::merge_non_conflicting_recovery_records(
                &source,
                durable_connection
                    .as_mut()
                    .ok_or_else(|| "Durable recovery state unexpectedly closed.".to_string())?,
                recovery_merge::STATE_RECOVERY_TABLES,
            )?;
            recovered_records = recovered_records
                .saturating_add(report.new_records)
                .saturating_add(report.source_newer_records);
            skipped_records = skipped_records
                .saturating_add(report.identical_records)
                .saturating_add(report.durable_newer_records);
        } else if replace_state {
            recovered_records = recovered_records.saturating_add(source_records);
        } else {
            skipped_records = skipped_records
                .saturating_add(state_assessment.identical_records)
                .saturating_add(state_assessment.durable_newer_records);
        }
        if merge_operations {
            let report = recovery_merge::merge_non_conflicting_recovery_records(
                &source_operations,
                durable_operations_connection.as_mut().ok_or_else(|| {
                    "Durable recovery operations unexpectedly closed.".to_string()
                })?,
                recovery_merge::OPERATIONS_RECOVERY_TABLES,
            )?;
            recovered_records = recovered_records
                .saturating_add(report.new_records)
                .saturating_add(report.source_newer_records);
            skipped_records = skipped_records
                .saturating_add(report.identical_records)
                .saturating_add(report.durable_newer_records);
        } else if replace_operations {
            recovered_records = recovered_records.saturating_add(source_operations_records);
        } else {
            skipped_records = skipped_records
                .saturating_add(operations_assessment.identical_records)
                .saturating_add(operations_assessment.durable_newer_records);
        }
        drop(durable_operations_connection);
        drop(durable_connection);
        drop(source_operations);

        let state_replacement_path = if replace_state {
            let mut random = [0u8; 16];
            OsRng.fill_bytes(&mut random);
            let path = durable_path
                .with_extension(format!("reconcile-{}.sqlcipher_tmp", hex::encode(random)));
            fs::copy(&source_path, &path).map_err(|error| error.to_string())?;
            set_private_file(&path).map_err(|error| error.to_string())?;
            let validation = (|| {
                let replacement = open_sqlcipher_database_connection_with_key(&path, &key)
                    .map_err(|error| error.to_string())?;
                verify_migration_ledger(&replacement).map_err(|error| error.to_string())?;
                verify_schema_invariants(&replacement, MIGRATIONS.len() as i64)
                    .map_err(|error| error.to_string())?;
                durable_read_write_probe(&replacement).map_err(|error| error.to_string())
            })();
            if let Err(error) = validation {
                remove_sqlite_sidecars(&path);
                let _ = fs::remove_file(&path);
                return Err(error);
            }
            remove_sqlite_sidecars(&path);
            Some(path)
        } else {
            None
        };

        let operations_replacement_path = if replace_operations {
            let mut operations_random = [0u8; 16];
            OsRng.fill_bytes(&mut operations_random);
            let path = durable_operations_path.with_extension(format!(
                "reconcile-{}.sqlcipher_tmp",
                hex::encode(operations_random)
            ));
            create_verified_operations_copy(&source_operations_path, &path, &key)
                .map_err(|error| error.to_string())?;
            Some(path)
        } else {
            None
        };

        if let Some(operations_replacement_path) = operations_replacement_path.as_ref() {
            remove_sqlite_sidecars(&durable_operations_path);
            if let Err(error) = fs::rename(operations_replacement_path, &durable_operations_path) {
                remove_sqlite_sidecars(operations_replacement_path);
                let _ = fs::remove_file(operations_replacement_path);
                if let Some(state_replacement_path) = state_replacement_path.as_ref() {
                    remove_sqlite_sidecars(state_replacement_path);
                    let _ = fs::remove_file(state_replacement_path);
                }
                return Err(error.to_string());
            }
            remove_sqlite_sidecars(&durable_operations_path);
            set_private_file(&durable_operations_path).map_err(|error| error.to_string())?;
        }

        if let Some(state_replacement_path) = state_replacement_path.as_ref() {
            // Remove stale WAL/SHM sidecars before replacing SQLite.
            remove_sqlite_sidecars(&durable_path);
            if let Err(error) = fs::rename(state_replacement_path, &durable_path) {
                remove_sqlite_sidecars(state_replacement_path);
                let _ = fs::remove_file(state_replacement_path);
                return Err(error.to_string());
            }
            remove_sqlite_sidecars(&durable_path);
            set_private_file(&durable_path).map_err(|error| error.to_string())?;
        }
        let durable = open_sqlcipher_database_connection_with_key(&durable_path, &key)
            .map_err(|error| error.to_string())?;
        verify_migration_ledger(&durable).map_err(|error| error.to_string())?;
        verify_schema_invariants(&durable, MIGRATIONS.len() as i64)
            .map_err(|error| error.to_string())?;
        durable_read_write_probe(&durable).map_err(|error| error.to_string())?;
        drop(durable);
        let durable_operations =
            open_sqlcipher_database_connection_with_key(&durable_operations_path, &key)
                .map_err(|error| error.to_string())?;
        verify_operations_database(&durable_operations).map_err(|error| error.to_string())?;

        *self
            .db_path
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = durable_path.clone();
        *self
            .storage_class
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = BackingStoreClass::Persistent;

        Ok(PersistenceRecoveryReport {
            recovered_records,
            skipped_records,
            conflicting_records,
            failed_records: 0,
            durable_probe_verified: true,
            requires_confirmation: false,
            backup_created: state_backup_path.is_some() || operations_backup_path.is_some(),
        })
    }

    pub fn probe_active_durable_store(&self) -> Result<(), String> {
        self.require_durable_store("durable persistence recovery probe")?;
        let _guard = self.lock_writes();
        let connection = self.open_connection().map_err(|error| error.to_string())?;
        verify_migration_ledger(&connection).map_err(|error| error.to_string())?;
        verify_schema_invariants(&connection, MIGRATIONS.len() as i64)
            .map_err(|error| error.to_string())?;
        durable_read_write_probe(&connection).map_err(|error| error.to_string())?;
        let operations = self
            .open_ops_connection()
            .map_err(|error| error.to_string())?;
        verify_operations_database(&operations).map_err(|error| error.to_string())
    }

    pub fn restore_migration_backup(&self, backup_path: &Path) -> Result<(), String> {
        let _guard = self.lock_writes();
        let durable_path = project_root().join(DB_FILE);
        let expected_directory = durable_path
            .parent()
            .ok_or_else(|| "Durable database path has no parent directory.".to_string())?
            .join(".oomu-migration-backups");
        let canonical_backup = backup_path
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let canonical_directory = expected_directory
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if canonical_backup.parent() != Some(canonical_directory.as_path()) {
            return Err("Migration restore path is outside the private backup directory.".into());
        }
        let key = get_database_key()?;
        if durable_path.exists() {
            let current = open_sqlcipher_database_connection_with_key(&durable_path, &key)
                .map_err(|error| error.to_string())?;
            current
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .map_err(|error| error.to_string())?;
        }
        let backup = open_sqlcipher_database_connection_with_key(&canonical_backup, &key)
            .map_err(|error| error.to_string())?;
        let integrity: String = backup
            .pragma_query_value(None, "integrity_check", |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if integrity != "ok" {
            return Err(format!(
                "Migration backup failed integrity validation: {integrity}"
            ));
        }
        verify_migration_ledger(&backup).map_err(|error| error.to_string())?;
        drop(backup);

        let mut random = [0u8; 16];
        OsRng.fill_bytes(&mut random);
        let replacement =
            durable_path.with_extension(format!("restore-{}.sqlcipher_tmp", hex::encode(random)));
        fs::copy(&canonical_backup, &replacement).map_err(|error| error.to_string())?;
        set_private_file(&replacement).map_err(|error| error.to_string())?;
        let replacement_validation = (|| {
            let replacement_connection =
                open_sqlcipher_database_connection_with_key(&replacement, &key)
                    .map_err(|error| error.to_string())?;
            let replacement_integrity: String = replacement_connection
                .pragma_query_value(None, "integrity_check", |row| row.get(0))
                .map_err(|error| error.to_string())?;
            if replacement_integrity != "ok" {
                return Err(format!(
                    "Migration restore replacement failed integrity validation: {replacement_integrity}"
                ));
            }
            verify_migration_ledger(&replacement_connection).map_err(|error| error.to_string())
        })();
        if let Err(error) = replacement_validation {
            remove_sqlite_sidecars(&replacement);
            let _ = fs::remove_file(&replacement);
            return Err(error);
        }
        remove_sqlite_sidecars(&replacement);

        // Remove sidecars from the database being replaced before the main-file rename.
        // The active write lock prevents application writers during this boundary.
        remove_sqlite_sidecars(&durable_path);
        if let Err(error) = fs::rename(&replacement, &durable_path) {
            remove_sqlite_sidecars(&replacement);
            let _ = fs::remove_file(&replacement);
            return Err(error.to_string());
        }
        remove_sqlite_sidecars(&durable_path);
        set_private_file(&durable_path).map_err(|error| error.to_string())?;
        let restored = open_sqlcipher_database_connection_with_key(&durable_path, &key)
            .map_err(|error| error.to_string())?;
        let integrity: String = restored
            .pragma_query_value(None, "integrity_check", |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if integrity != "ok" {
            return Err(format!(
                "Restored database failed integrity validation: {integrity}"
            ));
        }
        Ok(())
    }

    pub fn audit_recovery(&self) {
        agent_execution_restart::audit_recovery(self);
    }

    pub async fn save_intent(&self, plan: ActionPlan) -> Result<(), String> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || engine.insert_intent(&plan))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }

    pub async fn save_action_result(
        &self,
        plan_id: String,
        tool: String,
        input: String,
        output: Option<String>,
        status: String,
    ) -> Result<i64, String> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            engine.insert_action(&plan_id, &tool, &input, output.as_deref(), &status)
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
    }

    pub async fn save_certificate(
        &self,
        plan_id: String,
        action_id: Option<i64>,
        mlc_path: String,
        mlc_content: String,
    ) -> Result<(), String> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            engine.insert_certificate(&plan_id, action_id, &mlc_path, &mlc_content)
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
    }

    pub async fn save_plan_generation_state(
        &self,
        plan_id: String,
        plan_json: String,
        current_step_index: usize,
        status: String,
        generated_text: String,
    ) -> Result<(), String> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            engine.upsert_plan_generation_state(
                &plan_id,
                &plan_json,
                current_step_index as i64,
                &status,
                &generated_text,
            )
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
    }

    pub async fn load_state(&self) -> Result<AgenticState, String> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || engine.select_state())
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }

    pub async fn update_action_result(
        &self,
        action_id: i64,
        output: Option<String>,
        status: String,
    ) -> Result<(), String> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            engine.update_action(action_id, output.as_deref(), &status)
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
    }

    fn run_migrations(&self) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let key = get_database_key().map_err(database_key_error)?;
        self.run_migrations_with_database_key(&key)
    }

    fn prepare_migration_schema(connection: &Connection) -> rusqlite::Result<()> {
        configure_incremental_auto_vacuum(connection)?;
        connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        initialize_migration_ledger(connection)?;
        verify_migration_ledger(connection)?;
        if table_exists(connection, "execution_instances_before_approval_gateway")? {
            return Err(migration_recovery_error(
                "partial 0004 table rebuild detected; restore the verified pre-migration backup",
            ));
        }
        Ok(())
    }

    fn run_migrations_with_database_key(&self, database_key: &str) -> rusqlite::Result<()> {
        let _migration_lock = MigrationFileLock::acquire(&self.current_db_path())?;
        let mut connection =
            open_sqlcipher_database_connection_with_key(&self.current_db_path(), database_key)?;
        Self::prepare_migration_schema(&connection)?;
        let enc_state = get_current_encryption_state();
        let workspace_id = self.workspace_id.as_str();
        // MIGRATION_IMPL_BEGIN:0001_core_schema
        let migration_default_context_budget = 8_192usize;
        let sql = format!(
            "
            CREATE TABLE IF NOT EXISTS intents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                plan_id TEXT NOT NULL UNIQUE,
                prompt TEXT NOT NULL,
                metadata TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                encryption_state TEXT NOT NULL DEFAULT '{}'
            );
            CREATE TABLE IF NOT EXISTS actions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                plan_id TEXT NOT NULL,
                tool TEXT NOT NULL,
                input TEXT NOT NULL,
                output TEXT,
                status TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                encryption_state TEXT NOT NULL DEFAULT '{}'
            );

            CREATE TABLE IF NOT EXISTS certificates (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                plan_id TEXT NOT NULL,
                action_id INTEGER,
                mlc_path TEXT NOT NULL,
                mlc_content TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                encryption_state TEXT NOT NULL DEFAULT '{}',
                FOREIGN KEY(action_id) REFERENCES actions(id)
            );

            CREATE TABLE IF NOT EXISTS plan_generation_states (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                plan_id TEXT NOT NULL UNIQUE,
                plan_json TEXT NOT NULL,
                current_step_index INTEGER NOT NULL,
                status TEXT NOT NULL,
                generated_text TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                encryption_state TEXT NOT NULL DEFAULT '{}'
            );

            CREATE INDEX IF NOT EXISTS idx_actions_status ON actions(status);
            CREATE INDEX IF NOT EXISTS idx_actions_plan_id ON actions(plan_id);
            CREATE INDEX IF NOT EXISTS idx_certificates_plan_id ON certificates(plan_id);
            CREATE INDEX IF NOT EXISTS idx_plan_generation_states_status ON plan_generation_states(status);

            CREATE TABLE IF NOT EXISTS agent_execution_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                execution_id TEXT NOT NULL,
                plan_id TEXT NOT NULL,
                session_id TEXT,
                agent_id TEXT,
                level TEXT NOT NULL,
                phase TEXT NOT NULL,
                message TEXT NOT NULL,
                payload_json TEXT,
                created_at_ms INTEGER NOT NULL,
                encryption_state TEXT NOT NULL DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_agent_execution_logs_execution_id_id
                ON agent_execution_logs(execution_id, id);
            CREATE INDEX IF NOT EXISTS idx_agent_execution_logs_plan_id
                ON agent_execution_logs(plan_id, created_at_ms);

            CREATE TABLE IF NOT EXISTS agent_executions (
                execution_id TEXT PRIMARY KEY,
                plan_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                model_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                generation_token TEXT NOT NULL,
                parent_turn_id TEXT,
                root_turn_id TEXT NOT NULL,
                turn_kind TEXT NOT NULL,
                context_json TEXT NOT NULL CHECK (json_valid(context_json)),
                status TEXT NOT NULL CHECK(status IN ('running', 'completed', 'failed', 'halted', 'cancelled')),
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                encryption_state TEXT NOT NULL DEFAULT '{agent_execution_enc_state}'
            );
            CREATE INDEX IF NOT EXISTS idx_agent_executions_session_status
                ON agent_executions(session_id, status, updated_at_ms);
            CREATE INDEX IF NOT EXISTS idx_agent_executions_turn_generation
                ON agent_executions(turn_id, generation_token);

            CREATE TABLE IF NOT EXISTS chat_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                workspace_id TEXT NOT NULL DEFAULT '{}',
                session_id TEXT NOT NULL DEFAULT '',
                agent_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                provider_id TEXT,
                model_id TEXT,
                metadata_json TEXT,
                is_compacted INTEGER NOT NULL DEFAULT 0,
                compaction_type TEXT,
                timestamp_ms INTEGER NOT NULL,
                encryption_state TEXT NOT NULL DEFAULT '{}'
            );
            CREATE TABLE IF NOT EXISTS chat_sessions (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL DEFAULT '{}',
                agent_id TEXT NOT NULL,
                title TEXT NOT NULL,
                title_source TEXT NOT NULL DEFAULT 'auto',
                provider_id TEXT NOT NULL,
                model_id TEXT NOT NULL,
                web_grounding_override INTEGER,
                dynamic_routing_override INTEGER,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                encryption_state TEXT NOT NULL DEFAULT '{}'
            );
            CREATE TABLE IF NOT EXISTS chat_turns (
                turn_id TEXT PRIMARY KEY,
                generation_token TEXT NOT NULL UNIQUE,
                workspace_id TEXT NOT NULL DEFAULT '{}',
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                model_id TEXT NOT NULL,
                parent_turn_id TEXT,
                root_turn_id TEXT NOT NULL,
                turn_kind TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('running', 'completed', 'failed', 'cancelled', 'escalated')),
                created_at_ms INTEGER NOT NULL,
                completed_at_ms INTEGER,
                encryption_state TEXT NOT NULL DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_chat_messages_agent_id ON chat_messages(agent_id);
            CREATE INDEX IF NOT EXISTS idx_chat_sessions_updated_at ON chat_sessions(updated_at_ms);
            CREATE INDEX IF NOT EXISTS idx_chat_turns_session_created
                ON chat_turns(session_id, created_at_ms);
            CREATE INDEX IF NOT EXISTS idx_chat_turns_generation
                ON chat_turns(generation_token);

            CREATE TABLE IF NOT EXISTS workflows (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                steps TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                encryption_state TEXT NOT NULL DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_workflows_updated_at ON workflows(updated_at);

            CREATE TABLE IF NOT EXISTS workflow_approvals (
                approval_token TEXT PRIMARY KEY,
                workflow_instance_id TEXT NOT NULL,
                node_id TEXT NOT NULL,
                target_tool_name TEXT NOT NULL,
                arguments_hash TEXT NOT NULL,
                decision TEXT NOT NULL CHECK(decision IN ('approve', 'deny')),
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_workflow_approvals_lookup
                ON workflow_approvals(
                    workflow_instance_id,
                    node_id,
                    target_tool_name,
                    arguments_hash,
                    expires_at
                );
            CREATE INDEX IF NOT EXISTS idx_workflow_approvals_expires_at
                ON workflow_approvals(expires_at);

            CREATE TABLE IF NOT EXISTS routing_preferences (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                encryption_state TEXT NOT NULL DEFAULT '{}'
            );

            CREATE TABLE IF NOT EXISTS app_preferences (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                encryption_state TEXT NOT NULL DEFAULT '{}'
            );

            CREATE TABLE IF NOT EXISTS user_routing_preferences (
                key TEXT PRIMARY KEY,
                primary_route_id TEXT,
                fallback_route_id TEXT,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS active_session_configs (
                session_id TEXT PRIMARY KEY,
                reasoning_depth TEXT DEFAULT 'medium',
                context_budget INTEGER DEFAULT {default_context_budget},
                model_id TEXT,
                provider_id TEXT,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS message_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                turn_id TEXT,
                generation_token TEXT,
                parent_turn_id TEXT,
                root_turn_id TEXT,
                turn_kind TEXT,
                session_id TEXT,
                agent_id TEXT NOT NULL,
                message TEXT NOT NULL,
                attachments_json TEXT NOT NULL DEFAULT '[]',
                provider_id TEXT,
                model_id TEXT,
                reasoning TEXT,
                context_limit TEXT,
                steering TEXT,
                automated_web_grounding_enabled INTEGER,
                dynamic_routing_override INTEGER, auto_route_identity_json TEXT,
                status TEXT NOT NULL DEFAULT 'queued',
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                executed_at_ms INTEGER,
                error_message TEXT,
                encryption_state TEXT NOT NULL DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_message_queue_status_created
                ON message_queue(status, created_at_ms);
            CREATE INDEX IF NOT EXISTS idx_message_queue_session_status
                ON message_queue(session_id, status, created_at_ms);

            CREATE TABLE IF NOT EXISTS gateway_message_receipts (
                platform TEXT NOT NULL,
                message_id TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('processing', 'completed')),
                received_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                PRIMARY KEY(platform, message_id)
            );
            CREATE INDEX IF NOT EXISTS idx_gateway_message_receipts_updated
                ON gateway_message_receipts(updated_at_ms);

            CREATE TABLE IF NOT EXISTS channel_configs (
                platform TEXT PRIMARY KEY
                    CHECK(platform IN ('signal', 'whatsapp', 'telegram', 'discord')),
                is_active INTEGER NOT NULL DEFAULT 0 CHECK(is_active IN (0, 1)),
                credentials_json TEXT NOT NULL DEFAULT '{{}}',
                owner_id TEXT NOT NULL DEFAULT '',
                updated_at_ms INTEGER NOT NULL DEFAULT 0,
                encryption_state TEXT NOT NULL DEFAULT '{}'
            );
            CREATE TRIGGER IF NOT EXISTS validate_whatsapp_owner_on_insert
            BEFORE INSERT ON channel_configs
            WHEN NEW.platform = 'whatsapp'
              AND NEW.is_active = 1
              AND length(trim(NEW.owner_id)) = 0
            BEGIN
                SELECT RAISE(ABORT, 'active WhatsApp requires an allowlisted owner');
            END;
            CREATE TRIGGER IF NOT EXISTS validate_whatsapp_owner_on_update
            BEFORE UPDATE ON channel_configs
            WHEN NEW.platform = 'whatsapp'
              AND NEW.is_active = 1
              AND length(trim(NEW.owner_id)) = 0
            BEGIN
                SELECT RAISE(ABORT, 'active WhatsApp requires an allowlisted owner');
            END;
            ",
            enc_state,
            enc_state,
            enc_state,
            enc_state,
            enc_state,
            workspace_id,
            enc_state,
            workspace_id,
            enc_state,
            workspace_id,
            enc_state,
            enc_state,
            enc_state,
            enc_state,
            enc_state,
            enc_state,
            default_context_budget = migration_default_context_budget,
            agent_execution_enc_state = enc_state
        );
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[0],
            |transaction| {
                transaction.execute_batch(&sql)?;
                seed_channel_configs(transaction, enc_state)
            },
        )?;
        // MIGRATION_IMPL_END:0001_core_schema
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[1],
            |transaction| transaction.execute_batch(WORKFLOW_EXECUTION_MIGRATION),
        )?;
        // MIGRATION_IMPL_BEGIN:0003_workflow_compilation_status
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[2],
            |transaction| {
                add_column_if_missing(
                    transaction,
                    "workflow_blueprints",
                    "compilation_status",
                    "
                    ALTER TABLE workflow_blueprints
                    ADD COLUMN compilation_status TEXT NOT NULL DEFAULT 'Draft'
                    CHECK (compilation_status IN ('Draft', 'Compiling', 'Compiled', 'Failed'))
                    ",
                )?;
                add_column_if_missing(
                    transaction,
                    "workflow_blueprints",
                    "compilation_error",
                    "ALTER TABLE workflow_blueprints ADD COLUMN compilation_error TEXT",
                )?;
                transaction.execute_batch(
                    "CREATE INDEX IF NOT EXISTS idx_workflow_blueprints_compilation_status
                     ON workflow_blueprints(compilation_status, updated_at_ms DESC);",
                )
            },
        )?;
        // MIGRATION_IMPL_END:0003_workflow_compilation_status
        // MIGRATION_IMPL_BEGIN:0004_workflow_approval_gateway
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[3],
            |transaction| {
                if column_exists(transaction, "execution_instances", "memory_json")? {
                    Ok(())
                } else {
                    transaction.execute_batch(WORKFLOW_APPROVAL_GATEWAY_MIGRATION)
                }
            },
        )?;
        // MIGRATION_IMPL_END:0004_workflow_approval_gateway
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[4],
            |transaction| transaction.execute_batch(WORKFLOW_SCHEDULES_MIGRATION),
        )?;
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[5],
            |transaction| {
                run_chat_context_migration(transaction, workspace_id)?;
                Ok(())
            },
        )?;
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[6],
            |transaction| {
                self.run_sovereign_trust_migration(transaction, enc_state)?;
                Ok(())
            },
        )?;
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[7],
            |_transaction| Ok(()),
        )?;
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[8],
            |_transaction| self.initialize_operations_schema(database_key),
        )?;
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[9],
            |transaction| transaction.execute_batch(PROJECT_WORKSPACES_MIGRATION),
        )?;
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[10],
            |transaction| transaction.execute_batch(TASK_CONTROL_PLANE_MIGRATION),
        )?;
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[11],
            |transaction| transaction.execute_batch(CAPABILITY_CONNECTORS_MIGRATION),
        )?;
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[12],
            |transaction| transaction.execute_batch(ROUTINES_BACKGROUND_MIGRATION),
        )?;
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[13],
            |transaction| transaction.execute_batch(GUARDED_BROWSER_AUTOMATION_MIGRATION),
        )?;
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[14],
            |transaction| transaction.execute_batch(VERIFIED_ARTIFACT_PIPELINE_MIGRATION),
        )?;
        self.apply_migration(
            &mut connection,
            database_key,
            MIGRATIONS[15],
            |transaction| transaction.execute_batch(CONSTRAINED_DELEGATION_TRUST_UX_MIGRATION),
        )?;
        static_migrations::apply(self, &mut connection, database_key)?;
        verify_migration_ledger(&connection)?;
        verify_schema_invariants(&connection, MIGRATIONS.len() as i64)?;
        // A completed companion-store migration must validate existing durable
        // history rather than silently replacing it with an empty database.
        let operations = self.open_ops_connection_with_key(database_key)?;
        verify_operations_database(&operations)?;
        Ok(())
    }
    fn apply_migration<F>(
        &self,
        connection: &mut Connection,
        database_key: &str,
        migration: MigrationDescriptor,
        apply: F,
    ) -> rusqlite::Result<()>
    where
        F: FnOnce(&rusqlite::Transaction<'_>) -> rusqlite::Result<()>,
    {
        if migration_completed(connection, migration)? {
            verify_schema_invariants(connection, migration.sequence)?;
            return Ok(());
        }

        let backup_path = if migration.destructive {
            Some(create_verified_migration_backup(
                connection,
                &self.current_db_path(),
                database_key,
                migration.id,
            )?)
        } else {
            None
        };

        if migration.destructive {
            connection.pragma_update(None, "foreign_keys", "OFF")?;
        }

        let result = (|| {
            let transaction =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

            // The fast-path check above is only a hint. Another app process may
            // have completed this migration while this connection waited for
            // the SQLite writer lock. Re-read the ledger inside the acquired
            // transaction before inserting or running any DDL.
            if migration_completed(&transaction, migration)? {
                verify_schema_invariants(&transaction, migration.sequence)?;
                return transaction.commit();
            }

            let completed_count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM schema_migration_ledger WHERE state = 'completed'",
                [],
                |row| row.get(0),
            )?;
            if completed_count != migration.sequence - 1 {
                return Err(migration_recovery_error(&format!(
                    "migration {} is out of order: expected {} completed predecessors, found {completed_count}",
                    migration.id,
                    migration.sequence - 1
                )));
            }

            let checksum = migration_checksum(migration)?;
            transaction.execute(
                "
                INSERT INTO schema_migration_ledger (
                    sequence, migration_id, checksum_sha256, state,
                    application_version, started_at_ms, completed_at_ms, backup_path
                ) VALUES (?1, ?2, ?3, 'running', ?4, ?5, NULL, ?6)
                ",
                params![
                    migration.sequence,
                    migration.id,
                    checksum,
                    env!("CARGO_PKG_VERSION"),
                    unix_time_ms(),
                    backup_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().to_string())
                ],
            )?;
            apply(&transaction)?;
            verify_schema_invariants(&transaction, migration.sequence)?;
            let updated = transaction.execute(
                "
                UPDATE schema_migration_ledger
                SET state = 'completed', completed_at_ms = MAX(?1, started_at_ms)
                WHERE sequence = ?2 AND migration_id = ?3 AND state = 'running'
                ",
                params![unix_time_ms(), migration.sequence, migration.id],
            )?;
            if updated != 1 {
                return Err(migration_recovery_error(&format!(
                    "migration {} completion ledger update was not unique",
                    migration.id
                )));
            }
            transaction.commit()
        })();

        if migration.destructive {
            let foreign_key_restore = connection.pragma_update(None, "foreign_keys", "ON");
            if let Err(error) = result {
                foreign_key_restore?;
                return Err(error);
            }
            foreign_key_restore?;
            let foreign_keys_enabled: i64 =
                connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
            if foreign_keys_enabled != 1 {
                return Err(migration_recovery_error(
                    "foreign-key enforcement was not restored after destructive migration",
                ));
            }
        } else {
            result?;
        }

        verify_migration_ledger(connection)
    }

    fn current_db_path(&self) -> PathBuf {
        self.db_path
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn claim_gateway_message(
        &self,
        platform: &str,
        message_id: &str,
        received_at_ms: i64,
    ) -> rusqlite::Result<bool> {
        let platform = normalize_channel_platform(platform)?;
        let message_id = message_id.trim();
        if message_id.is_empty() || message_id.len() > 512 {
            return Err(rusqlite::Error::InvalidParameterName(
                "gateway message_id must contain 1 to 512 characters.".to_string(),
            ));
        }
        let now = unix_time_ms();
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "DELETE FROM gateway_message_receipts WHERE updated_at_ms < ?1",
            params![now - GATEWAY_MESSAGE_RECEIPT_RETENTION_MS],
        )?;
        let changed = connection.execute(
            "
            INSERT INTO gateway_message_receipts (
                platform, message_id, status, received_at_ms, updated_at_ms
            ) VALUES (?1, ?2, 'processing', ?3, ?4)
            ON CONFLICT(platform, message_id) DO UPDATE SET
                status = 'processing',
                received_at_ms = excluded.received_at_ms,
                updated_at_ms = excluded.updated_at_ms
            WHERE gateway_message_receipts.status = 'processing'
              AND gateway_message_receipts.updated_at_ms < ?5
            ",
            params![
                platform,
                message_id,
                received_at_ms,
                now,
                now - GATEWAY_MESSAGE_PROCESSING_LEASE_MS,
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn finish_gateway_message(
        &self,
        platform: &str,
        message_id: &str,
        delivered: bool,
    ) -> rusqlite::Result<()> {
        let platform = normalize_channel_platform(platform)?;
        let message_id = message_id.trim();
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        if delivered {
            connection.execute(
                "
                UPDATE gateway_message_receipts
                SET status = 'completed', updated_at_ms = ?3
                WHERE platform = ?1 AND message_id = ?2 AND status = 'processing'
                ",
                params![platform, message_id, unix_time_ms()],
            )?;
        } else {
            connection.execute(
                "
                DELETE FROM gateway_message_receipts
                WHERE platform = ?1 AND message_id = ?2 AND status = 'processing'
                ",
                params![platform, message_id],
            )?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn select_channel_configs(&self) -> rusqlite::Result<Vec<ChannelConfigRecord>> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let configs = {
            let mut statement = connection.prepare(
                "
                SELECT platform, is_active, credentials_json, owner_id, updated_at_ms
                FROM channel_configs
                ORDER BY
                    CASE platform
                        WHEN 'signal' THEN 0
                        WHEN 'whatsapp' THEN 1
                        WHEN 'telegram' THEN 2
                        WHEN 'discord' THEN 3
                        ELSE 4
                    END
                ",
            )?;
            let rows = statement
                .query_map([], channel_config_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        hydrate_channel_credentials(&connection, configs)
    }

    pub(crate) fn select_channel_config_summaries(
        &self,
    ) -> rusqlite::Result<Vec<ChannelConfigSummary>> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let mut statement = connection.prepare(
            "
            SELECT platform, is_active, credentials_json, owner_id, updated_at_ms
            FROM channel_configs
            WHERE platform IN ('telegram', 'discord', 'slack')
            ORDER BY
                CASE platform
                    WHEN 'telegram' THEN 0
                    WHEN 'discord' THEN 1
                    WHEN 'slack' THEN 2
                    ELSE 3
                END
            ",
        )?;
        let configs = statement
            .query_map([], channel_config_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(configs.iter().map(ChannelConfigSummary::from).collect())
    }

    pub(crate) fn select_active_channel_configs(
        &self,
    ) -> rusqlite::Result<Vec<ChannelConfigRecord>> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let configs = {
            let mut statement = connection.prepare(
                "
                SELECT platform, is_active, credentials_json, owner_id, updated_at_ms
                FROM channel_configs
                WHERE is_active = 1
                  AND platform IN ('telegram', 'discord', 'slack')
                ORDER BY platform
                ",
            )?;
            let rows = statement
                .query_map([], channel_config_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        hydrate_channel_credentials(&connection, configs)
    }

    pub(crate) fn select_channel_config(
        &self,
        platform: &str,
    ) -> rusqlite::Result<Option<ChannelConfigRecord>> {
        let platform = normalize_channel_platform(platform)?;
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let config = connection
            .query_row(
                "
                SELECT platform, is_active, credentials_json, owner_id, updated_at_ms
                FROM channel_configs
                WHERE platform = ?1
                ",
                params![platform],
                channel_config_from_row,
            )
            .optional()?;
        hydrate_channel_credentials(&connection, config.into_iter().collect())
            .map(|mut configs| configs.pop())
    }

    pub(crate) fn upsert_channel_config(
        &self,
        request: SaveChannelConfigRequest,
    ) -> rusqlite::Result<ChannelConfigRecord> {
        let platform = normalize_channel_platform(&request.platform)?;
        let credentials_json = request
            .credentials_json
            .map(|value| value.trim().to_string());
        if credentials_json
            .as_ref()
            .is_some_and(|value| value.is_empty() || serde_json::from_str::<Value>(value).is_err())
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "channel credentials_json must be non-empty valid JSON.".to_string(),
            ));
        }
        if credentials_json.as_ref().is_some_and(|value| {
            !serde_json::from_str::<Value>(value).is_ok_and(|value| value.is_object())
        }) {
            return Err(rusqlite::Error::InvalidParameterName(
                "channel credentials_json must be a JSON object.".to_string(),
            ));
        }
        let owner_id = request.owner_id.map(|value| value.trim().to_string());
        let now = unix_time_ms();
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let previous_secrets = if credentials_json.is_some() {
            crate::secret_store::get_channel_secrets(&platform).map_err(database_key_error)?
        } else {
            None
        };
        let persisted_credentials = if let Some(credentials) = credentials_json.as_deref() {
            crate::secret_store::set_channel_secrets(&platform, credentials)
                .map_err(database_key_error)?;
            Some(channel_credential_marker(&platform))
        } else {
            None
        };
        let write_result = connection.execute(
            "
            INSERT INTO channel_configs (
                platform,
                is_active,
                credentials_json,
                owner_id,
                updated_at_ms,
                encryption_state
            )
            VALUES (?1, ?2, COALESCE(?3, '{}'), COALESCE(?4, ''), ?5, ?6)
            ON CONFLICT(platform) DO UPDATE SET
                is_active = excluded.is_active,
                credentials_json = COALESCE(?3, channel_configs.credentials_json),
                owner_id = COALESCE(?4, channel_configs.owner_id),
                updated_at_ms = excluded.updated_at_ms,
                encryption_state = excluded.encryption_state
            ",
            params![
                platform.as_str(),
                if request.is_active { 1 } else { 0 },
                persisted_credentials,
                owner_id,
                now,
                get_current_encryption_state(),
            ],
        );
        if let Err(error) = write_result {
            if credentials_json.is_some() {
                let compensation = match previous_secrets {
                    Some(previous) => {
                        crate::secret_store::set_channel_secrets(&platform, &previous)
                    }
                    None => crate::secret_store::delete_channel_secrets(&platform),
                };
                compensation.map_err(database_key_error)?;
            }
            return Err(error);
        }
        let config = connection.query_row(
            "
            SELECT platform, is_active, credentials_json, owner_id, updated_at_ms
            FROM channel_configs
            WHERE platform = ?1
            ",
            params![platform],
            channel_config_from_row,
        )?;
        hydrate_channel_credentials(&connection, vec![config]).map(|mut configs| configs.remove(0))
    }

    // MIGRATION_IMPL_BEGIN:0007_sovereign_trust
    fn run_sovereign_trust_migration(
        &self,
        connection: &Connection,
        enc_state: &str,
    ) -> rusqlite::Result<()> {
        let migration_default_daily_token_limit = 100_000i64;
        let migration_default_daily_cpu_seconds_limit = 3_600.0f64;
        let sql = format!(
            "
            CREATE TABLE IF NOT EXISTS sovereign_trust_policies (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                directory_path TEXT NOT NULL,
                canonical_directory_path TEXT NOT NULL,
                allowed_tool_categories TEXT NOT NULL CHECK (json_valid(allowed_tool_categories)),
                permission_level TEXT NOT NULL CHECK (
                    permission_level IN ('one_time', 'session_gated', 'global_trust')
                ),
                expires_at_ms INTEGER,
                daily_token_cost_limit INTEGER NOT NULL DEFAULT {migration_default_daily_token_limit},
                daily_cpu_seconds_limit REAL NOT NULL DEFAULT {migration_default_daily_cpu_seconds_limit},
                token_cost_used_today INTEGER NOT NULL DEFAULT 0,
                cpu_seconds_used_today REAL NOT NULL DEFAULT 0,
                usage_day INTEGER NOT NULL DEFAULT 0,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                last_used_at_ms INTEGER,
                encryption_state TEXT NOT NULL DEFAULT '{enc_state}'
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_sovereign_trust_policies_scope_level
            ON sovereign_trust_policies(canonical_directory_path, permission_level);
            CREATE INDEX IF NOT EXISTS idx_sovereign_trust_policies_active
            ON sovereign_trust_policies(permission_level, expires_at_ms);

            CREATE TABLE IF NOT EXISTS active_trust_sessions (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                policy_id INTEGER,
                directory_path TEXT NOT NULL,
                canonical_directory_path TEXT NOT NULL,
                allowed_tool_categories TEXT NOT NULL CHECK (json_valid(allowed_tool_categories)),
                permission_level TEXT NOT NULL DEFAULT 'session_gated' CHECK (
                    permission_level IN ('session_gated')
                ),
                expires_at_ms INTEGER NOT NULL,
                daily_token_cost_limit INTEGER NOT NULL DEFAULT {migration_default_daily_token_limit},
                daily_cpu_seconds_limit REAL NOT NULL DEFAULT {migration_default_daily_cpu_seconds_limit},
                token_cost_used_today INTEGER NOT NULL DEFAULT 0,
                cpu_seconds_used_today REAL NOT NULL DEFAULT 0,
                usage_day INTEGER NOT NULL DEFAULT 0,
                created_at_ms INTEGER NOT NULL,
                last_used_at_ms INTEGER,
                encryption_state TEXT NOT NULL DEFAULT '{enc_state}',
                FOREIGN KEY(policy_id) REFERENCES sovereign_trust_policies(id)
            );

            CREATE INDEX IF NOT EXISTS idx_active_trust_sessions_lookup
            ON active_trust_sessions(session_id, expires_at_ms);
            CREATE INDEX IF NOT EXISTS idx_active_trust_sessions_scope
            ON active_trust_sessions(canonical_directory_path, expires_at_ms);
            "
        );
        connection.execute_batch(&sql)
    }

    // MIGRATION_IMPL_END:0007_sovereign_trust

    pub(crate) fn upsert_sovereign_trust_policy(
        &self,
        directory_path: &str,
        allowed_tool_categories: &[SovereignTrustToolCategory],
        permission_level: SovereignTrustPermissionLevel,
        expires_at_ms: Option<i64>,
        daily_token_cost_limit: Option<i64>,
        daily_cpu_seconds_limit: Option<f64>,
    ) -> rusqlite::Result<i64> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let directory = canonicalize_trust_directory(directory_path)?;
        let canonical_directory_path = directory.to_string_lossy().to_string();
        let categories_json = trust_categories_json(allowed_tool_categories)?;
        let now = unix_time_ms();
        let enc_state = get_current_encryption_state();
        let token_limit =
            daily_token_cost_limit.unwrap_or(DEFAULT_SOVEREIGN_TRUST_DAILY_TOKEN_LIMIT);
        let cpu_limit =
            daily_cpu_seconds_limit.unwrap_or(DEFAULT_SOVEREIGN_TRUST_DAILY_CPU_SECONDS_LIMIT);

        connection.execute(
            "
            INSERT INTO sovereign_trust_policies (
                directory_path,
                canonical_directory_path,
                allowed_tool_categories,
                permission_level,
                expires_at_ms,
                daily_token_cost_limit,
                daily_cpu_seconds_limit,
                usage_day,
                created_at_ms,
                updated_at_ms,
                encryption_state
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)
            ON CONFLICT(canonical_directory_path, permission_level) DO UPDATE SET
                directory_path=excluded.directory_path,
                allowed_tool_categories=excluded.allowed_tool_categories,
                expires_at_ms=excluded.expires_at_ms,
                daily_token_cost_limit=excluded.daily_token_cost_limit,
                daily_cpu_seconds_limit=excluded.daily_cpu_seconds_limit,
                updated_at_ms=excluded.updated_at_ms,
                encryption_state=excluded.encryption_state
            ",
            params![
                directory_path.trim(),
                canonical_directory_path,
                categories_json,
                permission_level.as_str(),
                expires_at_ms,
                token_limit,
                cpu_limit,
                trust_usage_day(now),
                now,
                enc_state,
            ],
        )?;

        connection.query_row(
            "
            SELECT id
            FROM sovereign_trust_policies
            WHERE canonical_directory_path = ?1 AND permission_level = ?2
            ",
            params![canonical_directory_path, permission_level.as_str()],
            |row| row.get(0),
        )
    }

    pub(crate) fn activate_sovereign_trust_session(
        &self,
        session_id: &str,
        directory_path: &str,
        allowed_tool_categories: &[SovereignTrustToolCategory],
        expires_at_ms: Option<i64>,
        daily_token_cost_limit: Option<i64>,
        daily_cpu_seconds_limit: Option<f64>,
    ) -> rusqlite::Result<String> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(database_key_error(
                "Sovereign trust session_id cannot be empty.".to_string(),
            ));
        }

        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let directory = canonicalize_trust_directory(directory_path)?;
        let canonical_directory_path = directory.to_string_lossy().to_string();
        let categories_json = trust_categories_json(allowed_tool_categories)?;
        let now = unix_time_ms();
        let expires_at_ms = expires_at_ms.unwrap_or(now + SOVEREIGN_TRUST_SESSION_DURATION_MS);
        if expires_at_ms <= now {
            return Err(database_key_error(
                "Sovereign trust session expiration must be in the future.".to_string(),
            ));
        }
        let token_limit =
            daily_token_cost_limit.unwrap_or(DEFAULT_SOVEREIGN_TRUST_DAILY_TOKEN_LIMIT);
        let cpu_limit =
            daily_cpu_seconds_limit.unwrap_or(DEFAULT_SOVEREIGN_TRUST_DAILY_CPU_SECONDS_LIMIT);
        let active_id = sha256_hex(
            format!("{session_id}:{}:{}", directory.display(), categories_json).as_bytes(),
        );
        let enc_state = get_current_encryption_state();

        let policy_id = connection
            .query_row(
                "
                SELECT id
                FROM sovereign_trust_policies
                WHERE canonical_directory_path = ?1
                  AND permission_level = 'session_gated'
                ",
                params![canonical_directory_path],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;

        connection.execute(
            "
            INSERT INTO active_trust_sessions (
                id,
                session_id,
                policy_id,
                directory_path,
                canonical_directory_path,
                allowed_tool_categories,
                permission_level,
                expires_at_ms,
                daily_token_cost_limit,
                daily_cpu_seconds_limit,
                usage_day,
                created_at_ms,
                last_used_at_ms,
                encryption_state
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'session_gated', ?7, ?8, ?9, ?10, ?11, NULL, ?12)
            ON CONFLICT(id) DO UPDATE SET
                policy_id=excluded.policy_id,
                directory_path=excluded.directory_path,
                allowed_tool_categories=excluded.allowed_tool_categories,
                expires_at_ms=excluded.expires_at_ms,
                daily_token_cost_limit=excluded.daily_token_cost_limit,
                daily_cpu_seconds_limit=excluded.daily_cpu_seconds_limit,
                encryption_state=excluded.encryption_state
            ",
            params![
                active_id,
                session_id,
                policy_id,
                directory_path.trim(),
                canonical_directory_path,
                categories_json,
                expires_at_ms,
                token_limit,
                cpu_limit,
                trust_usage_day(now),
                now,
                enc_state,
            ],
        )?;

        Ok(active_id)
    }

    pub(crate) fn select_matching_sovereign_trust_grant(
        &self,
        session_id: Option<&str>,
        target_path: &Path,
        tool_category: SovereignTrustToolCategory,
        now_ms: i64,
    ) -> rusqlite::Result<Option<SovereignTrustGrant>> {
        let connection = self.open_connection()?;
        let target_path = canonicalize_trust_target_path(target_path)?;
        if let Some(session_id) = session_id.and_then(non_empty_trimmed) {
            let mut statement = connection.prepare(
                "
                SELECT id, directory_path, canonical_directory_path, allowed_tool_categories,
                       permission_level, expires_at_ms, daily_token_cost_limit,
                       daily_cpu_seconds_limit, token_cost_used_today,
                       cpu_seconds_used_today, usage_day
                FROM active_trust_sessions
                WHERE session_id = ?1 AND expires_at_ms > ?2
                ORDER BY expires_at_ms DESC, created_at_ms DESC
                ",
            )?;
            let rows =
                statement.query_map(params![session_id, now_ms], session_trust_grant_from_row)?;
            for row in rows {
                let grant = row?;
                if trust_grant_matches(&grant, &target_path, tool_category)? {
                    return Ok(Some(grant));
                }
            }
        }

        let mut statement = connection.prepare(
            "
            SELECT id, directory_path, canonical_directory_path, allowed_tool_categories,
                   permission_level, expires_at_ms, daily_token_cost_limit,
                   daily_cpu_seconds_limit, token_cost_used_today,
                   cpu_seconds_used_today, usage_day
            FROM sovereign_trust_policies
            WHERE permission_level = 'global_trust'
              AND (expires_at_ms IS NULL OR expires_at_ms > ?1)
            ORDER BY updated_at_ms DESC
            ",
        )?;
        let rows = statement.query_map(params![now_ms], policy_trust_grant_from_row)?;
        for row in rows {
            let grant = row?;
            if trust_grant_matches(&grant, &target_path, tool_category)? {
                return Ok(Some(grant));
            }
        }

        Ok(None)
    }

    pub(crate) fn record_sovereign_trust_usage(
        &self,
        grant: &SovereignTrustGrant,
        token_cost: i64,
        cpu_seconds: f64,
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let usage_day = trust_usage_day(now_ms);
        match &grant.source {
            SovereignTrustGrantSource::Policy(policy_id) => {
                connection.execute(
                    "
                    UPDATE sovereign_trust_policies
                    SET token_cost_used_today = CASE
                            WHEN usage_day = ?1 THEN token_cost_used_today + ?2
                            ELSE ?2
                        END,
                        cpu_seconds_used_today = CASE
                            WHEN usage_day = ?1 THEN cpu_seconds_used_today + ?3
                            ELSE ?3
                        END,
                        usage_day = ?1,
                        last_used_at_ms = ?4,
                        updated_at_ms = ?4
                    WHERE id = ?5
                    ",
                    params![
                        usage_day,
                        token_cost.max(0),
                        cpu_seconds.max(0.0),
                        now_ms,
                        policy_id
                    ],
                )?;
            }
            SovereignTrustGrantSource::Session(session_grant_id) => {
                connection.execute(
                    "
                    UPDATE active_trust_sessions
                    SET token_cost_used_today = CASE
                            WHEN usage_day = ?1 THEN token_cost_used_today + ?2
                            ELSE ?2
                        END,
                        cpu_seconds_used_today = CASE
                            WHEN usage_day = ?1 THEN cpu_seconds_used_today + ?3
                            ELSE ?3
                        END,
                        usage_day = ?1,
                        last_used_at_ms = ?4
                    WHERE id = ?5
                    ",
                    params![
                        usage_day,
                        token_cost.max(0),
                        cpu_seconds.max(0.0),
                        now_ms,
                        session_grant_id
                    ],
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn select_sovereign_trust_dashboard(
        &self,
        audit_limit: usize,
    ) -> rusqlite::Result<SovereignTrustDashboardResponse> {
        let now_ms = unix_time_ms();
        Ok(SovereignTrustDashboardResponse {
            policies: self.select_sovereign_trust_policies(now_ms)?,
            active_sessions: self.select_active_sovereign_trust_sessions(now_ms)?,
            audit_events: self.select_sovereign_trust_audit_events(audit_limit)?,
        })
    }

    pub(crate) fn select_sovereign_trust_policies(
        &self,
        now_ms: i64,
    ) -> rusqlite::Result<Vec<SovereignTrustPolicyRecord>> {
        let connection = self.open_connection()?;
        let mut statement = connection.prepare(
            "
            SELECT id, directory_path, canonical_directory_path, allowed_tool_categories,
                   permission_level, expires_at_ms, daily_token_cost_limit,
                   daily_cpu_seconds_limit, token_cost_used_today,
                   cpu_seconds_used_today, usage_day, created_at_ms, updated_at_ms,
                   last_used_at_ms
            FROM sovereign_trust_policies
            ORDER BY
                CASE WHEN expires_at_ms IS NULL OR expires_at_ms > ?1 THEN 0 ELSE 1 END,
                updated_at_ms DESC
            ",
        )?;
        let rows = statement.query_map(params![now_ms], |row| {
            sovereign_trust_policy_record_from_row(row, now_ms)
        })?;
        rows.collect()
    }

    pub(crate) fn select_active_sovereign_trust_sessions(
        &self,
        now_ms: i64,
    ) -> rusqlite::Result<Vec<SovereignTrustSessionRecord>> {
        let connection = self.open_connection()?;
        let mut statement = connection.prepare(
            "
            SELECT id, session_id, policy_id, directory_path, canonical_directory_path,
                   allowed_tool_categories, permission_level, expires_at_ms,
                   daily_token_cost_limit, daily_cpu_seconds_limit,
                   token_cost_used_today, cpu_seconds_used_today, usage_day,
                   created_at_ms, last_used_at_ms
            FROM active_trust_sessions
            WHERE expires_at_ms > ?1
            ORDER BY expires_at_ms DESC, created_at_ms DESC
            ",
        )?;
        let rows = statement.query_map(params![now_ms], |row| {
            sovereign_trust_session_record_from_row(row, now_ms)
        })?;
        rows.collect()
    }

    pub(crate) fn select_sovereign_trust_audit_events(
        &self,
        limit: usize,
    ) -> rusqlite::Result<Vec<SovereignTrustAuditEvent>> {
        let connection = self.open_connection()?;
        let mut statement = connection.prepare(
            "
            SELECT id, plan_id, tool, input, output, status, timestamp_ms
            FROM actions
            WHERE tool IN (
                'shell_command',
                'file_write',
                'codebase_patch',
                'codebase_compile',
                'system_audit',
                'airlock_export'
            )
            ORDER BY id DESC
            LIMIT ?1
            ",
        )?;
        let rows = statement.query_map(
            params![limit.clamp(1, 100) as i64],
            sovereign_trust_audit_event_from_row,
        )?;
        rows.collect()
    }

    pub(crate) fn revoke_sovereign_trust_policy(&self, policy_id: i64) -> rusqlite::Result<usize> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let canonical_directory_path = transaction
            .query_row(
                "
                SELECT canonical_directory_path
                FROM sovereign_trust_policies
                WHERE id = ?1
                ",
                params![policy_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        let Some(canonical_directory_path) = canonical_directory_path else {
            transaction.commit()?;
            return Ok(0);
        };

        let session_rows = transaction.execute(
            "
            DELETE FROM active_trust_sessions
            WHERE policy_id = ?1 OR canonical_directory_path = ?2
            ",
            params![policy_id, canonical_directory_path],
        )?;
        let policy_rows = transaction.execute(
            "
            DELETE FROM sovereign_trust_policies
            WHERE id = ?1
            ",
            params![policy_id],
        )?;
        transaction.commit()?;
        Ok(policy_rows + session_rows)
    }

    pub(crate) fn revoke_sovereign_trust_session(
        &self,
        active_session_id: &str,
    ) -> rusqlite::Result<usize> {
        let active_session_id = active_session_id.trim();
        if active_session_id.is_empty() {
            return Ok(0);
        }
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            DELETE FROM active_trust_sessions
            WHERE id = ?1
            ",
            params![active_session_id],
        )
    }

    pub(crate) fn run_sqlite_maintenance_if_due(&self, now_ms: i64) -> rusqlite::Result<bool> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let last_run_ms = connection
            .query_row(
                "SELECT value FROM routing_preferences WHERE key=?1",
                params![SQLITE_MAINTENANCE_LAST_RUN_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|value| value.parse::<i64>().ok());
        if last_run_ms.is_some_and(|last_run_ms| {
            now_ms.saturating_sub(last_run_ms) < SQLITE_MAINTENANCE_INTERVAL_MS
        }) {
            return Ok(false);
        }

        run_sqlite_maintenance_on_connection(&connection)?;
        let enc_state = get_current_encryption_state();
        connection.execute(
            "
            INSERT INTO routing_preferences (key, value, updated_at, encryption_state)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(key) DO UPDATE SET
                value=excluded.value,
                updated_at=excluded.updated_at,
                encryption_state=excluded.encryption_state
            ",
            params![
                SQLITE_MAINTENANCE_LAST_RUN_KEY,
                now_ms.to_string(),
                now_ms,
                enc_state
            ],
        )?;
        Ok(true)
    }

    pub fn apply_safe_mode_boot_rules(&self) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        if table_exists(&connection, "chat_sessions")?
            && column_exists(&connection, "chat_sessions", "dynamic_routing_override")?
        {
            connection.execute(
                "
                UPDATE chat_sessions
                SET dynamic_routing_override = 0, updated_at_ms = ?1
                WHERE dynamic_routing_override IS NOT NULL
                  AND dynamic_routing_override <> 0
                ",
                params![unix_time_ms()],
            )?;
        }
        Ok(())
    }

    fn initialize_operations_schema(&self, database_key: &str) -> rusqlite::Result<()> {
        let connection = open_ops_database_connection_with_key(&self.ops_db_path(), database_key)?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS operations_store_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            INSERT INTO operations_store_metadata (key, value)
            VALUES ('schema_version', '1')
            ON CONFLICT(key) DO UPDATE SET value=excluded.value;
            ",
        )?;
        verify_operations_database(&connection)
    }

    pub fn select_workflows(&self) -> rusqlite::Result<Vec<SavedWorkflowProjectionRecord>> {
        let connection = self.open_connection()?;
        let mut statement = connection.prepare(
            "
            SELECT w.id, w.name, w.steps, w.project_id, b.version,
                   b.compilation_status, b.workflow_ir_json,
                   w.created_at, w.updated_at
            FROM workflows w
            LEFT JOIN workflow_blueprints b
              ON b.workflow_id = w.id
             AND b.version = (
                    SELECT MAX(candidate.version)
                    FROM workflow_blueprints candidate
                    WHERE candidate.workflow_id = w.id
                      AND candidate.compilation_status = 'Compiled'
                      AND candidate.workflow_ir_json IS NOT NULL
                )
            ORDER BY w.updated_at DESC, w.created_at DESC
            ",
        )?;
        let rows = statement.query_map([], workflow_projection_from_row)?;
        rows.collect()
    }

    pub fn select_latest_workflow_ir_blueprints(&self) -> rusqlite::Result<Vec<WorkflowBlueprint>> {
        let connection = self.open_connection()?;
        let mut statement = connection.prepare(
            "
            SELECT b.workflow_id, b.version, b.project_id, b.name, b.description, b.visual_state_json,
                   b.workflow_ir_json, b.compilation_status, b.compilation_error,
                   b.is_active, b.created_at_ms, b.updated_at_ms, b.compiled_at_ms
            FROM workflow_blueprints b
            INNER JOIN (
                SELECT workflow_id, MAX(version) AS version
                FROM workflow_blueprints
                WHERE workflow_ir_json IS NOT NULL
                GROUP BY workflow_id
            ) latest
              ON latest.workflow_id = b.workflow_id AND latest.version = b.version
            WHERE b.workflow_ir_json IS NOT NULL
            ORDER BY b.updated_at_ms DESC, b.created_at_ms DESC
            ",
        )?;
        let rows = statement.query_map([], workflow_blueprint_from_row)?;
        rows.collect()
    }

    pub fn upsert_workflow(&self, workflow: SavedWorkflowRecord) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let now = unix_time_ms();
        let created_at = if workflow.created_at > 0 {
            workflow.created_at
        } else {
            now
        };
        let updated_at = if workflow.updated_at > 0 {
            workflow.updated_at
        } else {
            now
        };

        connection.execute(
            "
            INSERT INTO workflows (id, name, steps, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                steps = excluded.steps,
                updated_at = excluded.updated_at
            ",
            params![
                workflow.id,
                workflow.name,
                workflow.steps,
                created_at,
                updated_at
            ],
        )?;
        Ok(())
    }

    pub fn update_workflow_last_run(&self, id: &str, last_run_at: i64) -> rusqlite::Result<bool> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let steps = connection
            .query_row(
                "SELECT steps FROM workflows WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        let Some(steps) = steps else {
            return Ok(false);
        };

        let mut visual_state = serde_json::from_str::<Value>(&steps).map_err(json_to_sql_error)?;
        if !visual_state.is_object() {
            return Err(rusqlite::Error::InvalidParameterName(
                "Stored workflow visual state is not a JSON object.".to_string(),
            ));
        }
        if let Some(object) = visual_state.as_object_mut() {
            object.insert(
                "lastRunAt".to_string(),
                Value::Number(serde_json::Number::from(last_run_at)),
            );
        }
        let updated_steps = serde_json::to_string(&visual_state).map_err(json_to_sql_error)?;
        let changed = connection.execute(
            "UPDATE workflows SET steps = ?2 WHERE id = ?1",
            params![id, updated_steps],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        let persisted_steps: String = connection.query_row(
            "SELECT steps FROM workflows WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        let persisted =
            serde_json::from_str::<Value>(&persisted_steps).map_err(json_to_sql_error)?;
        if persisted.get("lastRunAt").and_then(Value::as_i64) != Some(last_run_at) {
            return Err(rusqlite::Error::InvalidParameterName(
                "Workflow last-run timestamp could not be verified after persistence.".to_string(),
            ));
        }
        Ok(true)
    }

    pub fn create_scheduled_knowledge_sync_workflow(
        &self,
        path: &str,
        schedule_expression: &str,
        next_run_at_ms: i64,
    ) -> rusqlite::Result<ScheduledKnowledgeSyncRecord> {
        let path = canonical_knowledge_sync_path(path)?;
        let schedule_expression = schedule_expression.trim();
        if schedule_expression.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "Knowledge sync schedule expression must not be empty.".to_string(),
            ));
        }

        let path_text = path.to_string_lossy().replace('\\', "/");
        let workflow_id = knowledge_sync_workflow_id(&path_text, schedule_expression);
        let schedule_id = format!("workflow:{workflow_id}:knowledge-sync");
        let now = unix_time_ms();
        let name = format!("Knowledge Vault Sync: {path_text}");
        let visual_state = knowledge_sync_visual_state(&path_text, schedule_expression);
        let workflow = SavedWorkflowRecord {
            id: workflow_id.clone(),
            name: name.clone(),
            steps: serde_json::to_string(&visual_state).map_err(json_to_sql_error)?,
            created_at: now,
            updated_at: now,
        };
        let mut workflow_ir = knowledge_sync_workflow_ir(&workflow_id, &name, &path_text);
        let version =
            self.reserve_workflow_blueprint(&workflow, &visual_state, &mut workflow_ir)?;
        self.publish_compiled_workflow(&workflow, &workflow_ir, &[], true)?;
        let schedule = self.upsert_workflow_schedule(WorkflowScheduleUpsert {
            id: schedule_id,
            workflow_id: workflow_id.clone(),
            workflow_version: Some(version),
            label: name,
            schedule_expression: schedule_expression.to_string(),
            run_request: json!({}),
            is_active: true,
            next_run_at_ms: Some(next_run_at_ms),
        })?;

        Ok(ScheduledKnowledgeSyncRecord {
            workflow_id,
            workflow_version: version,
            schedule_id: schedule.id,
            path: path_text,
            schedule_expression: schedule.schedule_expression,
            next_run_at_ms,
        })
    }

    pub fn reserve_workflow_blueprint(
        &self,
        workflow: &SavedWorkflowRecord,
        visual_state: &Value,
        workflow_ir: &mut WorkflowIr,
    ) -> rusqlite::Result<u32> {
        self.reserve_workflow_blueprint_for_project(workflow, visual_state, workflow_ir, None)
            .map(|(version, _)| version)
    }

    pub fn reserve_workflow_blueprint_for_project(
        &self,
        workflow: &SavedWorkflowRecord,
        visual_state: &Value,
        workflow_ir: &mut WorkflowIr,
        requested_project_id: Option<&str>,
    ) -> rusqlite::Result<(u32, Option<String>)> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let project_id =
            resolve_workflow_project_binding(&transaction, &workflow.id, requested_project_id)?;
        let version = transaction.query_row(
            "
            SELECT COALESCE(MAX(version), 0) + 1
            FROM workflow_blueprints
            WHERE workflow_id = ?1
            ",
            params![workflow.id],
            |row| row.get::<_, u32>(0),
        )?;
        workflow_ir.workflow_version = version;
        let mut reserved_visual_state = visual_state.clone();
        let reserved_object = reserved_visual_state.as_object_mut().ok_or_else(|| {
            rusqlite::Error::InvalidParameterName(
                "Workflow visual state must be a JSON object.".to_string(),
            )
        })?;
        reserved_object.insert(
            "workflowIr".to_string(),
            serde_json::to_value(&*workflow_ir).map_err(json_to_sql_error)?,
        );
        reserved_object.insert("workflowVersion".to_string(), json!(version));
        reserved_object.insert("compilationStatus".to_string(), json!("Compiling"));
        match project_id.as_deref() {
            Some(project_id) => {
                reserved_object.insert("projectId".to_string(), json!(project_id));
            }
            None => {
                reserved_object.remove("projectId");
            }
        }
        let visual_state_json =
            serde_json::to_string(&reserved_visual_state).map_err(json_to_sql_error)?;
        let workflow_ir_json = serde_json::to_string(workflow_ir).map_err(json_to_sql_error)?;
        let created_at = workflow.updated_at.max(0);
        let updated_at = created_at;

        transaction.execute(
            "
            INSERT INTO workflow_blueprints (
                workflow_id,
                version,
                name,
                description,
                visual_state_json,
                workflow_ir_json,
                compilation_status,
                compilation_error,
                is_active,
                created_at_ms,
                updated_at_ms,
                compiled_at_ms,
                encryption_state,
                project_id
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'Compiling', NULL, 0, ?7, ?8, NULL, ?9, ?10)
            ",
            params![
                workflow.id,
                version,
                workflow.name,
                workflow_ir.description,
                visual_state_json,
                workflow_ir_json,
                created_at,
                updated_at,
                get_current_encryption_state(),
                project_id,
            ],
        )?;
        transaction.commit()?;
        Ok((version, project_id))
    }

    pub fn publish_compiled_workflow(
        &self,
        workflow: &SavedWorkflowRecord,
        workflow_ir: &WorkflowIr,
        instructions: &[CompiledInstruction],
        activate: bool,
    ) -> rusqlite::Result<()> {
        let visual_state =
            serde_json::from_str::<Value>(&workflow.steps).map_err(json_to_sql_error)?;
        self.publish_compiled_workflow_for_project(
            workflow,
            &visual_state,
            workflow_ir,
            instructions,
            activate,
            None,
        )
    }

    pub fn publish_compiled_workflow_for_project(
        &self,
        workflow: &SavedWorkflowRecord,
        visual_state: &Value,
        workflow_ir: &WorkflowIr,
        instructions: &[CompiledInstruction],
        activate: bool,
        project_id: Option<&str>,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let workflow_ir_json = serde_json::to_string(workflow_ir).map_err(json_to_sql_error)?;
        let visual_state_json = serde_json::to_string(visual_state).map_err(json_to_sql_error)?;
        let published_at = unix_time_ms();

        transaction.execute(
            "
            DELETE FROM compiled_instructions
            WHERE workflow_id = ?1 AND workflow_version = ?2
            ",
            params![workflow_ir.workflow_id, workflow_ir.workflow_version],
        )?;
        for instruction in instructions {
            let node_kind = serde_json::to_value(instruction.node_kind)
                .map_err(json_to_sql_error)?
                .as_str()
                .unwrap_or_default()
                .to_string();
            let input_variable_mappings_json =
                serde_json::to_string(&instruction.input_variable_mappings)
                    .map_err(json_to_sql_error)?;
            let evaluation_protocol_json = serde_json::to_string(&instruction.evaluation_protocol)
                .map_err(json_to_sql_error)?;
            transaction.execute(
                "
                INSERT INTO compiled_instructions (
                    id,
                    workflow_id,
                    workflow_version,
                    node_id,
                    node_kind,
                    system_prompt,
                    input_variable_mappings_json,
                    evaluation_protocol_json,
                    compiler_model,
                    compiler_version,
                    created_at_ms,
                    encryption_state
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                ",
                params![
                    instruction.id,
                    instruction.workflow_id,
                    instruction.workflow_version,
                    instruction.node_id,
                    node_kind,
                    instruction.system_prompt,
                    input_variable_mappings_json,
                    evaluation_protocol_json,
                    instruction.compiler_model,
                    instruction.compiler_version,
                    instruction.created_at_ms,
                    get_current_encryption_state(),
                ],
            )?;
        }

        transaction.execute(
            "
            UPDATE workflow_blueprints
            SET is_active = 0
            WHERE workflow_id = ?1
            ",
            params![workflow_ir.workflow_id],
        )?;
        let updated = transaction.execute(
            "
            UPDATE workflow_blueprints
            SET workflow_ir_json = ?3,
                visual_state_json = ?4,
                compilation_status = 'Compiled',
                compilation_error = NULL,
                is_active = ?5,
                compiled_at_ms = ?6,
                updated_at_ms = MAX(updated_at_ms, ?6)
            WHERE workflow_id = ?1 AND version = ?2 AND project_id IS ?7
            ",
            params![
                workflow_ir.workflow_id,
                workflow_ir.workflow_version,
                workflow_ir_json,
                visual_state_json,
                activate,
                published_at,
                project_id,
            ],
        )?;
        if updated != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        upsert_legacy_workflow(&transaction, workflow, project_id)?;
        transaction.commit()
    }

    pub fn mark_workflow_compilation_failed(
        &self,
        workflow_id: &str,
        workflow_version: u32,
        message: &str,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            UPDATE workflow_blueprints
            SET compilation_status = 'Failed',
                compilation_error = ?3,
                is_active = 0,
                updated_at_ms = MAX(updated_at_ms, ?4)
            WHERE workflow_id = ?1 AND version = ?2
            ",
            params![workflow_id, workflow_version, message, unix_time_ms(),],
        )?;
        Ok(())
    }

    pub fn load_compiled_workflow(
        &self,
        workflow_id: &str,
        workflow_version: Option<u32>,
    ) -> rusqlite::Result<CompiledWorkflow> {
        let connection = self.open_connection()?;
        let (workflow_ir_json, version): (String, u32) = if let Some(version) = workflow_version {
            connection.query_row(
                "
                SELECT workflow_ir_json, version
                FROM workflow_blueprints
                WHERE workflow_id = ?1
                  AND version = ?2
                  AND compilation_status = 'Compiled'
                  AND workflow_ir_json IS NOT NULL
                ",
                params![workflow_id, version],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
        } else {
            connection.query_row(
                "
                SELECT workflow_ir_json, version
                FROM workflow_blueprints
                WHERE workflow_id = ?1
                  AND compilation_status = 'Compiled'
                  AND workflow_ir_json IS NOT NULL
                ORDER BY is_active DESC, version DESC
                LIMIT 1
                ",
                params![workflow_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
        };
        let workflow_ir =
            serde_json::from_str::<WorkflowIr>(&workflow_ir_json).map_err(json_from_sql_error)?;
        let mut statement = connection.prepare(
            "
            SELECT id, workflow_id, workflow_version, node_id, node_kind, system_prompt,
                   input_variable_mappings_json, evaluation_protocol_json, compiler_model,
                   compiler_version, created_at_ms
            FROM compiled_instructions
            WHERE workflow_id = ?1 AND workflow_version = ?2
            ",
        )?;
        let rows = statement.query_map(params![workflow_id, version], |row| {
            let node_kind = parse_node_kind(&row.get::<_, String>(4)?)?;
            let input_variable_mappings =
                serde_json::from_str(&row.get::<_, String>(6)?).map_err(json_from_sql_error)?;
            let evaluation_protocol =
                serde_json::from_str(&row.get::<_, String>(7)?).map_err(json_from_sql_error)?;
            Ok(CompiledInstruction {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                workflow_version: row.get(2)?,
                node_id: row.get(3)?,
                node_kind,
                system_prompt: row.get(5)?,
                input_variable_mappings,
                evaluation_protocol,
                compiler_model: row.get(8)?,
                compiler_version: row.get(9)?,
                created_at_ms: row.get(10)?,
            })
        })?;
        let instructions = rows
            .map(|row| row.map(|instruction| (instruction.node_id.clone(), instruction)))
            .collect::<rusqlite::Result<_>>()?;
        Ok(CompiledWorkflow {
            workflow_ir,
            instructions,
        })
    }

    pub fn insert_execution_instance(&self, instance: &ExecutionInstance) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        write_execution_instance(&connection, instance, false)
    }

    pub fn insert_direct_workflow_execution_instance(
        &self,
        instance: &ExecutionInstance,
    ) -> rusqlite::Result<Option<String>> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let project_id = transaction.query_row(
            "SELECT project_id FROM workflow_blueprints WHERE workflow_id=?1 AND version=?2 AND compilation_status='Compiled' AND workflow_ir_json IS NOT NULL",
            params![instance.workflow_id, instance.workflow_version],
            |row| row.get::<_, Option<String>>(0),
        )?;
        if let Some(project_id) = project_id.as_deref() {
            ProjectId::parse(project_id)
                .map_err(|error| rusqlite::Error::InvalidParameterName(error.to_string()))?;
            let project_is_active: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1 AND archived_at_ms IS NULL)",
                params![project_id],
                |row| row.get(0),
            )?;
            if !project_is_active {
                return Err(rusqlite::Error::InvalidParameterName(
                    "The Project attached to this Workflow is unavailable.".to_string(),
                ));
            }
        }
        write_execution_instance(&transaction, instance, false)?;
        if let Some(project_id) = project_id.as_deref() {
            let bound = transaction.execute(
                "UPDATE execution_instances SET project_id=?2 WHERE id=?1 AND project_id IS NULL",
                params![instance.id, project_id],
            )?;
            if bound != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            let task_id = crate::p0_contracts::TaskId::new().to_string();
            let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
            transaction.execute(
                "INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'workflow',?4,'running','workflow',?2,?5,?6,?6,'reconciled')",
                params![
                    task_run_id,
                    task_id,
                    project_id,
                    instance.id,
                    format!("Workflow run {}", instance.workflow_id),
                    instance.created_at_ms,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(project_id)
    }

    pub fn update_execution_instance(&self, instance: &ExecutionInstance) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        write_execution_instance(&connection, instance, true)
    }

    pub fn load_execution_instance(&self, id: &str) -> rusqlite::Result<ExecutionInstance> {
        let connection = self.open_connection()?;
        select_execution_instance(&connection, id)
    }

    pub fn claim_execution_instance_for_approval(
        &self,
        id: &str,
    ) -> rusqlite::Result<ExecutionInstance> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "
            UPDATE execution_instances
            SET status = 'Running', updated_at_ms = MAX(updated_at_ms, ?2)
            WHERE id = ?1 AND status = 'AwaitingApproval'
            ",
            params![id, unix_time_ms()],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        let instance = select_execution_instance(&transaction, id)?;
        transaction.commit()?;
        Ok(instance)
    }

    pub fn record_workflow_approval(
        &self,
        approval_token: &str,
        workflow_instance_id: &str,
        node_id: &str,
        target_tool_name: &str,
        arguments: &Value,
        decision: &str,
    ) -> rusqlite::Result<()> {
        let approval_token = approval_token.trim();
        let workflow_instance_id = workflow_instance_id.trim();
        let node_id = node_id.trim();
        let target_tool_name = target_tool_name.trim();
        let decision = decision.trim();
        if approval_token.is_empty()
            || workflow_instance_id.is_empty()
            || node_id.is_empty()
            || target_tool_name.is_empty()
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "Workflow approval rows require token, instance, node, and tool identifiers."
                    .to_string(),
            ));
        }
        if !matches!(decision, "approve" | "deny") {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "Unsupported workflow approval decision: {decision}"
            )));
        }

        let created_at = unix_time_seconds();
        let expires_at = created_at + WORKFLOW_APPROVAL_TTL_SECONDS;
        let arguments_hash = hash_arguments(arguments);
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "DELETE FROM workflow_approvals WHERE expires_at <= ?1",
            params![created_at],
        )?;
        connection.execute(
            "
            INSERT INTO workflow_approvals (
                approval_token,
                workflow_instance_id,
                node_id,
                target_tool_name,
                arguments_hash,
                decision,
                created_at,
                expires_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(approval_token) DO UPDATE SET
                workflow_instance_id = excluded.workflow_instance_id,
                node_id = excluded.node_id,
                target_tool_name = excluded.target_tool_name,
                arguments_hash = excluded.arguments_hash,
                decision = excluded.decision,
                created_at = excluded.created_at,
                expires_at = excluded.expires_at
            ",
            params![
                approval_token,
                workflow_instance_id,
                node_id,
                target_tool_name,
                arguments_hash,
                decision,
                created_at,
                expires_at
            ],
        )?;
        Ok(())
    }

    pub fn verify_workflow_approval(
        &self,
        workflow_instance_id: &str,
        node_id: &str,
        target_tool_name: &str,
        arguments: &Value,
    ) -> rusqlite::Result<bool> {
        let connection = self.open_connection()?;
        verify_step_approval(
            &connection,
            workflow_instance_id,
            node_id,
            target_tool_name,
            arguments,
        )
        .map_err(rusqlite::Error::InvalidParameterName)
    }

    /// Remember a person's review for one immutable saved Workflow version.
    ///
    /// The existing `workflow_approvals` rows remain exact, short-lived
    /// continuation tokens for one execution instance. Reusable reviews use a
    /// separate synthetic subject inside the same encrypted ledger so they do
    /// not weaken or extend those one-run tokens. The lookup is bound to the
    /// immutable Workflow version, node, server, tool, and a closed approval
    /// material hash. Editing the Workflow or changing any material produces a
    /// different lookup key and therefore requires a new review.
    pub fn record_workflow_version_approval(
        &self,
        approval_token: &str,
        workflow_id: &str,
        workflow_version: u32,
        node_id: &str,
        server_name: &str,
        tool_name: &str,
        approval_material: &Value,
    ) -> rusqlite::Result<()> {
        let approval_token = approval_token.trim();
        let workflow_id = workflow_id.trim();
        let node_id = node_id.trim();
        let server_name = server_name.trim();
        let tool_name = tool_name.trim();
        if approval_token.is_empty()
            || workflow_id.is_empty()
            || workflow_version == 0
            || node_id.is_empty()
            || server_name.is_empty()
            || tool_name.is_empty()
        {
            return Err(rusqlite::Error::InvalidParameterName(
                "Reusable Workflow approval rows require a token, Workflow version, node, server, and tool."
                    .to_string(),
            ));
        }

        let created_at = unix_time_seconds();
        let workflow_subject = workflow_version_approval_subject(workflow_id, workflow_version);
        let target = workflow_version_approval_target(server_name, tool_name);
        let arguments_hash = hash_arguments(approval_material);
        let durable_token = format!("workflow-version:{approval_token}");
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "DELETE FROM workflow_approvals WHERE expires_at <= ?1",
            params![created_at],
        )?;
        connection.execute(
            "
            INSERT INTO workflow_approvals (
                approval_token,
                workflow_instance_id,
                node_id,
                target_tool_name,
                arguments_hash,
                decision,
                created_at,
                expires_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, 'approve', ?6, ?7)
            ON CONFLICT(approval_token) DO UPDATE SET
                workflow_instance_id = excluded.workflow_instance_id,
                node_id = excluded.node_id,
                target_tool_name = excluded.target_tool_name,
                arguments_hash = excluded.arguments_hash,
                decision = excluded.decision,
                created_at = excluded.created_at,
                expires_at = excluded.expires_at
            ",
            params![
                durable_token,
                workflow_subject,
                node_id,
                target,
                arguments_hash,
                created_at,
                i64::MAX,
            ],
        )?;
        Ok(())
    }

    pub fn verify_workflow_version_approval(
        &self,
        workflow_id: &str,
        workflow_version: u32,
        node_id: &str,
        server_name: &str,
        tool_name: &str,
        approval_material: &Value,
    ) -> rusqlite::Result<bool> {
        if workflow_id.trim().is_empty()
            || workflow_version == 0
            || node_id.trim().is_empty()
            || server_name.trim().is_empty()
            || tool_name.trim().is_empty()
        {
            return Ok(false);
        }
        let connection = self.open_connection()?;
        verify_step_approval(
            &connection,
            &workflow_version_approval_subject(workflow_id, workflow_version),
            node_id,
            &workflow_version_approval_target(server_name, tool_name),
            approval_material,
        )
        .map_err(rusqlite::Error::InvalidParameterName)
    }

    pub fn verify_routine_authority(
        &self,
        workflow_instance_id: &str,
        target_action: &str,
        arguments: &Value,
    ) -> rusqlite::Result<bool> {
        let connection = self.open_connection()?;
        let arguments_hash = hash_arguments(arguments);
        connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM routine_runs r JOIN routine_authority_grants g ON g.schedule_id=r.schedule_id WHERE r.execution_instance_id=?1 AND g.action_name=?2 AND g.arguments_hash=?3 AND g.revoked_at_ms IS NULL AND g.expires_at_ms>=?4)",
            params![workflow_instance_id, target_action, arguments_hash, unix_time_ms()],
            |row| row.get(0),
        )
    }

    pub fn upsert_workflow_schedule(
        &self,
        schedule: WorkflowScheduleUpsert,
    ) -> rusqlite::Result<WorkflowScheduleRecord> {
        let id = schedule.id.trim();
        let workflow_id = schedule.workflow_id.trim();
        let expression = schedule.schedule_expression.trim();
        if id.is_empty() || workflow_id.is_empty() || expression.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "Workflow schedules require id, workflow_id, and schedule_expression.".to_string(),
            ));
        }

        let run_request_json =
            serde_json::to_string(&schedule.run_request).map_err(json_to_sql_error)?;
        let now = unix_time_ms();
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            INSERT INTO workflow_schedules (
                id, workflow_id, workflow_version, label, schedule_expression,
                run_request_json, is_active, next_run_at_ms, claimed_at_ms,
                created_at_ms, updated_at_ms, encryption_state
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?9, ?10)
            ON CONFLICT(id) DO UPDATE SET
                workflow_id = excluded.workflow_id,
                workflow_version = excluded.workflow_version,
                label = excluded.label,
                schedule_expression = excluded.schedule_expression,
                run_request_json = excluded.run_request_json,
                is_active = excluded.is_active,
                next_run_at_ms = excluded.next_run_at_ms,
                claimed_at_ms = NULL,
                updated_at_ms = excluded.updated_at_ms
            ",
            params![
                id,
                workflow_id,
                schedule.workflow_version,
                schedule.label.trim(),
                expression,
                run_request_json,
                schedule.is_active,
                schedule.next_run_at_ms,
                now,
                get_current_encryption_state(),
            ],
        )?;
        select_workflow_schedule_by_id(&connection, id)
    }

    pub fn disable_workflow_schedule(&self, id: &str) -> rusqlite::Result<bool> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let changed = connection.execute(
            "
            UPDATE workflow_schedules
            SET is_active = 0,
                claimed_at_ms = NULL,
                next_run_at_ms = NULL,
                updated_at_ms = ?2
            WHERE id = ?1
            ",
            params![id.trim(), unix_time_ms()],
        )?;
        Ok(changed > 0)
    }

    pub fn claim_due_workflow_schedules(
        &self,
        now_ms: i64,
        limit: usize,
        lease_ms: i64,
    ) -> rusqlite::Result<Vec<WorkflowScheduleRecord>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let lease_cutoff_ms = now_ms.saturating_sub(lease_ms.max(0));
        let due = select_due_workflow_schedules(&transaction, now_ms, lease_cutoff_ms, limit)?;
        let mut claimed = Vec::new();
        for schedule in due {
            let changed = transaction.execute(
                "
                UPDATE workflow_schedules
                SET claimed_at_ms = ?2,
                    last_started_at_ms = ?2,
                    last_status = 'Running',
                    last_error = NULL, updated_at_ms = MAX(updated_at_ms, ?2)
                WHERE id = ?1
                  AND is_active = 1
                  AND next_run_at_ms IS NOT NULL
                  AND next_run_at_ms <= ?2
                  AND (claimed_at_ms IS NULL OR claimed_at_ms <= ?3)
                ",
                params![schedule.id.as_str(), now_ms, lease_cutoff_ms],
            )?;
            if changed == 1 {
                claimed.push(select_workflow_schedule_by_id(&transaction, &schedule.id)?);
            }
        }
        transaction.commit()?;
        Ok(claimed)
    }

    pub fn mark_workflow_schedule_run_result(
        &self,
        id: &str,
        status: ExecutionStatus,
        instance_id: Option<&str>,
        error_message: Option<&str>,
        next_run_at_ms: Option<i64>,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let now = unix_time_ms();
        let changed = connection.execute(
            "
            UPDATE workflow_schedules
            SET claimed_at_ms = NULL,
                last_completed_at_ms = CASE
                    WHEN ?3 IN ('Completed', 'Failed') THEN ?2
                    ELSE last_completed_at_ms
                END,
                last_status = ?3,
                last_error = ?4,
                last_instance_id = ?5,
                next_run_at_ms = ?6,
                updated_at_ms = ?2
            WHERE id = ?1
            ",
            params![
                id.trim(),
                now,
                execution_status(status),
                error_message.map(str::trim),
                instance_id.map(str::trim),
                next_run_at_ms,
            ],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        Ok(())
    }

    pub fn update_compiled_instruction(
        &self,
        workflow_id: &str,
        workflow_version: u32,
        node_id: &str,
        system_prompt: &str,
    ) -> rusqlite::Result<CompiledInstruction> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let changed = connection.execute(
            "
            UPDATE compiled_instructions
            SET system_prompt = ?4
            WHERE workflow_id = ?1 AND workflow_version = ?2
              AND node_id = ?3 AND node_kind = 'agent'
            ",
            params![workflow_id, workflow_version, node_id, system_prompt],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        select_compiled_instruction(&connection, workflow_id, workflow_version, node_id)
    }

    pub fn delete_workflow_by_id(&self, id: &str) -> rusqlite::Result<bool> {
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM execution_instances WHERE workflow_id = ?1",
            params![id],
        )?;
        transaction.execute(
            "DELETE FROM workflow_schedules WHERE workflow_id = ?1",
            params![id],
        )?;
        transaction.execute(
            "DELETE FROM workflow_blueprints WHERE workflow_id = ?1",
            params![id],
        )?;
        let removed = transaction.execute("DELETE FROM workflows WHERE id = ?1", params![id])?;
        transaction.commit()?;
        Ok(removed > 0)
    }

    pub(crate) fn select_app_preference(&self, key: &str) -> rusqlite::Result<Option<String>> {
        let key = key.trim();
        if key.is_empty() {
            return Ok(None);
        }
        let connection = self.open_connection()?;
        connection
            .query_row(
                "SELECT value FROM app_preferences WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
    }

    pub(crate) fn upsert_app_preference(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        let key = key.trim();
        if key.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "app preference key".to_string(),
            ));
        }
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at_ms = excluded.updated_at_ms,
                encryption_state = excluded.encryption_state
            ",
            params![key, value, unix_time_ms(), get_current_encryption_state()],
        )?;
        Ok(())
    }

    pub fn insert_agent_execution_log(
        &self,
        execution_id: &str,
        plan_id: &str,
        session_id: Option<&str>,
        agent_id: Option<&str>,
        level: &str,
        phase: &str,
        message: &str,
        payload_json: Option<&str>,
    ) -> rusqlite::Result<AgentExecutionLogRecord> {
        let execution_id = execution_id.trim();
        let plan_id = plan_id.trim();
        let level = level.trim();
        let phase = phase.trim();
        let message = message.trim();
        if execution_id.is_empty() || plan_id.is_empty() || level.is_empty() || phase.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "Agent execution logs require execution_id, plan_id, level, and phase.".to_string(),
            ));
        }

        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        if table_exists(&connection, "agent_executions")? {
            let managed_status: Option<String> = connection
                .query_row(
                    "SELECT status FROM agent_executions WHERE execution_id = ?1",
                    params![execution_id],
                    |row| row.get(0),
                )
                .optional()?;
            if managed_status
                .as_deref()
                .is_some_and(|status| status != "running")
            {
                return Err(rusqlite::Error::InvalidParameterName(
                    "agent execution is no longer authorized to append logs".to_string(),
                ));
            }
        }
        let now = unix_time_ms();
        connection.execute(
            "
            INSERT INTO agent_execution_logs (
                execution_id, plan_id, session_id, agent_id, level, phase,
                message, payload_json, created_at_ms, encryption_state
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ",
            params![
                execution_id,
                plan_id,
                session_id.map(str::trim).filter(|value| !value.is_empty()),
                agent_id.map(str::trim).filter(|value| !value.is_empty()),
                level,
                phase,
                message,
                payload_json,
                now,
                get_current_encryption_state(),
            ],
        )?;
        select_agent_execution_log_by_id(&connection, connection.last_insert_rowid())
    }

    pub fn select_agent_execution_logs_after(
        &self,
        execution_id: &str,
        after_id: i64,
        limit: usize,
    ) -> rusqlite::Result<Vec<AgentExecutionLogRecord>> {
        let execution_id = execution_id.trim();
        if execution_id.is_empty() {
            return Ok(Vec::new());
        }

        let connection = self.open_connection()?;
        let mut statement = connection.prepare(
            "
            SELECT id, execution_id, plan_id, session_id, agent_id, level,
                   phase, message, payload_json, created_at_ms
            FROM agent_execution_logs
            WHERE execution_id = ?1 AND id > ?2
            ORDER BY id ASC
            LIMIT ?3
            ",
        )?;
        let rows = statement.query_map(
            params![execution_id, after_id.max(0), limit.clamp(1, 500) as i64],
            agent_execution_log_from_row,
        )?;
        rows.collect()
    }

    fn insert_intent(&self, plan: &ActionPlan) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let metadata = serde_json::to_string(&plan.intent)
            .unwrap_or_else(|_| "{\"error\":\"intent_metadata_unavailable\"}".to_string());

        connection.execute(
            "
            INSERT INTO intents (plan_id, prompt, metadata, timestamp_ms)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(plan_id) DO UPDATE SET
                prompt = excluded.prompt,
                metadata = excluded.metadata,
                timestamp_ms = excluded.timestamp_ms
            ",
            params![&plan.id, &plan.objective, metadata, unix_time_ms()],
        )?;
        Ok(())
    }

    fn insert_action(
        &self,
        plan_id: &str,
        tool: &str,
        input: &str,
        output: Option<&str>,
        status: &str,
    ) -> rusqlite::Result<i64> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            INSERT INTO actions (plan_id, tool, input, output, status, timestamp_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ",
            params![plan_id, tool, input, output, status, unix_time_ms()],
        )?;
        Ok(connection.last_insert_rowid())
    }

    fn update_action(
        &self,
        action_id: i64,
        output: Option<&str>,
        status: &str,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            UPDATE actions
            SET output = ?1, status = ?2, timestamp_ms = ?3
            WHERE id = ?4
            ",
            params![output, status, unix_time_ms(), action_id],
        )?;
        Ok(())
    }

    fn insert_certificate(
        &self,
        plan_id: &str,
        action_id: Option<i64>,
        mlc_path: &str,
        mlc_content: &str,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            INSERT INTO certificates (plan_id, action_id, mlc_path, mlc_content, timestamp_ms)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ",
            params![plan_id, action_id, mlc_path, mlc_content, unix_time_ms()],
        )?;
        Ok(())
    }

    fn upsert_plan_generation_state(
        &self,
        plan_id: &str,
        plan_json: &str,
        current_step_index: i64,
        status: &str,
        generated_text: &str,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            INSERT INTO plan_generation_states (
                plan_id,
                plan_json,
                current_step_index,
                status,
                generated_text,
                timestamp_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(plan_id) DO UPDATE SET
                plan_json = excluded.plan_json,
                current_step_index = excluded.current_step_index,
                status = excluded.status,
                generated_text = excluded.generated_text,
                timestamp_ms = excluded.timestamp_ms
            ",
            params![
                plan_id,
                plan_json,
                current_step_index,
                status,
                generated_text,
                unix_time_ms()
            ],
        )?;
        Ok(())
    }

    fn mark_interrupted_actions(&self) -> rusqlite::Result<()> {
        let _ = self.persist_interrupted_approved_execution_recoveries();
        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE actions SET status='blocked' WHERE status='running' AND plan_id IN (
                SELECT plan_id FROM agent_executions WHERE status='running'
             )",
            [],
        )?;
        transaction.execute(
            "UPDATE actions SET status='recoverable' WHERE status='running'",
            [],
        )?;
        transaction.execute(
            "UPDATE plan_generation_states SET status = 'recoverable' WHERE status = 'running'",
            [],
        )?;
        let recovery_at_ms = unix_time_ms();
        transaction.execute(
            "INSERT INTO agent_execution_logs (
                execution_id, plan_id, session_id, agent_id, level, phase,
                message, payload_json, created_at_ms, encryption_state
             )
             SELECT execution_id, plan_id, session_id, agent_id, 'warn', 'halted',
                    'OOMU restarted. This execution can resume from its last verified checkpoint.',
                    '{\"code\":\"agent_execution_interrupted\",\"recoverable\":true}', ?1, ?2
             FROM agent_executions WHERE status = 'running'",
            params![recovery_at_ms, get_current_encryption_state()],
        )?;
        transaction.execute(
            "UPDATE agent_executions SET status = 'halted', updated_at_ms = ?1
             WHERE status = 'running'",
            params![recovery_at_ms],
        )?;
        transaction.execute(
            "UPDATE task_runs SET state='blocked',
                    last_error='OOMU restarted. Resume from the last verified checkpoint.',
                    updated_at_ms=?1,completed_at_ms=NULL,recovery_state='recoverable'
             WHERE runtime_kind='agent' AND state='running' AND runtime_record_id IN (
                SELECT execution_id FROM agent_executions WHERE status='halted'
             )",
            params![recovery_at_ms],
        )?;
        transaction.execute(
            "UPDATE chat_messages
             SET metadata_json = json_set(
                 COALESCE(metadata_json, '{}'), '$.turnState', 'interrupted'
             )
             WHERE role = 'user'
               AND json_extract(metadata_json, '$.turnState') IN ('accepted','permission_waiting')
               AND EXISTS (
                   SELECT 1 FROM chat_turns
                   WHERE chat_turns.turn_id = json_extract(chat_messages.metadata_json, '$.turnId')
                     AND chat_turns.session_id = chat_messages.session_id
                     AND chat_turns.status = 'running'
               )",
            [],
        )?;
        transaction.execute(
            "UPDATE chat_turns
             SET status = 'escalated', completed_at_ms = ?1
             WHERE status IN ('running','completed') AND EXISTS (
                SELECT 1 FROM agent_executions
                WHERE agent_executions.turn_id = chat_turns.turn_id
                  AND agent_executions.generation_token = chat_turns.generation_token
                  AND agent_executions.status = 'halted'
             )",
            params![recovery_at_ms],
        )?;
        transaction.execute(
            "UPDATE chat_turns
             SET status = 'failed', completed_at_ms = ?1
             WHERE status = 'running'",
            params![recovery_at_ms],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn select_state(&self) -> rusqlite::Result<AgenticState> {
        let connection = self.open_connection()?;
        Ok(AgenticState {
            intents: select_intents(&connection)?,
            actions: select_actions(&connection)?,
            certificates: select_certificates(&connection)?,
            plan_generation_states: select_plan_generation_states(&connection)?,
            recoverable_actions: select_recoverable_actions(&connection)?,
        })
    }

    pub fn begin_chat_turn(&self, context: &ChatTurnPersistenceContext) -> rusqlite::Result<()> {
        validate_chat_turn_context_fields(context)?;
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, &context.session_id, &self.workspace_id)?;
        let session_agent_id: String = connection.query_row(
            "SELECT agent_id FROM chat_sessions WHERE id = ?1 AND workspace_id = ?2",
            params![context.session_id, workspace_id],
            |row| row.get(0),
        )?;
        if session_agent_id != context.agent_id {
            return Err(rusqlite::Error::InvalidParameterName(
                "chat turn agent_id does not own the requested session".to_string(),
            ));
        }
        validate_chat_turn_parent(&connection, context)?;
        let now = unix_time_ms();
        connection.execute(
            "
            INSERT INTO chat_turns (
                turn_id, generation_token, workspace_id, session_id, agent_id,
                provider_id, model_id, parent_turn_id, root_turn_id, turn_kind,
                status, created_at_ms, response_claimed_at_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'running', ?11, NULL)
            ",
            params![
                context.turn_id,
                context.generation_token,
                workspace_id,
                context.session_id,
                context.agent_id,
                context.provider_id,
                context.model_id,
                context.parent_turn_id,
                context.root_turn_id,
                context.turn_kind,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn begin_or_validate_running_chat_turn(
        &self,
        context: &ChatTurnPersistenceContext,
    ) -> rusqlite::Result<()> {
        match self.begin_chat_turn(context) {
            Ok(()) => Ok(()),
            Err(insert_error) => self
                .validate_chat_turn_generation(context)
                .map_err(|_| insert_error),
        }
    }

    pub fn validate_chat_turn_generation(
        &self,
        context: &ChatTurnPersistenceContext,
    ) -> rusqlite::Result<()> {
        validate_chat_turn_context_fields(context)?;
        let connection = self.open_connection()?;
        let matches: i64 = connection.query_row(
            "
            SELECT COUNT(*)
            FROM chat_turns turns
            JOIN chat_sessions sessions
              ON sessions.id = turns.session_id
             AND sessions.workspace_id = turns.workspace_id
            WHERE turns.turn_id = ?1
              AND turns.generation_token = ?2
              AND turns.session_id = ?3
              AND turns.agent_id = ?4
              AND turns.provider_id = ?5
              AND turns.model_id = ?6
              AND turns.root_turn_id = ?7
              AND turns.turn_kind = ?8
              AND COALESCE(turns.parent_turn_id, '') = COALESCE(?9, '')
              AND turns.workspace_id = ?10
              AND turns.status = 'running'
              AND sessions.agent_id = turns.agent_id
            ",
            params![
                context.turn_id,
                context.generation_token,
                context.session_id,
                context.agent_id,
                context.provider_id,
                context.model_id,
                context.root_turn_id,
                context.turn_kind,
                context.parent_turn_id,
                self.workspace_id,
            ],
            |row| row.get(0),
        )?;
        if matches != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "chat turn session or generation no longer matches its immutable context"
                    .to_string(),
            ));
        }
        Ok(())
    }

    pub fn validate_accepted_chat_turn_generation(
        &self,
        context: &ChatTurnPersistenceContext,
    ) -> rusqlite::Result<()> {
        validate_chat_turn_context_fields(context)?;
        let connection = self.open_connection()?;
        let matches: i64 = connection.query_row(
            "
            SELECT COUNT(*)
            FROM chat_turns turns
            JOIN chat_sessions sessions
              ON sessions.id = turns.session_id
             AND sessions.workspace_id = turns.workspace_id
            WHERE turns.turn_id = ?1
              AND turns.generation_token = ?2
              AND turns.session_id = ?3
              AND turns.agent_id = ?4
              AND turns.provider_id = ?5
              AND turns.model_id = ?6
              AND turns.root_turn_id = ?7
              AND turns.turn_kind = ?8
              AND COALESCE(turns.parent_turn_id, '') = COALESCE(?9, '')
              AND turns.workspace_id = ?10
              AND turns.status = 'running'
              AND sessions.agent_id = turns.agent_id
              AND EXISTS (
                SELECT 1
                FROM chat_messages messages
                WHERE messages.workspace_id = turns.workspace_id
                  AND messages.session_id = turns.session_id
                  AND messages.role = 'user'
                  AND json_extract(messages.metadata_json, '$.turnId') = turns.turn_id
                  AND json_extract(messages.metadata_json, '$.generationToken') = turns.generation_token
                  AND json_extract(messages.metadata_json, '$.turnState') = 'accepted'
              )
            ",
            params![
                context.turn_id,
                context.generation_token,
                context.session_id,
                context.agent_id,
                context.provider_id,
                context.model_id,
                context.root_turn_id,
                context.turn_kind,
                context.parent_turn_id,
                self.workspace_id,
            ],
            |row| row.get(0),
        )?;
        if matches != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "chat turn does not match one durable accepted user turn".to_string(),
            ));
        }
        Ok(())
    }

    pub fn select_chat_turn_context(
        &self,
        turn_id: &str,
    ) -> rusqlite::Result<Option<ChatTurnPersistenceContext>> {
        let connection = self.open_connection()?;
        connection
            .query_row(
                "
                SELECT turn_id, generation_token, session_id, agent_id, provider_id, model_id,
                       parent_turn_id, root_turn_id, turn_kind
                FROM chat_turns
                WHERE turn_id = ?1
                ",
                params![turn_id.trim()],
                |row| {
                    Ok(ChatTurnPersistenceContext {
                        turn_id: row.get(0)?,
                        generation_token: row.get(1)?,
                        session_id: row.get(2)?,
                        agent_id: row.get(3)?,
                        provider_id: row.get(4)?,
                        model_id: row.get(5)?,
                        parent_turn_id: row.get(6)?,
                        root_turn_id: row.get(7)?,
                        turn_kind: row.get(8)?,
                    })
                },
            )
            .optional()
    }

    pub fn finish_chat_turn(
        &self,
        context: &ChatTurnPersistenceContext,
        status: &str,
    ) -> rusqlite::Result<()> {
        if !matches!(status, "completed" | "failed" | "cancelled" | "escalated") {
            return Err(rusqlite::Error::InvalidParameterName(
                "invalid terminal chat turn status".to_string(),
            ));
        }
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let changed = connection.execute(
            "
            UPDATE chat_turns
            SET status = ?1, completed_at_ms = ?2
            WHERE turn_id = ?3
              AND generation_token = ?4
              AND session_id = ?5
              AND agent_id = ?6
              AND provider_id = ?7
              AND model_id = ?8
              AND root_turn_id = ?9
              AND turn_kind = ?10
              AND COALESCE(parent_turn_id, '') = COALESCE(?11, '')
              AND status = 'running'
            ",
            params![
                status,
                unix_time_ms(),
                context.turn_id,
                context.generation_token,
                context.session_id,
                context.agent_id,
                context.provider_id,
                context.model_id,
                context.root_turn_id,
                context.turn_kind,
                context.parent_turn_id,
            ],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::InvalidParameterName(
                "chat turn completion did not match one running generation".to_string(),
            ));
        }
        connection.execute(
            "UPDATE chat_messages
             SET metadata_json = json_set(COALESCE(metadata_json, '{}'), '$.turnState', ?2)
             WHERE session_id = ?1 AND json_extract(metadata_json, '$.turnId') = ?3",
            params![context.session_id, status, context.turn_id],
        )?;
        Ok(())
    }

    pub fn insert_chat_message(
        &self,
        session_id: &str,
        agent_id: &str,
        role: &str,
        content: &str,
    ) -> rusqlite::Result<i64> {
        self.insert_chat_message_with_metadata(
            session_id, agent_id, role, content, None, None, None,
        )
    }

    pub fn insert_chat_message_with_metadata(
        &self,
        session_id: &str,
        agent_id: &str,
        role: &str,
        content: &str,
        provider_id: Option<&str>,
        model_id: Option<&str>,
        metadata: Option<&Value>,
    ) -> rusqlite::Result<i64> {
        let content = if role.eq_ignore_ascii_case("assistant") {
            let canonical = assistant_content::canonicalize_assistant_content(content);
            if canonical.is_empty() {
                return Err(rusqlite::Error::InvalidParameterName(
                    "assistant chat content is empty after canonicalization".to_string(),
                ));
            }
            canonical
        } else {
            content.to_string()
        };
        let provider_id = provider_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let model_id = model_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let metadata_json = metadata
            .map(serde_json::to_string)
            .transpose()
            .map_err(json_to_sql_error)?;
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)?;
        connection.execute(
            "
            INSERT INTO chat_messages (
                workspace_id, session_id, agent_id, role, content, provider_id, model_id,
                metadata_json, is_compacted, compaction_type, timestamp_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 'raw', ?9)
            ",
            params![
                workspace_id,
                session_id,
                agent_id,
                role,
                content,
                provider_id,
                model_id,
                metadata_json,
                unix_time_ms()
            ],
        )?;
        Ok(connection.last_insert_rowid())
    }

    pub fn get_chat_history(
        &self,
        session_id: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<crate::inference::InferenceMessage>> {
        let connection = self.open_connection()?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)?;
        let mut statement = connection.prepare(
            "
            SELECT role, content, metadata_json
            FROM chat_messages
            WHERE workspace_id = ?1 AND session_id = ?2
              AND COALESCE(is_compacted, 0) = 0
              AND COALESCE(json_extract(metadata_json, '$.uiOnlyCheckpoint'), 0) = 0
            ORDER BY timestamp_ms DESC, id DESC
            LIMIT ?3
            ",
        )?;

        let rows = statement.query_map(params![workspace_id, session_id, limit as i64], |row| {
            Ok(crate::inference::InferenceMessage {
                role: row.get(0)?,
                content: row.get(1)?,
                attachments: crate::inference::public_grounding_attachments_from_metadata(
                    row.get::<_, Option<String>>(2)?.as_deref(),
                ),
            })
        })?;

        let mut messages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut latest_public_grounding_found = false;
        for message in &mut messages {
            if message.attachments.is_empty() {
                continue;
            }
            if latest_public_grounding_found {
                message.attachments.clear();
            } else {
                latest_public_grounding_found = true;
            }
        }
        messages.reverse(); // Order from oldest to newest for inference
        Ok(messages)
    }

    pub fn search_relevant_chat_memory_blocks(
        &self,
        session_id: Option<&str>,
        agent_id: &str,
        query: &str,
        exclude_content: Option<&str>,
        limit: usize,
    ) -> rusqlite::Result<Vec<RelevantChatMemoryBlock>> {
        let limit = limit.min(25);
        if limit == 0 {
            return Ok(Vec::new());
        }

        let terms = keyword_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }

        let connection = self.open_connection()?;
        let workspace_id = match session_id {
            Some(session_id) => {
                workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)?
            }
            None => self.workspace_id.clone(),
        };
        let like_clauses = terms
            .iter()
            .enumerate()
            .map(|(index, _)| format!("LOWER(content) LIKE ?{}", index + 5))
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!(
            "
            SELECT workspace_id, session_id, role, content, timestamp_ms
            FROM chat_messages
            WHERE workspace_id = ?1
              AND agent_id = ?2
              AND (?3 = '' OR session_id = ?3)
              AND (?4 = '' OR content <> ?4)
              AND COALESCE(is_compacted, 0) = 0
              AND ({like_clauses})
            ORDER BY timestamp_ms DESC, id DESC
            LIMIT 200
            "
        );
        let mut sql_params = vec![
            workspace_id,
            agent_id.trim().to_string(),
            session_id.unwrap_or_default().trim().to_string(),
            exclude_content.unwrap_or_default().trim().to_string(),
        ];
        sql_params.extend(terms.iter().map(|term| format!("%{term}%")));

        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(sql_params.iter()), |row| {
            Ok(RelevantChatMemoryBlock {
                workspace_id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                created_at_ms: row.get(4)?,
                score: 0.0,
            })
        })?;

        let mut blocks = Vec::new();
        for row in rows {
            let mut block = row?;
            block.score = chat_memory_relevance(&block, &terms);
            if block.score > 0.0 {
                blocks.push(block);
            }
        }
        blocks.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.created_at_ms.cmp(&left.created_at_ms))
        });
        blocks.truncate(limit);
        Ok(blocks)
    }

    pub fn ensure_chat_session(
        &self,
        request: CreateChatSessionRequest,
    ) -> rusqlite::Result<ChatSessionRecord> {
        let now = unix_time_ms();
        let workspace_id =
            workspace_id_from_request(request.workspace_id.as_deref(), &self.workspace_id)?;
        let id = format!("chat-session-{now}");
        let title = request
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("New Session")
            .to_string();
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            INSERT INTO chat_sessions (
                id, workspace_id, agent_id, title, title_source, provider_id, model_id,
                dynamic_routing_override, created_at_ms, updated_at_ms
            )
            VALUES (?1, ?2, ?3, ?4, 'auto', ?5, ?6, ?7, ?8, ?9)
            ",
            params![
                &id,
                &workspace_id,
                &request.agent_id,
                &title,
                &request.provider_id,
                &request.model_id,
                request.dynamic_routing_override,
                now,
                now
            ],
        )?;
        Ok(ChatSessionRecord {
            id,
            workspace_id,
            project_id: None,
            agent_id: request.agent_id,
            title,
            title_source: "auto".to_string(),
            provider_id: request.provider_id,
            model_id: request.model_id,
            web_grounding_override: None,
            dynamic_routing_override: request.dynamic_routing_override,
            unread_completion: false,
            created_at_ms: now,
            updated_at_ms: now,
        })
    }

    pub fn ensure_chat_session_with_id(
        &self,
        session_id: &str,
        request: CreateChatSessionRequest,
    ) -> rusqlite::Result<ChatSessionRecord> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "chat session id must not be empty.".to_string(),
            ));
        }
        let now = unix_time_ms();
        let workspace_id =
            workspace_id_from_request(request.workspace_id.as_deref(), &self.workspace_id)?;
        let title = request
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("New Session")
            .to_string();
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            INSERT OR IGNORE INTO chat_sessions (
                id, workspace_id, agent_id, title, title_source, provider_id, model_id,
                dynamic_routing_override, created_at_ms, updated_at_ms
            )
            VALUES (?1, ?2, ?3, ?4, 'auto', ?5, ?6, ?7, ?8, ?9)
            ",
            params![
                session_id,
                &workspace_id,
                &request.agent_id,
                &title,
                &request.provider_id,
                &request.model_id,
                request.dynamic_routing_override,
                now,
                now
            ],
        )?;
        drop(connection);
        self.select_chat_session_by_id(session_id)
    }

    pub fn touch_chat_session(
        &self,
        session_id: &str,
        title: Option<&str>,
        provider_id: &str,
        model_id: &str,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)?;
        let now = unix_time_ms();
        if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
            connection.execute(
                "
                UPDATE chat_sessions
                SET title = CASE
                        WHEN title_source = 'user' THEN title
                        ELSE ?1
                    END,
                    title_source = CASE
                        WHEN title_source = 'user' THEN title_source
                        ELSE 'auto'
                    END,
                    provider_id = ?2,
                    model_id = ?3,
                    updated_at_ms = ?4
                WHERE id = ?5 AND workspace_id = ?6
                ",
                params![title, provider_id, model_id, now, session_id, workspace_id],
            )?;
        } else {
            connection.execute(
                "
                UPDATE chat_sessions
                SET provider_id = ?1, model_id = ?2, updated_at_ms = ?3
                WHERE id = ?4 AND workspace_id = ?5
                ",
                params![provider_id, model_id, now, session_id, workspace_id],
            )?;
        }
        Ok(())
    }

    pub fn select_chat_sessions(&self) -> rusqlite::Result<Vec<ChatSessionRecord>> {
        self.purge_expired_recoverable_chat_session_deletions()?;
        let connection = self.open_connection()?;
        let workspace_id = self.workspace_id.as_str();
        let mut statement = connection.prepare(
            "
            SELECT id, workspace_id, project_id, agent_id, title, title_source, provider_id, model_id, web_grounding_override, dynamic_routing_override, unread_completion, created_at_ms, updated_at_ms
            FROM chat_sessions
            WHERE workspace_id = ?1
            ORDER BY updated_at_ms DESC
            ",
        )?;
        let rows = statement.query_map(params![workspace_id], chat_session_from_row)?;
        rows.collect()
    }

    pub fn select_chat_session_by_id(
        &self,
        session_id: &str,
    ) -> rusqlite::Result<ChatSessionRecord> {
        let connection = self.open_connection()?;
        let workspace_id = self.workspace_id.as_str();
        connection.query_row(
            "
            SELECT id, workspace_id, project_id, agent_id, title, title_source, provider_id, model_id, web_grounding_override, dynamic_routing_override, unread_completion, created_at_ms, updated_at_ms
            FROM chat_sessions
            WHERE id = ?1 AND workspace_id = ?2
            ",
            params![session_id, workspace_id],
            chat_session_from_row,
        )
    }

    pub fn set_chat_session_unread_completion(
        &self,
        session_id: &str,
        unread: bool,
    ) -> rusqlite::Result<i64> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "chat session id must not be empty.".to_string(),
            ));
        }
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let changed = connection.execute(
            "
            UPDATE chat_sessions
            SET unread_completion = ?1
            WHERE id = ?2 AND workspace_id = ?3
            ",
            params![unread, session_id, &self.workspace_id],
        )?;
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        connection.query_row(
            "SELECT COUNT(*) FROM chat_sessions WHERE workspace_id = ?1 AND unread_completion = 1",
            params![&self.workspace_id],
            |row| row.get(0),
        )
    }

    pub fn unread_chat_session_count(&self) -> rusqlite::Result<i64> {
        let connection = self.open_connection()?;
        connection.query_row(
            "SELECT COUNT(*) FROM chat_sessions WHERE workspace_id = ?1 AND unread_completion = 1",
            params![&self.workspace_id],
            |row| row.get(0),
        )
    }

    pub fn update_chat_session_web_grounding_override(
        &self,
        session_id: &str,
        web_grounding_override: Option<bool>,
    ) -> rusqlite::Result<ChatSessionRecord> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let workspace_id = self.workspace_id.as_str();
        let now = unix_time_ms();
        let changed = connection.execute(
            "
            UPDATE chat_sessions
            SET web_grounding_override = ?1, updated_at_ms = ?2
            WHERE id = ?3 AND workspace_id = ?4
            ",
            params![web_grounding_override, now, session_id, workspace_id],
        )?;
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        connection.query_row(
            "
            SELECT id, workspace_id, project_id, agent_id, title, title_source, provider_id, model_id, web_grounding_override, dynamic_routing_override, unread_completion, created_at_ms, updated_at_ms
            FROM chat_sessions
            WHERE id = ?1 AND workspace_id = ?2
            ",
            params![session_id, workspace_id],
            chat_session_from_row,
        )
    }

    pub fn rename_chat_session(
        &self,
        session_id: &str,
        title: &str,
    ) -> rusqlite::Result<ChatSessionRecord> {
        let title = title.trim();
        if title.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "title must not be empty".to_string(),
            ));
        }

        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let workspace_id = self.workspace_id.as_str();
        let now = unix_time_ms();
        let changed = connection.execute(
            "
            UPDATE chat_sessions
            SET title = ?1, title_source = 'user', updated_at_ms = ?2
            WHERE id = ?3 AND workspace_id = ?4
            ",
            params![title, now, session_id, workspace_id],
        )?;
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        connection.query_row(
            "
            SELECT id, workspace_id, project_id, agent_id, title, title_source, provider_id, model_id, web_grounding_override, dynamic_routing_override, unread_completion, created_at_ms, updated_at_ms
            FROM chat_sessions
            WHERE id = ?1 AND workspace_id = ?2
            ",
            params![session_id, workspace_id],
            chat_session_from_row,
        )
    }

    fn delete_auto_route_audit_for_session(&self, session_id: &str) -> rusqlite::Result<()> {
        if self.storage_class() != BackingStoreClass::Persistent {
            return Ok(());
        }
        let connection = self.open_ops_connection()?;
        ensure_local_inference_audit_schema(&connection)?;
        connection.execute(
            "DELETE FROM local_inference_audit
             WHERE event_kind = 'dynamic_routing' AND json_valid(metadata_json)
               AND json_extract(metadata_json, '$.sessionId') = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn select_chat_messages(
        &self,
        session_id: &str,
    ) -> rusqlite::Result<Vec<ChatMessageRecord>> {
        let connection = self.open_connection()?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)?;
        select_active_chat_messages_for_session(&connection, &workspace_id, session_id)
    }

    pub fn compact_session_messages(
        &self,
        session_id: &str,
    ) -> rusqlite::Result<SemanticCompactionResponse> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(
                "session_id must not be empty".to_string(),
            ));
        }

        let _guard = self.lock_writes();
        let mut connection = self.open_connection()?;
        let workspace_id =
            workspace_id_for_chat_session(&connection, session_id, &self.workspace_id)?;
        let active_messages =
            select_active_chat_messages_for_session(&connection, &workspace_id, session_id)?;
        let raw_user_indices = active_messages
            .iter()
            .enumerate()
            .filter(|(_, message)| {
                message.compaction_type.as_deref() != Some("summary_anchor")
                    && message.role.eq_ignore_ascii_case("user")
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if raw_user_indices.len() <= RECENT_RAW_CHAT_TURNS_TO_PRESERVE {
            return Ok(SemanticCompactionResponse {
                compacted_messages: 0,
                anchor_message_id: None,
            });
        }

        let preserved_start_index =
            raw_user_indices[raw_user_indices.len() - RECENT_RAW_CHAT_TURNS_TO_PRESERVE];
        let preserved_start = &active_messages[preserved_start_index];
        let source_messages = active_messages[..preserved_start_index]
            .iter()
            .filter(|message| {
                !message.content.trim().is_empty()
                    && !chat_message_is_protected_from_compaction(message)
            })
            .collect::<Vec<_>>();
        let raw_message_count = source_messages
            .iter()
            .filter(|message| message.compaction_type.as_deref() != Some("summary_anchor"))
            .count();
        if raw_message_count == 0 || source_messages.is_empty() {
            return Ok(SemanticCompactionResponse {
                compacted_messages: 0,
                anchor_message_id: None,
            });
        }

        let agent_id = compaction_anchor_agent_id(&connection, &workspace_id, session_id)?
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName(
                    "Cannot compact a session without chat messages.".to_string(),
                )
            })?;
        let summary = session_context_policy::build_extractive_checkpoint(&source_messages);
        let source_evidence = source_messages
            .iter()
            .map(|message| {
                json!({
                    "messageId": message.id,
                    "role": message.role,
                    "contentSha256": sha256_hex(message.content.as_bytes()),
                })
            })
            .collect::<Vec<_>>();
        let metadata_json = serde_json::to_string(&json!({
            "source": "deterministic_extractive_checkpoint",
            "activeMessageCount": active_messages.len(),
            "rawMessageCount": raw_message_count,
            "sourceEvidence": source_evidence,
        }))
        .map_err(json_to_sql_error)?;
        let anchor_timestamp_ms = preserved_start.created_at_ms.saturating_sub(1);
        let transaction = connection.transaction()?;
        let compacted_messages = transaction.execute(
            "
            UPDATE chat_messages
            SET is_compacted = 1,
                compaction_type = COALESCE(compaction_type, 'raw')
            WHERE workspace_id = ?1
              AND session_id = ?2
              AND COALESCE(is_compacted, 0) = 0
              AND COALESCE(json_extract(metadata_json, '$.uiOnlyCheckpoint'), 0) = 0
              AND COALESCE(json_extract(metadata_json, '$.turnState'), 'completed')
                  NOT IN ('accepted', 'interrupted', 'running', 'processing')
              AND (
                    timestamp_ms < ?3
                    OR (timestamp_ms = ?3 AND id < ?4)
                  )
            ",
            params![
                &workspace_id,
                session_id,
                preserved_start.created_at_ms,
                preserved_start.id
            ],
        )?;
        transaction.execute(
            "
            INSERT INTO chat_messages (
                workspace_id, session_id, agent_id, role, content, provider_id, model_id,
                metadata_json, is_compacted, compaction_type, timestamp_ms
            )
            VALUES (?1, ?2, ?3, 'system', ?4, NULL, NULL, ?5, 0, 'summary_anchor', ?6)
            ",
            params![
                &workspace_id,
                session_id,
                agent_id,
                summary,
                metadata_json,
                anchor_timestamp_ms
            ],
        )?;
        let anchor_message_id = transaction.last_insert_rowid();
        transaction.commit()?;

        Ok(SemanticCompactionResponse {
            compacted_messages,
            anchor_message_id: Some(anchor_message_id),
        })
    }

    pub fn sovereign_ledger_stats(
        &self,
        since_ms: Option<i64>,
    ) -> rusqlite::Result<SovereignLedgerStats> {
        let since_ms = effective_ledger_since_ms(since_ms, self.sovereign_ledger_reset_at_ms()?);
        let chat_counts = self.select_chat_ledger_counts(since_ms)?;
        let local_turns = chat_counts.local_turns;
        let cloud_turns = chat_counts.cloud_turns;
        let protected_input_tokens = chat_counts.protected_input_tokens;
        let protected_output_tokens = chat_counts.protected_output_tokens;

        let total_turns = local_turns.saturating_add(cloud_turns);
        let ratio_on_device = if total_turns == 0 {
            0.0
        } else {
            (local_turns as f64 / total_turns as f64) * 100.0
        };
        let estimated_api_savings =
            ledger_estimated_api_savings(protected_input_tokens, protected_output_tokens);
        let data_egress_protected_mb = ledger_protected_megabytes(
            protected_input_tokens.saturating_add(protected_output_tokens),
        );

        Ok(SovereignLedgerStats {
            total_local_turns: local_turns,
            total_cloud_turns: cloud_turns,
            ratio_on_device,
            estimated_api_savings,
            data_egress_protected_mb,
            protected_input_tokens,
            protected_output_tokens,
        })
    }

    pub fn reset_sovereign_ledger_stats(&self) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let now_ms = unix_time_ms();
        let connection = self.open_connection()?;
        connection.execute(
            "
            INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at_ms = excluded.updated_at_ms,
                encryption_state = excluded.encryption_state
            ",
            params![
                LEDGER_RESET_AT_KEY,
                now_ms.to_string(),
                now_ms,
                get_current_encryption_state()
            ],
        )?;

        let ops_connection = self.open_ops_connection()?;
        ensure_local_inference_audit_schema(&ops_connection)?;
        ops_connection.execute("DELETE FROM local_inference_audit", [])?;
        Ok(())
    }

    fn sovereign_ledger_reset_at_ms(&self) -> rusqlite::Result<Option<i64>> {
        self.select_app_preference(LEDGER_RESET_AT_KEY)
            .map(|value| value.and_then(|value| value.parse::<i64>().ok()))
    }

    fn select_chat_ledger_counts(
        &self,
        since_ms: Option<i64>,
    ) -> rusqlite::Result<SovereignLedgerChatCounts> {
        let connection = self.open_connection()?;
        let (sql, params) = if let Some(since_ms) = since_ms {
            (
                "
                SELECT provider_id, model_id, metadata_json, content
                FROM chat_messages
                WHERE role = 'assistant'
                  AND timestamp_ms >= ?1
                ",
                vec![since_ms],
            )
        } else {
            (
                "
                SELECT provider_id, model_id, metadata_json, content
                FROM chat_messages
                WHERE role = 'assistant'
                ",
                Vec::new(),
            )
        };
        let mut statement = connection.prepare(sql)?;
        let rows = statement.query_map(params_from_iter(params), |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;

        let mut counts = SovereignLedgerChatCounts::default();
        for row in rows {
            let (provider_id, model_id, metadata_json, content) = row?;
            let metadata = metadata_json
                .as_deref()
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                .unwrap_or(Value::Null);
            let route_is_local = ledger_metadata_route_is_local(&metadata)
                || ledger_route_is_local(provider_id.as_deref(), model_id.as_deref());
            if route_is_local {
                counts.local_turns = counts.local_turns.saturating_add(1);
                let tokens = ledger_token_estimate_from_value(&metadata);
                let output_tokens = if tokens.output_tokens > 0 {
                    tokens.output_tokens
                } else {
                    estimate_ledger_tokens(&content)
                };
                counts.protected_input_tokens = counts
                    .protected_input_tokens
                    .saturating_add(tokens.input_tokens);
                counts.protected_output_tokens =
                    counts.protected_output_tokens.saturating_add(output_tokens);
            } else if metadata != Value::Null || provider_id.is_some() || model_id.is_some() {
                counts.cloud_turns = counts.cloud_turns.saturating_add(1);
            }
        }
        Ok(counts)
    }

    pub(crate) fn open_connection(&self) -> rusqlite::Result<Connection> {
        let key = get_database_key().map_err(database_key_error)?;
        self.open_connection_with_key(&key)
    }

    fn open_connection_with_key(&self, key: &str) -> rusqlite::Result<Connection> {
        let path = self
            .db_path
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let connection = open_ready_db(&path, key)?;
        if self.storage_class() != BackingStoreClass::Persistent {
            enforce_private_sqlite_files(&path)?;
        }
        Ok(connection)
    }

    fn open_ops_connection(&self) -> rusqlite::Result<Connection> {
        self.require_durable_store("operations and audit database access")
            .map_err(database_key_error)?;
        let key = get_database_key().map_err(database_key_error)?;
        self.open_ops_connection_with_key(&key)
    }

    fn open_ops_connection_with_key(&self, key: &str) -> rusqlite::Result<Connection> {
        let path = self.ops_db_path();
        let connection = open_ready_db(&path, key)?;
        if self.storage_class() != BackingStoreClass::Persistent {
            enforce_private_sqlite_files(&path)?;
        }
        Ok(connection)
    }

    fn ops_db_path(&self) -> PathBuf {
        self.db_path
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .parent()
            .map(|parent| parent.join(OPS_DB_FILE))
            .unwrap_or_else(|| project_root().join(OPS_DB_FILE))
    }

    pub(crate) fn lock_writes(&self) -> std::sync::MutexGuard<'_, ()> {
        self.write_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub fn purge_transient_sqlite_cache() -> Result<(), String> {
    let db_path = project_root().join(DB_FILE);
    if !db_path.exists() {
        return Ok(());
    }

    let database_key = get_database_key()?;
    let connection = open_sqlcipher_database_connection_with_key(&db_path, &database_key)
        .map_err(|error| error.to_string())?;
    purge_transient_sqlite_cache_on_connection(&connection).map_err(|error| error.to_string())?;
    Ok(())
}

pub fn sanitize_release_database_at(
    db_path: &Path,
) -> Result<ReleaseDatabaseSanitationReport, String> {
    if !db_path.exists() {
        return Err(format!(
            "Release database artifact was not found at {}.",
            db_path.display()
        ));
    }

    let database_key = get_database_key()?;
    let connection = open_sqlcipher_database_connection_with_key(db_path, &database_key)
        .map_err(|error| error.to_string())?;
    let purged_tables =
        sanitize_release_database_on_connection(&connection).map_err(|error| error.to_string())?;

    Ok(ReleaseDatabaseSanitationReport {
        path: db_path.to_path_buf(),
        purged_tables,
    })
}

pub fn get_app_support_dir() -> Result<PathBuf, DatabaseError> {
    Ok(settings::app_data_root())
}

pub fn get_mod_db_path(mod_id: &str) -> Result<PathBuf, DatabaseError> {
    let namespace = mod_database_namespace(mod_id)?;
    Ok(get_app_support_dir()?
        .join("mods")
        .join(namespace)
        .join("knowledge")
        .join(MOD_VECTOR_DB_FILE))
}

pub fn get_mod_db_connection(mod_id: &str) -> Result<Connection, DatabaseError> {
    let db_path = get_mod_db_path(mod_id)?;
    open_read_only_mod_db_connection(&db_path)
}

fn open_read_only_mod_db_connection(db_path: &Path) -> Result<Connection, DatabaseError> {
    if !db_path.exists() {
        return Err(DatabaseError::NotFound(db_path.display().to_string()));
    }
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| DatabaseError::ConnectionFailed(error.to_string()))?;
    conn.busy_timeout(SQLITE_BUSY_TIMEOUT)
        .map_err(|error| DatabaseError::ConnectionFailed(error.to_string()))?;
    let _ = conn.pragma_update(None, "query_only", true);
    let _ = conn.execute("PRAGMA journal_mode=WAL;", []);
    Ok(conn)
}

fn mod_database_namespace(mod_id: &str) -> Result<String, DatabaseError> {
    let trimmed = mod_id.trim();
    if trimmed.is_empty() {
        return Err(DatabaseError::InvalidModId(
            "mod id must not be empty".to_string(),
        ));
    }
    if trimmed.len() > 256 {
        return Err(DatabaseError::InvalidModId(
            "mod id must be 256 characters or fewer".to_string(),
        ));
    }
    if trimmed == "." || trimmed == ".." {
        return Err(DatabaseError::InvalidModId(format!(
            "reserved path segment '{trimmed}'"
        )));
    }
    if !trimmed
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_'))
    {
        return Err(DatabaseError::InvalidModId(format!(
            "mod id contains unsupported path characters: {trimmed}"
        )));
    }
    Ok(trimmed.to_string())
}

fn canonicalize_trust_directory(path: &str) -> rusqlite::Result<PathBuf> {
    let expanded = expand_home_path(path.trim());
    if expanded.as_os_str().is_empty() {
        return Err(database_key_error(
            "Sovereign trust directory path cannot be empty.".to_string(),
        ));
    }
    let canonical = fs::canonicalize(&expanded).map_err(io_to_sql_error)?;
    if !canonical.is_dir() {
        return Err(database_key_error(format!(
            "Sovereign trust directory must be a folder: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn canonicalize_trust_target_path(path: &Path) -> rusqlite::Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path).map_err(io_to_sql_error);
    }

    let mut missing_components = Vec::new();
    let mut cursor = path;
    loop {
        let Some(parent) = cursor.parent() else {
            return Err(database_key_error(format!(
                "Sovereign trust target has no canonical parent: {}",
                path.display()
            )));
        };
        if parent.exists() {
            let mut canonical = fs::canonicalize(parent).map_err(io_to_sql_error)?;
            for component in missing_components.iter().rev() {
                canonical.push(component);
            }
            if let Some(file_name) = path.file_name() {
                canonical.push(file_name);
            }
            return Ok(canonical);
        }
        if let Some(file_name) = parent.file_name() {
            missing_components.push(file_name.to_os_string());
        }
        cursor = parent;
    }
}

fn expand_home_path(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn trust_categories_json(categories: &[SovereignTrustToolCategory]) -> rusqlite::Result<String> {
    if categories.is_empty() {
        return Err(database_key_error(
            "Sovereign trust policy requires at least one tool category.".to_string(),
        ));
    }
    let mut values = Vec::new();
    for category in categories {
        let value = category.as_str();
        if !values.contains(&value) {
            values.push(value);
        }
    }
    serde_json::to_string(&values).map_err(json_to_sql_error)
}

fn parse_trust_categories(values: &[String]) -> Result<Vec<SovereignTrustToolCategory>, String> {
    if values.is_empty() {
        return Err("Sovereign trust requires at least one tool category.".to_string());
    }
    let mut categories = Vec::new();
    for value in values {
        let category = SovereignTrustToolCategory::from_request(value)?;
        if !categories.contains(&category) {
            categories.push(category);
        }
    }
    Ok(categories)
}

fn trust_grant_matches(
    grant: &SovereignTrustGrant,
    target_path: &Path,
    tool_category: SovereignTrustToolCategory,
) -> rusqlite::Result<bool> {
    if !trust_categories_allow(&grant.allowed_tool_categories, tool_category)? {
        return Ok(false);
    }
    let scope = Path::new(&grant.canonical_directory_path);
    Ok(target_path == scope || target_path.starts_with(scope))
}

fn trust_categories_allow(
    categories_json: &str,
    tool_category: SovereignTrustToolCategory,
) -> rusqlite::Result<bool> {
    let categories: Vec<String> =
        serde_json::from_str(categories_json).map_err(json_from_sql_error)?;
    Ok(categories
        .iter()
        .any(|category| category == tool_category.as_str()))
}

fn policy_trust_grant_from_row(row: &Row<'_>) -> rusqlite::Result<SovereignTrustGrant> {
    Ok(SovereignTrustGrant {
        source: SovereignTrustGrantSource::Policy(row.get(0)?),
        directory_path: row.get(1)?,
        canonical_directory_path: row.get(2)?,
        allowed_tool_categories: row.get(3)?,
        permission_level: SovereignTrustPermissionLevel::parse(&row.get::<_, String>(4)?)?,
        expires_at_ms: row.get(5)?,
        daily_token_cost_limit: row.get(6)?,
        daily_cpu_seconds_limit: row.get(7)?,
        token_cost_used_today: row.get(8)?,
        cpu_seconds_used_today: row.get(9)?,
        usage_day: row.get(10)?,
    })
}

fn session_trust_grant_from_row(row: &Row<'_>) -> rusqlite::Result<SovereignTrustGrant> {
    Ok(SovereignTrustGrant {
        source: SovereignTrustGrantSource::Session(row.get(0)?),
        directory_path: row.get(1)?,
        canonical_directory_path: row.get(2)?,
        allowed_tool_categories: row.get(3)?,
        permission_level: SovereignTrustPermissionLevel::parse(&row.get::<_, String>(4)?)?,
        expires_at_ms: row.get(5)?,
        daily_token_cost_limit: row.get(6)?,
        daily_cpu_seconds_limit: row.get(7)?,
        token_cost_used_today: row.get(8)?,
        cpu_seconds_used_today: row.get(9)?,
        usage_day: row.get(10)?,
    })
}

// MIGRATION_IMPL_BEGIN:0001_seed_channel_configs
fn seed_channel_configs(connection: &Connection, enc_state: &str) -> rusqlite::Result<()> {
    let now = unix_time_ms();
    for platform in ["signal", "whatsapp", "telegram", "discord"] {
        connection.execute(
            "
            INSERT INTO channel_configs (
                platform,
                is_active,
                credentials_json,
                owner_id,
                updated_at_ms,
                encryption_state
            )
            VALUES (?1, 0, '{}', '', ?2, ?3)
            ON CONFLICT(platform) DO NOTHING
            ",
            params![platform, now, enc_state],
        )?;
    }
    Ok(())
}
// MIGRATION_IMPL_END:0001_seed_channel_configs

pub(crate) fn normalize_channel_platform(platform: &str) -> rusqlite::Result<String> {
    let normalized = platform.trim().replace('-', "_").to_ascii_lowercase();
    if COMMUNITY_CHANNEL_PLATFORMS.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err(database_key_error(format!(
            "Unsupported channel platform: {platform}"
        )))
    }
}

fn channel_platform_label(platform: &str) -> String {
    match platform {
        "telegram" => "Telegram",
        "discord" => "Discord",
        "slack" => "Slack",
        _ => platform,
    }
    .to_string()
}

fn channel_credential_marker(platform: &str) -> String {
    json!({ "credentialRef": format!("keychain://channel/{platform}") }).to_string()
}

fn is_channel_credential_marker(value: &str) -> bool {
    serde_json::from_str::<Value>(value)
        .ok()
        .and_then(|value| value.get("credentialRef").cloned())
        .and_then(|value| value.as_str().map(str::to_string))
        .is_some_and(|reference| reference.starts_with("keychain://channel/"))
}

fn hydrate_channel_credentials(
    connection: &Connection,
    mut configs: Vec<ChannelConfigRecord>,
) -> rusqlite::Result<Vec<ChannelConfigRecord>> {
    for config in &mut configs {
        if is_channel_credential_marker(&config.credentials_json) {
            config.credentials_json = crate::secret_store::get_channel_secrets(&config.platform)
                .map_err(database_key_error)?
                .ok_or_else(|| {
                    database_key_error("channel_credential_reference_unavailable".to_string())
                })?;
            continue;
        }
        if config.credentials_json.trim() == "{}" {
            continue;
        }

        // One-time fail-closed migration from legacy SQLCipher rows. The
        // plaintext column is replaced only after Keychain confirms the write.
        crate::secret_store::set_channel_secrets(&config.platform, &config.credentials_json)
            .map_err(database_key_error)?;
        connection.execute(
            "UPDATE channel_configs SET credentials_json = ?2 WHERE platform = ?1",
            params![
                config.platform.as_str(),
                channel_credential_marker(&config.platform)
            ],
        )?;
    }
    Ok(configs)
}

fn channel_config_from_row(row: &Row<'_>) -> rusqlite::Result<ChannelConfigRecord> {
    let platform: String = row.get(0)?;
    let owner_id: String = row.get(3)?;
    Ok(ChannelConfigRecord {
        label: channel_platform_label(&platform),
        platform,
        is_active: row.get::<_, i64>(1)? == 1,
        credentials_json: row.get(2)?,
        owner_id: if owner_id.trim().is_empty() {
            None
        } else {
            Some(owner_id)
        },
        updated_at_ms: row.get(4)?,
    })
}

fn sovereign_trust_policy_record_from_row(
    row: &Row<'_>,
    now_ms: i64,
) -> rusqlite::Result<SovereignTrustPolicyRecord> {
    let categories_json: String = row.get(3)?;
    let expires_at_ms: Option<i64> = row.get(5)?;
    Ok(SovereignTrustPolicyRecord {
        id: row.get(0)?,
        directory_path: row.get(1)?,
        canonical_directory_path: row.get(2)?,
        allowed_tool_categories: serde_json::from_str(&categories_json)
            .map_err(json_from_sql_error)?,
        permission_level: row.get(4)?,
        expires_at_ms,
        daily_token_cost_limit: row.get(6)?,
        daily_cpu_seconds_limit: row.get(7)?,
        estimated_token_cost_reserved_today: row.get(8)?,
        cpu_seconds_reserved_today: row.get(9)?,
        usage_day: row.get(10)?,
        created_at_ms: row.get(11)?,
        updated_at_ms: row.get(12)?,
        last_used_at_ms: row.get(13)?,
        active: expires_at_ms
            .map(|expires| expires > now_ms)
            .unwrap_or(true),
    })
}

fn sovereign_trust_session_record_from_row(
    row: &Row<'_>,
    now_ms: i64,
) -> rusqlite::Result<SovereignTrustSessionRecord> {
    let categories_json: String = row.get(5)?;
    let expires_at_ms: i64 = row.get(7)?;
    Ok(SovereignTrustSessionRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        policy_id: row.get(2)?,
        directory_path: row.get(3)?,
        canonical_directory_path: row.get(4)?,
        allowed_tool_categories: serde_json::from_str(&categories_json)
            .map_err(json_from_sql_error)?,
        permission_level: row.get(6)?,
        expires_at_ms,
        daily_token_cost_limit: row.get(8)?,
        daily_cpu_seconds_limit: row.get(9)?,
        estimated_token_cost_reserved_today: row.get(10)?,
        cpu_seconds_reserved_today: row.get(11)?,
        usage_day: row.get(12)?,
        created_at_ms: row.get(13)?,
        last_used_at_ms: row.get(14)?,
        active: expires_at_ms > now_ms,
    })
}

fn sovereign_trust_audit_event_from_row(
    row: &Row<'_>,
) -> rusqlite::Result<SovereignTrustAuditEvent> {
    let id: i64 = row.get(0)?;
    let plan_id: String = row.get(1)?;
    let operation: String = row.get(2)?;
    let input: String = row.get(3)?;
    let output: Option<String> = row.get(4)?;
    let status: String = row.get(5)?;
    let created_at_ms: i64 = row.get(6)?;
    let output_value = output
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok());
    let claims = output_value
        .as_ref()
        .and_then(|value| value.get("claims"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let auto_claim = claims
        .iter()
        .find(|claim| claim.contains("sovereign_trust_auto_approved"));
    let trust_tier = auto_claim
        .and_then(|claim| claim_field(claim, "tier"))
        .map(ToString::to_string);
    let authorization_mode = if let Some(tier) = trust_tier.as_deref() {
        match tier {
            "global_trust" => "global_trust_auto",
            "session_gated" => "session_gated_auto",
            _ => "trusted_auto",
        }
    } else if claims
        .iter()
        .any(|claim| claim.contains("shield_gate_approved"))
    {
        "manual_popup"
    } else {
        "recorded"
    }
    .to_string();
    let summary = output_value
        .as_ref()
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .map(|value| value.lines().next().unwrap_or(value).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("{operation} {status}"));

    Ok(SovereignTrustAuditEvent {
        id,
        plan_id,
        operation,
        input_kind: trust_audit_input_kind(&input),
        target_path: trust_audit_target_path(&input, &claims),
        status,
        authorization_mode,
        trust_tier,
        execution_hash: sha256_hex(output.as_deref().unwrap_or(&input).as_bytes()),
        summary,
        claims,
        created_at_ms,
    })
}

fn trust_audit_input_kind(input: &str) -> Option<String> {
    serde_json::from_str::<Value>(input).ok().and_then(|value| {
        value
            .get("kind")
            .and_then(Value::as_str)
            .map(ToString::to_string)
    })
}

fn trust_audit_target_path(input: &str, claims: &[String]) -> Option<String> {
    serde_json::from_str::<Value>(input)
        .ok()
        .and_then(|value| {
            value
                .get("path")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .or_else(|| {
            claims
                .iter()
                .find_map(|claim| claim_field(claim, "path").map(ToString::to_string))
        })
}

fn claim_field<'a>(claim: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    claim
        .split_whitespace()
        .find_map(|field| field.strip_prefix(&prefix))
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

pub(crate) fn trust_usage_day(now_ms: i64) -> i64 {
    now_ms.max(0) / (24 * 60 * 60 * 1000)
}

fn configure_incremental_auto_vacuum(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
    let auto_vacuum_mode: i64 =
        connection.query_row("PRAGMA auto_vacuum;", [], |row| row.get(0))?;
    if auto_vacuum_mode == 0 {
        connection.execute_batch(
            "
            PRAGMA auto_vacuum = INCREMENTAL;
            VACUUM;
            ",
        )?;
    }
    Ok(())
}

fn run_sqlite_maintenance_on_connection(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        PRAGMA incremental_vacuum(100);
        ANALYZE;
        ",
    )
}

pub(crate) fn open_ops_database_connection(path: &Path) -> rusqlite::Result<Connection> {
    let key = get_database_key().map_err(database_key_error)?;
    open_sqlcipher_database_connection_with_key(path, &key)
}

pub(crate) fn open_state_database_connection(path: &Path) -> rusqlite::Result<Connection> {
    let key = get_database_key().map_err(database_key_error)?;
    open_sqlcipher_database_connection_with_key(path, &key)
}

pub fn close_all_sqlcipher_sessions(_app_handle: &tauri::AppHandle) {
    clear_cached_database_key();
    eprintln!("SQLCIPHER_SHUTDOWN_CLEARED_VOLATILE_KEY_CACHE");
}

fn open_ops_database_connection_with_key(path: &Path, key: &str) -> rusqlite::Result<Connection> {
    open_sqlcipher_database_connection_with_key(path, key)
}

fn open_sqlcipher_database_connection_with_key(
    path: &Path,
    key: &str,
) -> rusqlite::Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    }
    ensure_sqlcipher_encrypted_database(path, key)?;
    open_ready_db(path, key)
}

fn open_ready_db(path: &Path, key: &str) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    configure_sqlite_connection(&conn)?;
    conn.pragma_update(None, "key", key)?;
    Ok(conn)
}

fn ensure_local_inference_audit_schema(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS local_inference_audit (
            event_id TEXT PRIMARY KEY,
            event_kind TEXT NOT NULL,
            prompt_hash TEXT NOT NULL,
            output_hash TEXT NOT NULL,
            trace_hash TEXT NOT NULL,
            metadata_json TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_local_inference_audit_created
            ON local_inference_audit(created_at_ms);
        CREATE INDEX IF NOT EXISTS idx_local_inference_audit_prompt
            ON local_inference_audit(prompt_hash);
        CREATE INDEX IF NOT EXISTS idx_local_inference_audit_kind
            ON local_inference_audit(event_kind, created_at_ms);
        ",
    )
}

fn configure_sqlite_connection(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
    Ok(())
}

fn ensure_sqlcipher_encrypted_database(path: &Path, key: &str) -> rusqlite::Result<()> {
    if !path.exists() || is_empty_file(path)? {
        return Ok(());
    }

    if !has_plaintext_sqlite_header(path) {
        return match sqlcipher_key_can_read_database(path, key) {
            Ok(()) => Ok(()),
            Err(current_error) => match migrate_legacy_sqlcipher_database(path, key) {
                Ok(true) => Ok(()),
                Ok(false) => Err(current_error),
                Err(error) => Err(error),
            },
        };
    }

    export_sqlcipher_database(path, None, key)?;
    eprintln!("SQLCIPHER_PLAINTEXT_DATABASE_MIGRATED database=application");
    Ok(())
}

fn migrate_legacy_sqlcipher_database(path: &Path, current_key: &str) -> rusqlite::Result<bool> {
    let mut legacy_key = get_legacy_database_key_for_migration().map_err(database_key_error)?;
    let legacy_opened = sqlcipher_key_can_read_database(path, &legacy_key).is_ok();
    if legacy_opened {
        export_sqlcipher_database(path, Some(&legacy_key), current_key)?;
        eprintln!("SQLCIPHER_LEGACY_DATABASE_REKEYED database=application");
    }
    legacy_key.zeroize();
    Ok(legacy_opened)
}

fn export_sqlcipher_database(
    path: &Path,
    source_key: Option<&str>,
    target_key: &str,
) -> rusqlite::Result<()> {
    let encrypted_path = path.with_extension("db.sqlcipher_tmp");
    let backup_path = path.with_extension("db.plaintext_backup");
    let _ = fs::remove_file(&encrypted_path);
    let _ = fs::remove_file(&backup_path);

    let source = Connection::open(path)?;
    configure_sqlite_connection(&source)?;
    if let Some(key) = source_key {
        source.pragma_update(None, "key", key)?;
    }
    let encrypted_path_string = encrypted_path.to_string_lossy().to_string();
    source.execute_batch("PRAGMA wal_checkpoint(FULL);")?;
    source.execute(
        "ATTACH DATABASE ?1 AS encrypted KEY ?2",
        params![encrypted_path_string, target_key],
    )?;
    source.execute_batch(
        "
        SELECT sqlcipher_export('encrypted');
        DETACH DATABASE encrypted;
        ",
    )?;
    drop(source);

    fs::rename(path, &backup_path).map_err(io_to_sql_error)?;
    fs::rename(&encrypted_path, path).map_err(io_to_sql_error)?;
    fs::remove_file(&backup_path).map_err(io_to_sql_error)?;
    remove_sqlite_sidecars(path);
    Ok(())
}

fn sqlcipher_key_can_read_database(path: &Path, key: &str) -> rusqlite::Result<()> {
    let conn = Connection::open(path)?;
    configure_sqlite_connection(&conn)?;
    conn.pragma_update(None, "key", key)?;
    let _: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))?;
    Ok(())
}

fn is_empty_file(path: &Path) -> rusqlite::Result<bool> {
    fs::metadata(path)
        .map(|metadata| metadata.len() == 0)
        .map_err(io_to_sql_error)
}

fn has_plaintext_sqlite_header(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 16];
    file.read_exact(&mut header).is_ok() && &header == b"SQLite format 3\0"
}

fn remove_sqlite_sidecars(path: &Path) {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    for suffix in ["-wal", "-shm"] {
        let sidecar = path.with_file_name(format!("{file_name}{suffix}"));
        let _ = fs::remove_file(sidecar);
    }
}

fn dump_recent_chat_sessions(connection: &Connection) -> rusqlite::Result<()> {
    println!("\n--- Recent Chat Sessions (10) ---");
    if !table_exists(connection, "chat_sessions")? {
        println!("chat_sessions table not found.");
        return Ok(());
    }

    let web_grounding_expr =
        if column_exists(connection, "chat_sessions", "web_grounding_override")? {
            "web_grounding_override"
        } else {
            "NULL"
        };
    let dynamic_routing_expr =
        if column_exists(connection, "chat_sessions", "dynamic_routing_override")? {
            "dynamic_routing_override"
        } else {
            "NULL"
        };
    let sql = format!(
        "
        SELECT id, agent_id, title, provider_id, model_id,
               {web_grounding_expr}, {dynamic_routing_expr}, updated_at_ms
        FROM chat_sessions
        ORDER BY updated_at_ms DESC
        LIMIT 10
        "
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, Option<i64>>(6)?,
            row.get::<_, i64>(7)?,
        ))
    })?;
    let mut count = 0;
    for row in rows {
        let (
            id,
            agent_id,
            title,
            provider_id,
            model_id,
            web_grounding_override,
            dynamic_routing_override,
            updated_at_ms,
        ) = row?;
        count += 1;
        println!(
            "#{count} id={} agent={} title={} route={}/{} web_grounding={} dynamic_routing={} updated_at_ms={}",
            id,
            agent_id,
            terminal_preview(&title, 80),
            provider_id,
            model_id,
            terminal_optional_bool(web_grounding_override),
            terminal_optional_bool(dynamic_routing_override),
            updated_at_ms
        );
    }
    if count == 0 {
        println!("No chat sessions found.");
    }
    Ok(())
}

fn dump_active_configurations(connection: &Connection) -> rusqlite::Result<()> {
    println!("\n--- Active Configurations ---");
    if table_exists(connection, "active_session_configs")? {
        let mut statement = connection.prepare(
            "
            SELECT session_id, reasoning_depth, context_budget,
                   COALESCE(provider_id, ''), COALESCE(model_id, ''), updated_at
            FROM active_session_configs
            ORDER BY updated_at DESC
            LIMIT 10
            ",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        let mut count = 0;
        for row in rows {
            let (session_id, reasoning_depth, context_budget, provider_id, model_id, updated_at) =
                row?;
            count += 1;
            println!(
                "session_config session={} reasoning={} context_budget={} route={}/{} updated_at={}",
                session_id,
                reasoning_depth,
                context_budget,
                terminal_empty_placeholder(&provider_id),
                terminal_empty_placeholder(&model_id),
                updated_at
            );
        }
        if count == 0 {
            println!("No active session configs found.");
        }
    } else {
        println!("active_session_configs table not found.");
    }

    if table_exists(connection, "routing_preferences")? {
        let mut statement = connection.prepare(
            "
            SELECT key, value, updated_at
            FROM routing_preferences
            WHERE key IN (
                'primary', 'fallback', 'oomu-primary-route', 'oomu-fallback-route',
                'sqlite_maintenance.last_run_ms'
            )
            ORDER BY key COLLATE NOCASE
            ",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (key, value, updated_at) = row?;
            println!(
                "routing_preference key={} value={} updated_at={}",
                key,
                terminal_preview(&value, 120),
                updated_at
            );
        }
    }

    if table_exists(connection, "user_routing_preferences")? {
        let mut statement = connection.prepare(
            "
            SELECT key, COALESCE(primary_route_id, ''), COALESCE(fallback_route_id, ''), updated_at
            FROM user_routing_preferences
            ORDER BY key COLLATE NOCASE
            LIMIT 20
            ",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (key, primary_route_id, fallback_route_id, updated_at) = row?;
            println!(
                "user_routing key={} primary={} fallback={} updated_at={}",
                key,
                terminal_empty_placeholder(&primary_route_id),
                terminal_empty_placeholder(&fallback_route_id),
                updated_at
            );
        }
    }
    Ok(())
}

fn dump_installed_mods(connection: &Connection) -> rusqlite::Result<()> {
    println!("\n--- Installed Mods ---");
    if !table_exists(connection, "installed_mods")? {
        println!("installed_mods table not found.");
        return Ok(());
    }

    let mut statement = connection.prepare(
        "
        SELECT id, name, is_active, version, author, category,
               COALESCE(installed_path, ''), COALESCE(entrypoint, '')
        FROM installed_mods
        ORDER BY is_active DESC, name COLLATE NOCASE
        ",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
        ))
    })?;
    let mut count = 0;
    for row in rows {
        let (id, name, is_active, version, author, category, installed_path, entrypoint) = row?;
        count += 1;
        println!(
            "#{count} id={} name={} active={} version={} author={} category={} path={} entrypoint={}",
            id,
            terminal_preview(&name, 80),
            is_active == 1,
            version,
            terminal_preview(&author, 80),
            terminal_preview(&category, 80),
            terminal_preview(&installed_path, 120),
            terminal_preview(&entrypoint, 80)
        );
    }
    if count == 0 {
        println!("No installed mods found.");
    }
    Ok(())
}
pub(crate) fn purge_transient_sqlite_cache_on_connection(
    connection: &Connection,
) -> rusqlite::Result<()> {
    connection.execute_batch("PRAGMA foreign_keys = OFF;")?;
    purge_table_if_exists(connection, "message_queue", "DELETE FROM message_queue")?;
    purge_table_if_exists(
        connection,
        "agent_execution_logs",
        "DELETE FROM agent_execution_logs",
    )?;
    purge_table_if_exists(
        connection,
        "plan_generation_states",
        "DELETE FROM plan_generation_states WHERE status IN ('queued', 'running', 'failed')",
    )?;
    purge_table_if_exists(
        connection,
        "actions",
        "UPDATE actions SET status = 'recoverable' WHERE status = 'running'",
    )?;
    drop_transient_user_tables(connection)?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(())
}
fn sanitize_release_database_on_connection(
    connection: &Connection,
) -> rusqlite::Result<Vec<ReleaseDatabaseTablePurge>> {
    let mut purged_tables = Vec::new();
    connection.execute_batch("PRAGMA foreign_keys = OFF;")?;
    for table in [
        "chat_messages",
        "chat_sessions",
        "agent_execution_logs",
        "plan_generation_states",
        "intents",
        "actions",
        "certificates",
    ] {
        if table_exists(connection, table)? {
            let rows_deleted = connection.execute(&format!("DELETE FROM {table}"), [])?;
            purged_tables.push(ReleaseDatabaseTablePurge {
                table: table.to_string(),
                rows_deleted,
            });
        }
    }
    connection.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        PRAGMA wal_checkpoint(FULL);
        VACUUM;
        PRAGMA optimize;
        ",
    )?;
    Ok(purged_tables)
}

fn purge_table_if_exists(
    connection: &Connection,
    table: &str,
    sql: &str,
) -> rusqlite::Result<usize> {
    if table_exists(connection, table)? {
        connection.execute(sql, [])
    } else {
        Ok(0)
    }
}

fn drop_transient_user_tables(connection: &Connection) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(
        "
        SELECT name
        FROM sqlite_master
        WHERE type='table'
          AND name NOT LIKE 'sqlite_%'
          AND (
            name LIKE 'tmp_%'
            OR name LIKE 'temp_%'
            OR name LIKE 'cache_%'
            OR name LIKE 'transient_%'
          )
        ",
    )?;
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for table in tables {
        connection.execute(
            &format!("DROP TABLE IF EXISTS {}", quote_identifier(&table)),
            [],
        )?;
    }
    Ok(())
}

fn initialize_migration_ledger(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_migration_ledger (
            sequence INTEGER NOT NULL UNIQUE CHECK(sequence > 0),
            migration_id TEXT PRIMARY KEY CHECK(length(trim(migration_id)) > 0),
            checksum_sha256 TEXT NOT NULL CHECK(length(checksum_sha256) = 64),
            state TEXT NOT NULL CHECK(state IN ('running', 'completed')),
            application_version TEXT NOT NULL,
            started_at_ms INTEGER NOT NULL CHECK(started_at_ms >= 0),
            completed_at_ms INTEGER CHECK(
                (state = 'running' AND completed_at_ms IS NULL)
                OR (state = 'completed' AND completed_at_ms IS NOT NULL
                    AND completed_at_ms >= started_at_ms)
            ),
            backup_path TEXT
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_schema_migration_ledger_sequence
            ON schema_migration_ledger(sequence);
        ",
    )
}

fn verify_migration_ledger(connection: &Connection) -> rusqlite::Result<()> {
    let duplicate_ids: i64 = connection.query_row(
        "
        SELECT COUNT(*) FROM (
            SELECT migration_id FROM schema_migration_ledger
            GROUP BY migration_id HAVING COUNT(*) <> 1
        )
        ",
        [],
        |row| row.get(0),
    )?;
    let duplicate_sequences: i64 = connection.query_row(
        "
        SELECT COUNT(*) FROM (
            SELECT sequence FROM schema_migration_ledger
            GROUP BY sequence HAVING COUNT(*) <> 1
        )
        ",
        [],
        |row| row.get(0),
    )?;
    if duplicate_ids != 0 || duplicate_sequences != 0 {
        return Err(migration_recovery_error(
            "duplicate migration identifiers or sequence numbers detected",
        ));
    }

    let mut statement = connection.prepare(
        "
        SELECT sequence, migration_id, checksum_sha256, state,
               started_at_ms, completed_at_ms
        FROM schema_migration_ledger
        ORDER BY sequence ASC
        ",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, Option<i64>>(5)?,
        ))
    })?;

    for (index, row) in rows.enumerate() {
        let (sequence, migration_id, checksum, state, started_at_ms, completed_at_ms) = row?;
        let Some(expected) = MIGRATIONS.get(index) else {
            return Err(migration_recovery_error(&format!(
                "unknown migration {migration_id} at sequence {sequence}"
            )));
        };
        if sequence != expected.sequence || migration_id != expected.id {
            return Err(migration_recovery_error(&format!(
                "out-of-order migration ledger entry {migration_id} at sequence {sequence}; expected {} at sequence {}",
                expected.id, expected.sequence
            )));
        }
        let expected_checksum = migration_checksum(*expected)?;
        if checksum != expected_checksum {
            if accepts_legacy_runner_checksum(&checksum, *expected) {
                verify_schema_invariants(connection, sequence)?;
                eprintln!(
                    "OOMU_MIGRATION_CHECKSUM_COMPAT sequence={} id={} schema=verified",
                    sequence,
                    crate::redaction::redacted_log_text(&migration_id)
                );
            } else {
                return Err(migration_recovery_error(&format!(
                    "migration checksum mismatch for {migration_id}"
                )));
            }
        }
        if state != "completed"
            || completed_at_ms.is_none()
            || completed_at_ms.is_some_and(|completed| completed < started_at_ms)
        {
            return Err(migration_recovery_error(&format!(
                "partial migration ledger entry detected for {migration_id}"
            )));
        }
    }
    Ok(())
}

fn migration_completed(
    connection: &Connection,
    migration: MigrationDescriptor,
) -> rusqlite::Result<bool> {
    connection
        .query_row(
            "
            SELECT 1 FROM schema_migration_ledger
            WHERE sequence = ?1 AND migration_id = ?2 AND state = 'completed'
            ",
            params![migration.sequence, migration.id],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
}

fn migration_checksum(migration: MigrationDescriptor) -> rusqlite::Result<String> {
    match migration.source {
        MigrationSource::Sql(source) => Ok(hash_migration_material(source, &[])),
        MigrationSource::RustImplementation {
            contract,
            implementation_ids,
        } => {
            let implementations = implementation_ids
                .iter()
                .map(|implementation_id| {
                    migration_implementation_source(implementation_id)
                        .map(|source| (*implementation_id, source))
                })
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(hash_migration_material(contract, &implementations))
        }
        MigrationSource::HistoricalChecksum(checksum) => Ok(checksum.to_string()),
    }
}

fn migration_implementation_source(implementation_id: &str) -> rusqlite::Result<&'static str> {
    const DB_SOURCE: &str = include_str!("db.rs");
    let begin = format!("// MIGRATION_IMPL_BEGIN:{implementation_id}");
    let end = format!("// MIGRATION_IMPL_END:{implementation_id}");
    let begin_offset = DB_SOURCE.find(&begin).ok_or_else(|| {
        migration_recovery_error(&format!(
            "authoritative migration implementation start marker is missing for {implementation_id}"
        ))
    })? + begin.len();
    let tail = &DB_SOURCE[begin_offset..];
    let end_offset = tail.find(&end).ok_or_else(|| {
        migration_recovery_error(&format!(
            "authoritative migration implementation end marker is missing for {implementation_id}"
        ))
    })?;
    let source = &tail[..end_offset];
    if source.trim().is_empty() {
        return Err(migration_recovery_error(&format!(
            "authoritative migration implementation is empty for {implementation_id}"
        )));
    }
    Ok(source)
}

fn hash_migration_material(contract: &str, implementations: &[(&str, &str)]) -> String {
    let implementation_chunks = implementations
        .iter()
        .flat_map(|(implementation_id, source)| {
            [
                b"\0OOMU_RUST_MIGRATION_IMPLEMENTATION\0".as_slice(),
                implementation_id.as_bytes(),
                b"\0".as_slice(),
                source.as_bytes(),
            ]
        });
    sha256_chunks(std::iter::once(contract.as_bytes()).chain(implementation_chunks)).to_hex()
}

fn migration_recovery_error(message: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(format!("MIGRATION_RECOVERY_REQUIRED: {message}"))
}

fn verify_schema_invariants(connection: &Connection, through: i64) -> rusqlite::Result<()> {
    let integrity: String =
        connection.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
    if integrity != "ok" {
        return Err(migration_recovery_error(&format!(
            "database integrity check failed: {integrity}"
        )));
    }

    let foreign_key_failures: i64 =
        connection.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if foreign_key_failures != 0 {
        return Err(migration_recovery_error(&format!(
            "foreign_key_check reported {foreign_key_failures} violation(s)"
        )));
    }

    if through >= 1 {
        require_schema_objects(
            connection,
            "table",
            &[
                "intents",
                "actions",
                "certificates",
                "app_preferences",
                "chat_messages",
                "chat_sessions",
                "chat_turns",
                "message_queue",
                "channel_configs",
            ],
        )?;
        if through < 33 && !ch_migration::is_applied(connection)? {
            require_schema_objects(
                connection,
                "trigger",
                &[
                    "validate_whatsapp_owner_on_insert",
                    "validate_whatsapp_owner_on_update",
                ],
            )?;
        }
        connection.execute_batch("SAVEPOINT oomu_migration_write_probe;")?;
        let probe_result = (|| {
            connection.execute(
                "
                INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state)
                VALUES ('__migration_probe__', 'verified', ?1, ?2)
                ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at_ms=excluded.updated_at_ms
                ",
                params![unix_time_ms(), get_current_encryption_state()],
            )?;
            let value: String = connection.query_row(
                "SELECT value FROM app_preferences WHERE key='__migration_probe__'",
                [],
                |row| row.get(0),
            )?;
            if value != "verified" {
                return Err(migration_recovery_error(
                    "representative durable read/write probe returned the wrong value",
                ));
            }
            Ok(())
        })();
        connection.execute_batch(
            "ROLLBACK TO oomu_migration_write_probe; RELEASE oomu_migration_write_probe;",
        )?;
        probe_result?;
    }
    if through >= 26 {
        require_schema_objects(connection, "table", &["private_data_egress_receipts"])?;
        require_schema_objects(
            connection,
            "index",
            &["idx_private_egress_receipts_dispatch"],
        )?;
    }
    if through >= 27 {
        require_columns(connection, "remote_audit_receipts", &["signer_public_key"])?;
        require_schema_objects(connection, "table", &["remote_effect_outbox"])?;
        require_schema_objects(
            connection,
            "index",
            &[
                "idx_remote_command_final_receipt",
                "idx_remote_effect_outbox_pending",
            ],
        )?;
        require_schema_objects(
            connection,
            "trigger",
            &[
                "validate_remote_command_receipt_insert",
                "validate_remote_command_receipt_update",
            ],
        )?;
    }
    if through >= 28 {
        require_columns(
            connection,
            "remote_artifact_grants",
            &[
                "content_state",
                "source_sha256",
                "transfer_sha256",
                "source_path",
                "transfer_path",
                "redaction_manifest_sha256",
                "approval_receipt_id",
            ],
        )?;
        require_schema_objects(
            connection,
            "index",
            &["idx_remote_artifact_grants_retrievable"],
        )?;
        require_schema_objects(
            connection,
            "trigger",
            &[
                "validate_remote_artifact_grant_insert",
                "validate_remote_artifact_grant_update",
            ],
        )?;
    }
    if through >= 2 {
        require_schema_objects(
            connection,
            "table",
            &[
                "workflow_blueprints",
                "compiled_instructions",
                "execution_instances",
            ],
        )?;
        require_schema_objects(
            connection,
            "index",
            &["idx_execution_instances_status_updated"],
        )?;
    }
    if through >= 3 {
        require_columns(
            connection,
            "workflow_blueprints",
            &["compilation_status", "compilation_error"],
        )?;
        require_schema_objects(
            connection,
            "index",
            &["idx_workflow_blueprints_compilation_status"],
        )?;
    }
    if through >= 4 {
        if table_exists(connection, "execution_instances_before_approval_gateway")? {
            return Err(migration_recovery_error(
                "half-renamed execution_instances schema detected",
            ));
        }
        require_columns(
            connection,
            "execution_instances",
            &["memory_json", "selected_edges_json"],
        )?;
        require_schema_objects(
            connection,
            "index",
            &[
                "idx_execution_instances_status_updated",
                "idx_execution_instances_workflow",
            ],
        )?;
    }
    if through >= 5 {
        require_schema_objects(connection, "table", &["workflow_schedules"])?;
        require_schema_objects(
            connection,
            "index",
            &[
                "idx_workflow_schedules_due",
                "idx_workflow_schedules_workflow",
            ],
        )?;
    }
    if through >= 6 {
        require_columns(
            connection,
            "chat_messages",
            &[
                "workspace_id",
                "session_id",
                "provider_id",
                "model_id",
                "metadata_json",
                "is_compacted",
                "compaction_type",
            ],
        )?;
        require_columns(
            connection,
            "message_queue",
            &[
                "turn_id",
                "generation_token",
                "parent_turn_id",
                "root_turn_id",
                "turn_kind",
                "automated_web_grounding_enabled",
                "dynamic_routing_override",
            ],
        )?;
        require_schema_objects(
            connection,
            "index",
            &[
                "idx_chat_messages_workspace_session",
                "idx_chat_sessions_workspace_updated",
            ],
        )?;
    }
    if through >= 7 {
        require_schema_objects(
            connection,
            "table",
            &["sovereign_trust_policies", "active_trust_sessions"],
        )?;
    }
    if through >= 10 {
        require_schema_objects(
            connection,
            "table",
            &[
                "projects",
                "project_sources",
                "project_policy",
                "project_instructions",
            ],
        )?;
        require_columns(connection, "chat_sessions", &["project_id"])?;
        require_columns(connection, "workflow_blueprints", &["project_id"])?;
    }
    if through >= 11 {
        require_schema_objects(
            connection,
            "table",
            &[
                "task_runs",
                "task_events",
                "task_effects",
                "task_recovery_audit",
            ],
        )?;
    }
    if through >= 12 {
        require_schema_objects(
            connection,
            "table",
            &[
                "connector_accounts",
                "connector_project_bindings",
                "connector_oauth_attempts",
                "setup_progress",
                "activation_receipts",
                "setup_sample_tasks",
            ],
        )?;
    }
    if through >= 13 {
        require_schema_objects(
            connection,
            "table",
            &[
                "scheduler_owner_lease",
                "background_service_state",
                "routine_authority_grants",
                "routine_delivery_receipts",
                "routine_runs",
                "routine_remote_approvals",
            ],
        )?;
        require_columns(
            connection,
            "workflow_schedules",
            &[
                "routine_timezone",
                "schedule_kind",
                "missed_run_policy",
                "delivery_target_json",
                "authority_json",
            ],
        )?;
    }
    if through >= 14 {
        require_schema_objects(
            connection,
            "table",
            &[
                "browser_automation_sessions",
                "browser_automation_actions",
                "browser_download_quarantine",
            ],
        )?;
    }
    if through >= 15 {
        require_schema_objects(
            connection,
            "table",
            &[
                "artifact_records",
                "artifact_versions",
                "artifact_source_links",
                "artifact_exports",
            ],
        )?;
    }
    for (migration, tables) in [
        (
            16,
            &[
                "delegation_plans",
                "delegation_child_runs",
                "reviewed_approval_scopes",
                "approval_scope_audit",
            ][..],
        ),
        (17, &["connector_account_metadata"][..]),
        (
            18,
            &[
                "workbook_records",
                "workbook_revisions",
                "workbook_source_links",
                "workbook_exports",
                "workbook_template_imports",
            ][..],
        ),
        (
            19,
            &[
                "presentation_records",
                "presentation_revisions",
                "presentation_source_links",
                "presentation_exports",
                "presentation_template_imports",
            ][..],
        ),
    ] {
        if through >= migration {
            require_schema_objects(connection, "table", tables)?;
        }
    }
    if through >= 25 {
        require_columns(connection, "chat_turns", &["response_claimed_at_ms"])?;
    }
    if through >= 29 {
        require_schema_objects(
            connection,
            "table",
            &[
                "verified_filesystem_contexts",
                "pending_contextual_file_actions",
            ],
        )?;
    }
    if through >= 30 {
        require_schema_objects(
            connection,
            "index",
            &["idx_agent_executions_active_plan_origin"],
        )?;
        verify_agent_execution_origin_index(connection)?;
    }
    if through >= 32 {
        connector_scope_migration::verify(connection)?;
    }
    if through >= 41 {
        require_columns(
            connection,
            "active_session_configs",
            &[
                "local_provider_config_id",
                "local_provider_type",
                "local_route_generation",
            ],
        )?;
        require_columns(
            connection,
            "auto_route_baseline_backups",
            &[
                "local_provider_config_id",
                "local_provider_type",
                "local_route_generation",
            ],
        )?;
    }
    if through >= 42 {
        static_migrations::verify_truthful_background_runtime(connection)?;
    }
    Ok(())
}
fn require_schema_objects(
    connection: &Connection,
    object_type: &str,
    names: &[&str],
) -> rusqlite::Result<()> {
    for name in names {
        let exists = connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2 LIMIT 1",
                params![object_type, name],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(migration_recovery_error(&format!(
                "required {object_type} {name} is missing"
            )));
        }
    }
    Ok(())
}

fn require_columns(connection: &Connection, table: &str, columns: &[&str]) -> rusqlite::Result<()> {
    for column in columns {
        if !column_exists(connection, table, column)? {
            return Err(migration_recovery_error(&format!(
                "required column {table}.{column} is missing"
            )));
        }
    }
    Ok(())
}

fn count_operations_records(connection: &Connection) -> rusqlite::Result<usize> {
    let mut total = 0usize;
    for table in recovery_merge::OPERATIONS_RECOVERY_TABLES {
        if table_exists(connection, table)? {
            let count: i64 = connection.query_row(
                &format!("SELECT COUNT(*) FROM {}", quote_identifier(table)),
                [],
                |row| row.get(0),
            )?;
            total = total.saturating_add(count.max(0) as usize);
        }
    }
    Ok(total)
}

fn operations_database_has_no_user_schema(connection: &Connection) -> rusqlite::Result<bool> {
    let integrity: String =
        connection.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
    if integrity != "ok" {
        return Ok(false);
    }
    let user_tables: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )?;
    Ok(user_tables == 0)
}

fn verify_operations_database(connection: &Connection) -> rusqlite::Result<()> {
    let integrity: String =
        connection.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
    if integrity != "ok" {
        return Err(migration_recovery_error(&format!(
            "operations database integrity check failed: {integrity}"
        )));
    }
    let foreign_key_failures: i64 =
        connection.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if foreign_key_failures != 0 {
        return Err(migration_recovery_error(&format!(
            "operations database foreign_key_check reported {foreign_key_failures} violation(s)"
        )));
    }
    require_schema_objects(connection, "table", &["operations_store_metadata"])?;
    let schema_version: String = connection.query_row(
        "SELECT value FROM operations_store_metadata WHERE key='schema_version'",
        [],
        |row| row.get(0),
    )?;
    if schema_version != "1" {
        return Err(migration_recovery_error(
            "operations database schema marker is invalid",
        ));
    }

    connection.execute_batch(
        "
        SAVEPOINT oomu_operations_recovery_probe;
        CREATE TABLE __oomu_operations_recovery_probe__ (value TEXT NOT NULL);
        INSERT INTO __oomu_operations_recovery_probe__ (value) VALUES ('ok');
        ",
    )?;
    let value: String = connection.query_row(
        "SELECT value FROM __oomu_operations_recovery_probe__ LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    connection.execute_batch(
        "ROLLBACK TO oomu_operations_recovery_probe; RELEASE oomu_operations_recovery_probe;",
    )?;
    if value != "ok" {
        return Err(migration_recovery_error(
            "operations database durable read/write probe returned an unexpected value",
        ));
    }
    Ok(())
}
fn count_recoverable_records(connection: &Connection) -> rusqlite::Result<usize> {
    let mut total = 0usize;
    for table in recovery_merge::STATE_RECOVERY_TABLES {
        if table_exists(connection, table)? {
            let count: i64 = connection.query_row(
                &format!("SELECT COUNT(*) FROM {}", quote_identifier(table)),
                [],
                |row| row.get(0),
            )?;
            total = total.saturating_add(count.max(0) as usize);
        }
    }
    Ok(total)
}

fn durable_read_write_probe(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch("SAVEPOINT oomu_durable_recovery_probe;")?;
    let result = (|| {
        connection.execute(
            "
            INSERT INTO app_preferences (key, value, updated_at_ms, encryption_state)
            VALUES ('__durable_recovery_probe__', 'verified', ?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at_ms=excluded.updated_at_ms
            ",
            params![unix_time_ms(), get_current_encryption_state()],
        )?;
        let value: String = connection.query_row(
            "SELECT value FROM app_preferences WHERE key='__durable_recovery_probe__'",
            [],
            |row| row.get(0),
        )?;
        if value != "verified" {
            return Err(migration_recovery_error(
                "durable recovery read/write probe returned an unexpected value",
            ));
        }
        Ok(())
    })();
    connection.execute_batch(
        "ROLLBACK TO oomu_durable_recovery_probe; RELEASE oomu_durable_recovery_probe;",
    )?;
    result
}

// MIGRATION_IMPL_BEGIN:0006_chat_context
fn run_chat_context_migration(connection: &Connection, workspace_id: &str) -> rusqlite::Result<()> {
    for (table, column, sql) in [
        (
            "chat_messages",
            "workspace_id",
            format!(
                "ALTER TABLE chat_messages ADD COLUMN workspace_id TEXT NOT NULL DEFAULT '{}'",
                workspace_id
            ),
        ),
        (
            "chat_messages",
            "session_id",
            "ALTER TABLE chat_messages ADD COLUMN session_id TEXT NOT NULL DEFAULT ''".into(),
        ),
        (
            "chat_messages",
            "provider_id",
            "ALTER TABLE chat_messages ADD COLUMN provider_id TEXT".into(),
        ),
        (
            "chat_messages",
            "model_id",
            "ALTER TABLE chat_messages ADD COLUMN model_id TEXT".into(),
        ),
        (
            "chat_messages",
            "metadata_json",
            "ALTER TABLE chat_messages ADD COLUMN metadata_json TEXT".into(),
        ),
        (
            "chat_messages",
            "is_compacted",
            "ALTER TABLE chat_messages ADD COLUMN is_compacted INTEGER NOT NULL DEFAULT 0".into(),
        ),
        (
            "chat_messages",
            "compaction_type",
            "ALTER TABLE chat_messages ADD COLUMN compaction_type TEXT".into(),
        ),
        (
            "chat_sessions",
            "workspace_id",
            format!(
                "ALTER TABLE chat_sessions ADD COLUMN workspace_id TEXT NOT NULL DEFAULT '{}'",
                workspace_id
            ),
        ),
        (
            "chat_sessions",
            "title_source",
            "ALTER TABLE chat_sessions ADD COLUMN title_source TEXT NOT NULL DEFAULT 'auto'".into(),
        ),
        (
            "chat_sessions",
            "web_grounding_override",
            "ALTER TABLE chat_sessions ADD COLUMN web_grounding_override INTEGER".into(),
        ),
        (
            "chat_sessions",
            "dynamic_routing_override",
            "ALTER TABLE chat_sessions ADD COLUMN dynamic_routing_override INTEGER".into(),
        ),
    ] {
        add_column_if_missing(connection, table, column, &sql)?;
    }
    for (column, sql) in [
        (
            "turn_id",
            "ALTER TABLE message_queue ADD COLUMN turn_id TEXT",
        ),
        (
            "generation_token",
            "ALTER TABLE message_queue ADD COLUMN generation_token TEXT",
        ),
        (
            "parent_turn_id",
            "ALTER TABLE message_queue ADD COLUMN parent_turn_id TEXT",
        ),
        (
            "root_turn_id",
            "ALTER TABLE message_queue ADD COLUMN root_turn_id TEXT",
        ),
        (
            "turn_kind",
            "ALTER TABLE message_queue ADD COLUMN turn_kind TEXT",
        ),
        (
            "automated_web_grounding_enabled",
            "ALTER TABLE message_queue ADD COLUMN automated_web_grounding_enabled INTEGER",
        ),
        (
            "dynamic_routing_override",
            "ALTER TABLE message_queue ADD COLUMN dynamic_routing_override INTEGER",
        ),
        (
            "auto_route_identity_json",
            "ALTER TABLE message_queue ADD COLUMN auto_route_identity_json TEXT",
        ),
    ] {
        add_column_if_missing(connection, "message_queue", column, sql)?;
    }
    connection.execute(
        "
        UPDATE chat_sessions SET title_source = 'user'
        WHERE title_source = 'auto' AND trim(title) <> ''
          AND title <> 'New Session' AND title NOT LIKE '% Session'
        ",
        [],
    )?;
    connection.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_chat_messages_session_id ON chat_messages(session_id);
        CREATE INDEX IF NOT EXISTS idx_chat_messages_workspace_session
            ON chat_messages(workspace_id, session_id, timestamp_ms, id);
        CREATE INDEX IF NOT EXISTS idx_chat_messages_workspace_session_active
            ON chat_messages(workspace_id, session_id, is_compacted, timestamp_ms, id);
        CREATE INDEX IF NOT EXISTS idx_chat_messages_workspace_agent
            ON chat_messages(workspace_id, agent_id, timestamp_ms, id);
        CREATE INDEX IF NOT EXISTS idx_chat_sessions_workspace_updated
            ON chat_sessions(workspace_id, updated_at_ms DESC);
        ",
    )
}
// MIGRATION_IMPL_END:0006_chat_context

fn create_verified_migration_backup(
    connection: &Connection,
    database_path: &Path,
    database_key: &str,
    migration_id: &str,
) -> rusqlite::Result<PathBuf> {
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let parent = database_path.parent().ok_or_else(|| {
        migration_recovery_error("database path has no parent for migration backup")
    })?;
    let backup_dir = parent.join(".oomu-migration-backups");
    fs::create_dir_all(&backup_dir).map_err(io_to_sql_error)?;
    set_private_directory(&backup_dir)?;
    let mut random = [0u8; 16];
    OsRng.fill_bytes(&mut random);
    let backup_path = backup_dir.join(format!(
        "{}-{}-{}.sqlite",
        migration_id,
        unix_time_ms(),
        hex::encode(random)
    ));
    let result = (|| {
        fs::copy(database_path, &backup_path).map_err(io_to_sql_error)?;
        set_private_file(&backup_path)?;
        if has_plaintext_sqlite_header(&backup_path) {
            return Err(migration_recovery_error(
                "pre-migration backup unexpectedly contains a plaintext SQLite header",
            ));
        }
        let backup = open_sqlcipher_database_connection_with_key(&backup_path, database_key)?;
        let integrity: String =
            backup.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
        if integrity != "ok" {
            return Err(migration_recovery_error(&format!(
                "pre-migration backup validation failed: {integrity}"
            )));
        }
        verify_migration_ledger(&backup)?;
        Ok(())
    })();
    if let Err(error) = result {
        remove_sqlite_sidecars(&backup_path);
        let _ = fs::remove_file(&backup_path);
        return Err(error);
    }
    Ok(backup_path)
}

fn create_verified_operations_copy(
    source_path: &Path,
    destination_path: &Path,
    database_key: &str,
) -> rusqlite::Result<()> {
    if destination_path.exists() {
        return Err(migration_recovery_error(
            "operations database copy destination already exists",
        ));
    }
    let result = (|| {
        let source = open_sqlcipher_database_connection_with_key(source_path, database_key)?;
        verify_operations_database(&source)?;
        source.execute_batch("PRAGMA wal_checkpoint(FULL);")?;
        let source_records = count_operations_records(&source)?;

        let mut destination =
            open_sqlcipher_database_connection_with_key(destination_path, database_key)?;
        {
            let backup = rusqlite::backup::Backup::new(&source, &mut destination)?;
            backup.run_to_completion(128, Duration::from_millis(5), None)?;
        }
        verify_operations_database(&destination)?;
        let destination_records = count_operations_records(&destination)?;
        if destination_records != source_records {
            return Err(migration_recovery_error(&format!(
                "operations database copy record-count verification failed: expected {source_records}, found {destination_records}"
            )));
        }
        destination.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        drop(destination);
        remove_sqlite_sidecars(destination_path);
        set_private_file(destination_path)?;
        if has_plaintext_sqlite_header(destination_path) {
            return Err(migration_recovery_error(
                "operations database copy unexpectedly contains a plaintext SQLite header",
            ));
        }
        Ok(())
    })();
    if let Err(error) = result {
        remove_sqlite_sidecars(destination_path);
        let _ = fs::remove_file(destination_path);
        return Err(error);
    }
    Ok(())
}

fn create_verified_operations_backup(
    source_path: &Path,
    database_key: &str,
    backup_label: &str,
) -> rusqlite::Result<PathBuf> {
    let parent = source_path.parent().ok_or_else(|| {
        migration_recovery_error("operations database path has no parent for recovery backup")
    })?;
    let backup_dir = parent.join(".oomu-migration-backups");
    fs::create_dir_all(&backup_dir).map_err(io_to_sql_error)?;
    set_private_directory(&backup_dir)?;
    let mut random = [0u8; 16];
    OsRng.fill_bytes(&mut random);
    let backup_path = backup_dir.join(format!(
        "{}-{}-{}.sqlite",
        backup_label,
        unix_time_ms(),
        hex::encode(random)
    ));
    create_verified_operations_copy(source_path, &backup_path, database_key)?;
    Ok(backup_path)
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> rusqlite::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_to_sql_error)
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> rusqlite::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file(path: &Path) -> rusqlite::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(io_to_sql_error)
}

fn enforce_private_sqlite_files(path: &Path) -> rusqlite::Result<()> {
    if path.exists() {
        set_private_file(path)?;
    }
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    for suffix in ["-wal", "-shm"] {
        let sidecar = path.with_file_name(format!("{file_name}{suffix}"));
        if sidecar.exists() {
            set_private_file(&sidecar)?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_private_file(_path: &Path) -> rusqlite::Result<()> {
    Ok(())
}

// MIGRATION_IMPL_BEGIN:shared_schema_probes
fn table_exists(connection: &Connection, table: &str) -> rusqlite::Result<bool> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1 LIMIT 1",
            params![table],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn io_to_sql_error(error: io::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    sql: &str,
) -> rusqlite::Result<()> {
    if column_exists(connection, table, column)? {
        return Ok(());
    }
    connection.execute(sql, [])?;
    Ok(())
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(true);
        }
    }
    Ok(false)
}
// MIGRATION_IMPL_END:shared_schema_probes

fn json_to_sql_error(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

fn json_from_sql_error(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

fn parse_node_kind(value: &str) -> rusqlite::Result<WorkflowNodeKind> {
    match value {
        "input" => Ok(WorkflowNodeKind::Input),
        "agent" => Ok(WorkflowNodeKind::Agent),
        "router" => Ok(WorkflowNodeKind::Router),
        "conditional" => Ok(WorkflowNodeKind::Conditional),
        "loop" => Ok(WorkflowNodeKind::Loop),
        "permission" => Ok(WorkflowNodeKind::Permission),
        "mcp_tool" => Ok(WorkflowNodeKind::McpTool),
        "system_action" => Ok(WorkflowNodeKind::SystemAction),
        "output" => Ok(WorkflowNodeKind::Output),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn execution_status(status: ExecutionStatus) -> &'static str {
    match status {
        ExecutionStatus::Pending => "Pending",
        ExecutionStatus::Running => "Running",
        ExecutionStatus::AwaitingApproval => "AwaitingApproval",
        ExecutionStatus::Completed => "Completed",
        ExecutionStatus::Failed => "Failed",
    }
}

fn parse_execution_status(value: &str) -> rusqlite::Result<ExecutionStatus> {
    match value {
        "Pending" => Ok(ExecutionStatus::Pending),
        "Running" => Ok(ExecutionStatus::Running),
        "AwaitingApproval" => Ok(ExecutionStatus::AwaitingApproval),
        "Completed" => Ok(ExecutionStatus::Completed),
        "Failed" => Ok(ExecutionStatus::Failed),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn write_execution_instance(
    connection: &Connection,
    instance: &ExecutionInstance,
    update: bool,
) -> rusqlite::Result<()> {
    let input_payload_json =
        serde_json::to_string(&instance.input_payload).map_err(json_to_sql_error)?;
    let output_payload_json = instance
        .output_payload
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(json_to_sql_error)?;
    let node_payloads_json =
        serde_json::to_string(&instance.node_payloads).map_err(json_to_sql_error)?;
    let memory_json = serde_json::to_string(&instance.memory).map_err(json_to_sql_error)?;
    let selected_edges_json =
        serde_json::to_string(&instance.selected_edges).map_err(json_to_sql_error)?;
    let pause_context_json = instance
        .pause_context
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(json_to_sql_error)?;
    let error_json = instance
        .error
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(json_to_sql_error)?;
    let changed = if update {
        connection.execute(
            "
            UPDATE execution_instances
            SET status = ?2, active_node_id = ?3, input_payload_json = ?4,
                output_payload_json = ?5, node_payloads_json = ?6, memory_json = ?7,
                selected_edges_json = ?8, pause_context_json = ?9, error_json = ?10,
                execution_latency_ms = ?11, prompt_tokens = ?12,
                completion_tokens = ?13, total_tokens = ?14, started_at_ms = ?15,
                updated_at_ms = ?16, completed_at_ms = ?17
            WHERE id = ?1
            ",
            params![
                instance.id,
                execution_status(instance.status),
                instance.active_node_id,
                input_payload_json,
                output_payload_json,
                node_payloads_json,
                memory_json,
                selected_edges_json,
                pause_context_json,
                error_json,
                instance.execution_latency_ms,
                instance.prompt_tokens,
                instance.completion_tokens,
                instance.total_tokens,
                instance.started_at_ms,
                instance.updated_at_ms,
                instance.completed_at_ms,
            ],
        )?
    } else {
        connection.execute(
            "
            INSERT INTO execution_instances (
                id, workflow_id, workflow_version, status, active_node_id, input_payload_json,
                output_payload_json, node_payloads_json, memory_json, selected_edges_json,
                pause_context_json, error_json, execution_latency_ms, prompt_tokens,
                completion_tokens, total_tokens, created_at_ms, started_at_ms,
                updated_at_ms, completed_at_ms, encryption_state
            )
            VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, ?21
            )
            ",
            params![
                instance.id,
                instance.workflow_id,
                instance.workflow_version,
                execution_status(instance.status),
                instance.active_node_id,
                input_payload_json,
                output_payload_json,
                node_payloads_json,
                memory_json,
                selected_edges_json,
                pause_context_json,
                error_json,
                instance.execution_latency_ms,
                instance.prompt_tokens,
                instance.completion_tokens,
                instance.total_tokens,
                instance.created_at_ms,
                instance.started_at_ms,
                instance.updated_at_ms,
                instance.completed_at_ms,
                get_current_encryption_state(),
            ],
        )?
    };
    if changed != 1 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    Ok(())
}

fn select_execution_instance(
    connection: &Connection,
    id: &str,
) -> rusqlite::Result<ExecutionInstance> {
    connection.query_row(
        "
        SELECT id, workflow_id, workflow_version, status, active_node_id,
               input_payload_json, output_payload_json, node_payloads_json,
               memory_json, selected_edges_json, pause_context_json, error_json,
               execution_latency_ms, prompt_tokens, completion_tokens, total_tokens,
               created_at_ms, started_at_ms, updated_at_ms, completed_at_ms
        FROM execution_instances
        WHERE id = ?1
        ",
        params![id],
        |row| {
            Ok(ExecutionInstance {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                workflow_version: row.get(2)?,
                status: parse_execution_status(&row.get::<_, String>(3)?)?,
                active_node_id: row.get(4)?,
                input_payload: serde_json::from_str(&row.get::<_, String>(5)?)
                    .map_err(json_from_sql_error)?,
                output_payload: row
                    .get::<_, Option<String>>(6)?
                    .map(|value| serde_json::from_str(&value).map_err(json_from_sql_error))
                    .transpose()?,
                node_payloads: serde_json::from_str(&row.get::<_, String>(7)?)
                    .map_err(json_from_sql_error)?,
                memory: serde_json::from_str(&row.get::<_, String>(8)?)
                    .map_err(json_from_sql_error)?,
                selected_edges: serde_json::from_str(&row.get::<_, String>(9)?)
                    .map_err(json_from_sql_error)?,
                pause_context: row
                    .get::<_, Option<String>>(10)?
                    .map(|value| serde_json::from_str(&value).map_err(json_from_sql_error))
                    .transpose()?,
                error: row
                    .get::<_, Option<String>>(11)?
                    .map(|value| serde_json::from_str(&value).map_err(json_from_sql_error))
                    .transpose()?,
                execution_latency_ms: row.get(12)?,
                prompt_tokens: row.get(13)?,
                completion_tokens: row.get(14)?,
                total_tokens: row.get(15)?,
                created_at_ms: row.get(16)?,
                started_at_ms: row.get(17)?,
                updated_at_ms: row.get(18)?,
                completed_at_ms: row.get(19)?,
            })
        },
    )
}

fn select_workflow_schedule_by_id(
    connection: &Connection,
    id: &str,
) -> rusqlite::Result<WorkflowScheduleRecord> {
    connection.query_row(
        "
        SELECT id, workflow_id, workflow_version, label, schedule_expression,
               run_request_json, is_active, next_run_at_ms, claimed_at_ms,
               last_started_at_ms, last_completed_at_ms, last_status,
               last_error, last_instance_id, created_at_ms, updated_at_ms,
               routine_timezone, schedule_kind, project_id, missed_run_policy,
               missed_run_cap, active_window_start_minute, active_window_end_minute,
               delivery_target_json, authority_json
        FROM workflow_schedules
        WHERE id = ?1
        ",
        params![id],
        workflow_schedule_from_row,
    )
}

fn select_due_workflow_schedules(
    connection: &Connection,
    now_ms: i64,
    lease_cutoff_ms: i64,
    limit: usize,
) -> rusqlite::Result<Vec<WorkflowScheduleRecord>> {
    let mut statement = connection.prepare(
        "
        SELECT id, workflow_id, workflow_version, label, schedule_expression,
               run_request_json, is_active, next_run_at_ms, claimed_at_ms,
               last_started_at_ms, last_completed_at_ms, last_status,
               last_error, last_instance_id, created_at_ms, updated_at_ms,
               routine_timezone, schedule_kind, project_id, missed_run_policy,
               missed_run_cap, active_window_start_minute, active_window_end_minute,
               delivery_target_json, authority_json
        FROM workflow_schedules
        WHERE is_active = 1
          AND next_run_at_ms IS NOT NULL
          AND next_run_at_ms <= ?1
          AND (claimed_at_ms IS NULL OR claimed_at_ms <= ?2)
        ORDER BY next_run_at_ms ASC, updated_at_ms ASC
        LIMIT ?3
        ",
    )?;
    let rows = statement.query_map(params![now_ms, lease_cutoff_ms, limit as i64], |row| {
        workflow_schedule_from_row(row)
    })?;
    rows.collect()
}

fn workflow_schedule_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowScheduleRecord> {
    let run_request_json: String = row.get(5)?;
    Ok(WorkflowScheduleRecord {
        id: row.get(0)?,
        workflow_id: row.get(1)?,
        workflow_version: row.get(2)?,
        label: row.get(3)?,
        schedule_expression: row.get(4)?,
        run_request: serde_json::from_str(&run_request_json).map_err(json_from_sql_error)?,
        is_active: row.get::<_, i64>(6)? != 0,
        next_run_at_ms: row.get(7)?,
        claimed_at_ms: row.get(8)?,
        last_started_at_ms: row.get(9)?,
        last_completed_at_ms: row.get(10)?,
        last_status: row.get(11)?,
        last_error: row.get(12)?,
        last_instance_id: row.get(13)?,
        created_at_ms: row.get(14)?,
        updated_at_ms: row.get(15)?,
        routine_timezone: row.get(16)?,
        schedule_kind: row.get(17)?,
        project_id: row.get(18)?,
        missed_run_policy: row.get(19)?,
        missed_run_cap: row.get::<_, i64>(20)? as u8,
        active_window_start_minute: row.get(21)?,
        active_window_end_minute: row.get(22)?,
        delivery_target: serde_json::from_str(&row.get::<_, String>(23)?)
            .map_err(json_from_sql_error)?,
        authority: serde_json::from_str(&row.get::<_, String>(24)?).map_err(json_from_sql_error)?,
    })
}

fn select_compiled_instruction(
    connection: &Connection,
    workflow_id: &str,
    workflow_version: u32,
    node_id: &str,
) -> rusqlite::Result<CompiledInstruction> {
    connection.query_row(
        "
        SELECT id, workflow_id, workflow_version, node_id, node_kind, system_prompt,
               input_variable_mappings_json, evaluation_protocol_json, compiler_model,
               compiler_version, created_at_ms
        FROM compiled_instructions
        WHERE workflow_id = ?1 AND workflow_version = ?2 AND node_id = ?3
        ",
        params![workflow_id, workflow_version, node_id],
        |row| {
            Ok(CompiledInstruction {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                workflow_version: row.get(2)?,
                node_id: row.get(3)?,
                node_kind: parse_node_kind(&row.get::<_, String>(4)?)?,
                system_prompt: row.get(5)?,
                input_variable_mappings: serde_json::from_str(&row.get::<_, String>(6)?)
                    .map_err(json_from_sql_error)?,
                evaluation_protocol: serde_json::from_str(&row.get::<_, String>(7)?)
                    .map_err(json_from_sql_error)?,
                compiler_model: row.get(8)?,
                compiler_version: row.get(9)?,
                created_at_ms: row.get(10)?,
            })
        },
    )
}

fn upsert_legacy_workflow(
    connection: &Connection,
    workflow: &SavedWorkflowRecord,
    project_id: Option<&str>,
) -> rusqlite::Result<()> {
    connection.execute(
        "
        INSERT INTO workflows (id, name, steps, created_at, updated_at, project_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            steps = excluded.steps,
            updated_at = excluded.updated_at,
            project_id = excluded.project_id
        ",
        params![
            workflow.id,
            workflow.name,
            workflow.steps,
            workflow.created_at,
            workflow.updated_at,
            project_id,
        ],
    )?;
    Ok(())
}

fn resolve_workflow_project_binding(
    transaction: &rusqlite::Transaction<'_>,
    workflow_id: &str,
    requested_project_id: Option<&str>,
) -> rusqlite::Result<Option<String>> {
    let requested_project_id = requested_project_id
        .map(str::trim)
        .map(|value| {
            if value.is_empty() {
                return Err(rusqlite::Error::InvalidParameterName(
                    "projectId must be a valid Project id when supplied.".to_string(),
                ));
            }
            ProjectId::parse(value)
                .map(|project_id| project_id.to_string())
                .map_err(|error| rusqlite::Error::InvalidParameterName(error.to_string()))
        })
        .transpose()?;

    let workflow_project_id = transaction
        .query_row(
            "SELECT project_id FROM workflows WHERE id=?1",
            params![workflow_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let mut statement = transaction.prepare(
        "SELECT DISTINCT project_id FROM workflow_blueprints WHERE workflow_id=?1 AND project_id IS NOT NULL",
    )?;
    let blueprint_project_ids = statement
        .query_map(params![workflow_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    if blueprint_project_ids.len() > 1
        || workflow_project_id
            .as_ref()
            .zip(blueprint_project_ids.first())
            .is_some_and(|(workflow, blueprint)| workflow != blueprint)
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "Workflow versions have conflicting Project bindings.".to_string(),
        ));
    }
    let existing_project_id =
        workflow_project_id.or_else(|| blueprint_project_ids.first().cloned());
    if requested_project_id
        .as_ref()
        .zip(existing_project_id.as_ref())
        .is_some_and(|(requested, existing)| requested != existing)
    {
        return Err(rusqlite::Error::InvalidParameterName(
            "A saved Workflow cannot be moved to a different Project. Duplicate it in the intended Project instead."
                .to_string(),
        ));
    }
    let project_id = requested_project_id.or(existing_project_id);
    if let Some(project_id) = project_id.as_deref() {
        let project_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1 AND archived_at_ms IS NULL)",
            params![project_id],
            |row| row.get(0),
        )?;
        if !project_exists {
            return Err(rusqlite::Error::InvalidParameterName(
                "The selected Project is unavailable.".to_string(),
            ));
        }
        transaction.execute(
            "UPDATE workflows SET project_id=?2 WHERE id=?1 AND project_id IS NULL",
            params![workflow_id, project_id],
        )?;
        transaction.execute(
            "UPDATE workflow_blueprints SET project_id=?2 WHERE workflow_id=?1 AND project_id IS NULL",
            params![workflow_id, project_id],
        )?;
    }
    Ok(project_id)
}

fn chat_session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatSessionRecord> {
    Ok(ChatSessionRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        project_id: row.get(2)?,
        agent_id: row.get(3)?,
        title: row.get(4)?,
        title_source: row.get(5)?,
        provider_id: row.get(6)?,
        model_id: row.get(7)?,
        web_grounding_override: row.get(8)?,
        dynamic_routing_override: row.get(9)?,
        unread_completion: row.get::<_, i64>(10)? != 0,
        created_at_ms: row.get(11)?,
        updated_at_ms: row.get(12)?,
    })
}

fn workspace_id_from_request(
    value: Option<&str>,
    active_workspace_id: &str,
) -> rusqlite::Result<String> {
    let workspace_id = match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => normalize_workspace_id(Some(value)).map_err(database_key_error)?,
        None => active_workspace_id.to_string(),
    };
    if workspace_id != active_workspace_id {
        return Err(rusqlite::Error::InvalidParameterName(
            "chat session workspace_id does not match the active workspace namespace".to_string(),
        ));
    }
    Ok(workspace_id)
}

fn workspace_id_for_chat_session(
    connection: &Connection,
    session_id: &str,
    active_workspace_id: &str,
) -> rusqlite::Result<String> {
    let stored_workspace_id = connection
        .query_row(
            "SELECT workspace_id FROM chat_sessions WHERE id = ?1",
            params![session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    match stored_workspace_id {
        Some(value) if value == active_workspace_id => Ok(value),
        Some(_) => Err(rusqlite::Error::InvalidParameterName(
            "chat session belongs to a different workspace namespace".to_string(),
        )),
        None => Ok(active_workspace_id.to_string()),
    }
}

fn select_active_chat_messages_for_session(
    connection: &Connection,
    workspace_id: &str,
    session_id: &str,
) -> rusqlite::Result<Vec<ChatMessageRecord>> {
    let mut statement = connection.prepare(
        "
        SELECT id, workspace_id, session_id, role, content, timestamp_ms, provider_id, model_id,
               metadata_json, is_compacted, compaction_type
        FROM chat_messages
        WHERE workspace_id = ?1
          AND session_id = ?2
          AND COALESCE(is_compacted, 0) = 0
        ORDER BY timestamp_ms ASC, id ASC
        ",
    )?;
    let rows = statement.query_map(params![workspace_id, session_id], chat_message_from_row)?;
    rows.collect()
}

fn chat_message_from_row(row: &Row<'_>) -> rusqlite::Result<ChatMessageRecord> {
    Ok(ChatMessageRecord {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        session_id: row.get(2)?,
        role: row.get(3)?,
        content: row.get(4)?,
        created_at_ms: row.get(5)?,
        provider_id: row.get(6)?,
        model_id: row.get(7)?,
        metadata_json: row.get(8)?,
        is_compacted: row.get::<_, i64>(9)? != 0,
        compaction_type: row.get(10)?,
    })
}

#[derive(Debug, Clone, Default)]
struct SovereignLedgerChatCounts {
    local_turns: u64,
    cloud_turns: u64,
    protected_input_tokens: u64,
    protected_output_tokens: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct SovereignLedgerTokenEstimate {
    input_tokens: u64,
    output_tokens: u64,
}

fn effective_ledger_since_ms(
    requested_since_ms: Option<i64>,
    reset_at_ms: Option<i64>,
) -> Option<i64> {
    [requested_since_ms, reset_at_ms]
        .into_iter()
        .flatten()
        .filter(|value| *value > 0)
        .max()
}

fn ledger_token_estimate_from_value(metadata: &Value) -> SovereignLedgerTokenEstimate {
    SovereignLedgerTokenEstimate {
        input_tokens: ledger_u64_field(
            metadata,
            &[
                "input_tokens_estimate",
                "inputTokensEstimate",
                "promptTokens",
                "prompt_tokens",
                "estimatedInputTokens",
            ],
        ),
        output_tokens: ledger_u64_field(
            metadata,
            &[
                "output_tokens_estimate",
                "outputTokensEstimate",
                "completionTokens",
                "completion_tokens",
                "estimatedOutputTokens",
            ],
        ),
    }
}

fn ledger_u64_field(metadata: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| metadata.get(*key))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_f64().map(|number| number.max(0.0).round() as u64))
                .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
        })
        .unwrap_or(0)
}

fn ledger_metadata_route_is_local(metadata: &Value) -> bool {
    let provider_id = ledger_string_field(
        metadata,
        &[
            "executingProviderId",
            "executing_provider_id",
            "targetProviderId",
            "target_provider_id",
        ],
    );
    let model_id = ledger_string_field(
        metadata,
        &[
            "executingModelId",
            "executing_model_id",
            "targetModelId",
            "target_model_id",
        ],
    );
    ledger_route_is_local(provider_id.as_deref(), model_id.as_deref())
}

fn ledger_string_field(metadata: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| metadata.get(*key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn ledger_route_is_local(provider_id: Option<&str>, model_id: Option<&str>) -> bool {
    let provider = provider_id
        .unwrap_or_default()
        .trim()
        .to_lowercase()
        .replace(['-', ' '], "_");
    let model = model_id.unwrap_or_default().trim().to_lowercase();
    matches!(
        provider.as_str(),
        "local" | "local_model" | "local_gemma" | "gemma"
    ) || model.starts_with("gemma-")
        || model.contains("local")
}

fn ledger_estimated_api_savings(input_tokens: u64, output_tokens: u64) -> f64 {
    (input_tokens as f64 / 1_000_000.0) * LEDGER_AVOIDED_API_INPUT_USD_PER_MILLION
        + (output_tokens as f64 / 1_000_000.0) * LEDGER_AVOIDED_API_OUTPUT_USD_PER_MILLION
}

fn ledger_protected_megabytes(tokens: u64) -> f64 {
    (tokens as f64 * LEDGER_BYTES_PER_TOKEN_ESTIMATE) / (1024.0 * 1024.0)
}

fn estimate_ledger_tokens(content: &str) -> u64 {
    ((content.chars().count() + 3) / 4) as u64
}

fn select_session_config_for_connection(
    connection: &Connection,
    session_id: &str,
) -> rusqlite::Result<Option<SessionConfigRecord>> {
    connection
        .query_row(
            "
            SELECT session_id, reasoning_depth, context_budget, model_id, updated_at,
                   local_provider_config_id, local_provider_type, local_route_generation
            FROM active_session_configs
            WHERE session_id = ?1
            ",
            params![session_id.trim()],
            session_config_from_row,
        )
        .optional()
}

fn compaction_anchor_agent_id(
    connection: &Connection,
    workspace_id: &str,
    session_id: &str,
) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            "
            SELECT agent_id
            FROM chat_messages
            WHERE workspace_id = ?1 AND session_id = ?2
            ORDER BY timestamp_ms ASC, id ASC
            LIMIT 1
            ",
            params![workspace_id, session_id],
            |row| row.get(0),
        )
        .optional()
}

fn route_uses_local_context(provider_id: &str, model_id: &str) -> bool {
    let provider_key = normalized_route_key(provider_id);
    let model_key = normalized_route_key(model_id);
    if provider_key.is_empty() && model_key.is_empty() {
        return true;
    }
    matches!(
        provider_key.as_str(),
        "local" | "local_model" | "local_gemma" | "ollama"
    ) || model_key.contains("gemma")
        || model_key.contains("gguf")
        || model_key.contains("local")
}

fn resolved_context_horizon_tokens(
    provider_id: &str,
    model_id: &str,
    configured_budget: usize,
    is_cloud_model: bool,
) -> usize {
    if !is_cloud_model {
        // Use the session's saved, hardware-bounded context budget so the
        // compaction horizon matches the route the user selected.
        return configured_budget.max(1);
    }
    known_cloud_context_tokens(provider_id, model_id)
        .unwrap_or(DEFAULT_CLOUD_CONTEXT_TOKENS)
        .max(configured_budget.max(DEFAULT_CLOUD_CONTEXT_TOKENS))
}

fn known_cloud_context_tokens(provider_id: &str, model_id: &str) -> Option<usize> {
    let provider_key = normalized_route_key(provider_id);
    let model_key = normalized_route_key(model_id);
    if model_key.contains("gemini_3_1") {
        return Some(2_097_152);
    }
    if provider_key.contains("gemini") || model_key.contains("gemini") {
        return Some(1_048_576);
    }
    if model_key.contains("claude_fable_5")
        || provider_key.contains("anthropic")
        || provider_key.contains("claude")
        || model_key.contains("claude")
    {
        return Some(204_800);
    }
    if model_key.contains("gpt_5_5") {
        return Some(131_072);
    }
    if provider_key.contains("openai") || model_key.contains("gpt") {
        return Some(131_072);
    }
    None
}

fn normalized_route_key(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn chat_message_is_protected_from_compaction(message: &ChatMessageRecord) -> bool {
    let Some(metadata) = message
        .metadata_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
    else {
        return false;
    };
    metadata
        .get("uiOnlyCheckpoint")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || metadata
            .get("turnState")
            .and_then(Value::as_str)
            .is_some_and(|state| {
                matches!(state, "accepted" | "interrupted" | "running" | "processing")
            })
}

fn routing_lookup_argument(key: Option<String>, route_key: Option<String>) -> Option<String> {
    route_key
        .or(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn clean_required_routing_text(field: &str, value: String) -> Result<String, AgenticLoopError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AgenticLoopError::from_persistence(format!(
            "Routing preference {field} cannot be empty."
        )));
    }
    if value.len() > 512 {
        return Err(AgenticLoopError::from_persistence(format!(
            "Routing preference {field} is too long."
        )));
    }
    Ok(value.to_string())
}

fn clean_session_config_id(value: String) -> Result<String, AgenticLoopError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AgenticLoopError::from_persistence(
            "Session config requires a session_id.".to_string(),
        ));
    }
    if value.len() > 256 {
        return Err(AgenticLoopError::from_persistence(
            "Session config session_id is too long.".to_string(),
        ));
    }
    Ok(value.to_string())
}

pub fn get_default_reasoning_depth_for_provider(provider_id: &str) -> String {
    let provider_key = normalized_route_key(provider_id);
    if provider_key.contains("google") || provider_key.contains("gemini") {
        return "medium".to_string();
    }
    if provider_key.contains("openai")
        || provider_key.contains("gpt")
        || provider_key.contains("anthropic")
        || provider_key.contains("claude")
    {
        return "high".to_string();
    }
    if provider_key.contains("local")
        || provider_key.contains("gemma")
        || provider_key.contains("native")
    {
        return "low".to_string();
    }
    "medium".to_string()
}

fn default_session_reasoning_depth(provider_id: Option<&str>) -> String {
    provider_id
        .map(get_default_reasoning_depth_for_provider)
        .unwrap_or_else(|| "medium".to_string())
}

fn clean_session_reasoning_depth(
    provider_id: Option<&str>,
    value: String,
) -> Result<String, AgenticLoopError> {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "ultra" => Ok("max".to_string()),
        "on" if provider_id
            .map(|provider_id| {
                let provider_key = normalized_route_key(provider_id);
                provider_key.contains("local")
                    || provider_key.contains("gemma")
                    || provider_key.contains("native")
            })
            .unwrap_or(false) =>
        {
            Ok("on".to_string())
        }
        "on" => Ok("medium".to_string()),
        "off" | "low" | "medium" | "high" | "xhigh" | "max" => Ok(normalized),
        _ => Ok(default_session_reasoning_depth(provider_id)),
    }
}

fn clean_context_budget(value: i32) -> Result<i32, AgenticLoopError> {
    if !(1..=1_000_000).contains(&value) {
        return Err(AgenticLoopError::from_persistence(
            "Session config context_budget must be between 1 and 1000000.".to_string(),
        ));
    }
    Ok(value)
}

fn agent_execution_log_from_row(row: &Row<'_>) -> rusqlite::Result<AgentExecutionLogRecord> {
    Ok(AgentExecutionLogRecord {
        id: row.get(0)?,
        execution_id: row.get(1)?,
        plan_id: row.get(2)?,
        session_id: row.get(3)?,
        agent_id: row.get(4)?,
        level: row.get(5)?,
        phase: row.get(6)?,
        message: row.get(7)?,
        payload_json: row.get(8)?,
        created_at_ms: row.get(9)?,
    })
}

fn select_agent_execution_log_by_id(
    connection: &Connection,
    id: i64,
) -> rusqlite::Result<AgentExecutionLogRecord> {
    connection.query_row(
        "
        SELECT id, execution_id, plan_id, session_id, agent_id, level,
               phase, message, payload_json, created_at_ms
        FROM agent_execution_logs
        WHERE id = ?1
        ",
        params![id],
        agent_execution_log_from_row,
    )
}

fn workflow_projection_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SavedWorkflowProjectionRecord> {
    let workflow_ir_json = row.get::<_, Option<String>>(6)?;
    let review_capabilities = workflow_ir_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<WorkflowIr>(value).ok())
        .map(|workflow_ir| crate::workflow_ir::review::workflow_review_capabilities(&workflow_ir));
    Ok(SavedWorkflowProjectionRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        steps: row.get(2)?,
        project_id: row.get(3)?,
        workflow_version: row.get(4)?,
        compilation_status: row.get(5)?,
        review_capabilities,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn workflow_blueprint_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowBlueprint> {
    let visual_state_json: String = row.get(5)?;
    let workflow_ir_json: Option<String> = row.get(6)?;
    let compilation_status: String = row.get(7)?;
    Ok(WorkflowBlueprint {
        workflow_id: row.get(0)?,
        version: row.get(1)?,
        project_id: row.get(2)?,
        name: row.get(3)?,
        description: row.get(4)?,
        visual_state: serde_json::from_str(&visual_state_json).map_err(json_from_sql_error)?,
        workflow_ir: workflow_ir_json
            .map(|json| serde_json::from_str(&json).map_err(json_from_sql_error))
            .transpose()?,
        compilation_status: parse_blueprint_compilation_status(&compilation_status)?,
        compilation_error: row.get(8)?,
        is_active: row.get::<_, i64>(9)? != 0,
        created_at_ms: row.get(10)?,
        updated_at_ms: row.get(11)?,
        compiled_at_ms: row.get(12)?,
    })
}

fn parse_blueprint_compilation_status(value: &str) -> rusqlite::Result<BlueprintCompilationStatus> {
    match value {
        "Draft" => Ok(BlueprintCompilationStatus::Draft),
        "Compiling" => Ok(BlueprintCompilationStatus::Compiling),
        "Compiled" => Ok(BlueprintCompilationStatus::Compiled),
        "Failed" => Ok(BlueprintCompilationStatus::Failed),
        other => Err(rusqlite::Error::InvalidParameterName(format!(
            "Invalid workflow blueprint compilation_status: {other}"
        ))),
    }
}

fn canonical_knowledge_sync_path(path: &str) -> rusqlite::Result<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "Knowledge sync path must not be empty.".to_string(),
        ));
    }
    let candidate = PathBuf::from(trimmed);
    let absolute = if candidate.is_absolute() {
        candidate
    } else {
        project_root().join(candidate)
    };
    let canonical = absolute.canonicalize().map_err(io_to_sql_error)?;
    if !canonical.is_dir() {
        return Err(rusqlite::Error::InvalidParameterName(
            "Knowledge sync path must be an existing directory.".to_string(),
        ));
    }
    Ok(canonical)
}

fn knowledge_sync_workflow_id(path: &str, schedule_expression: &str) -> String {
    let digest = sha256_hex(format!("knowledge-sync:{path}:{schedule_expression}").as_bytes());
    format!("knowledge-sync-{}", &digest[..16])
}

fn knowledge_sync_visual_state(path: &str, schedule_expression: &str) -> Value {
    json!({
        "nodes": [
            {
                "id": "schedule-trigger",
                "kind": "schedule",
                "label": "Schedule",
                "schedule": schedule_expression,
            },
            {
                "id": "sync-knowledge-vault",
                "kind": "mcp_tool",
                "label": "Sync Knowledge Vault",
                "serverName": "system",
                "toolName": "sync_knowledge_vault",
                "arguments": {
                    "path": path,
                    "maxFiles": 60,
                },
            },
            {
                "id": "sync-result",
                "kind": "output",
                "label": "Sync Result",
            }
        ],
        "edges": [
            {
                "id": "edge-schedule-sync",
                "sourceNodeId": "schedule-trigger",
                "sourcePort": "out",
                "targetNodeId": "sync-knowledge-vault",
            },
            {
                "id": "edge-sync-result",
                "sourceNodeId": "sync-knowledge-vault",
                "sourcePort": "out",
                "targetNodeId": "sync-result",
            }
        ],
        "metadata": {
            "createdBy": "create_scheduled_knowledge_sync",
            "path": path,
        }
    })
}

fn knowledge_sync_workflow_ir(workflow_id: &str, name: &str, path: &str) -> WorkflowIr {
    WorkflowIr {
        schema_version: WORKFLOW_IR_SCHEMA_VERSION.to_string(),
        workflow_id: workflow_id.to_string(),
        workflow_version: 1,
        name: name.to_string(),
        description: "Scheduled differential knowledge vault sync.".to_string(),
        compiler: CompilerTarget {
            model: WORKFLOW_COMPILER_MODEL.to_string(),
        },
        metadata: None,
        nodes: vec![
            WorkflowNode::Input(InputNode {
                id: "schedule-trigger".to_string(),
                label: "Schedule".to_string(),
                output_key: "workflow.input".to_string(),
                input_schema: json!({ "type": "object" }),
            }),
            WorkflowNode::McpTool(McpToolNode {
                id: "sync-knowledge-vault".to_string(),
                label: "Sync Knowledge Vault".to_string(),
                server_name: "system".to_string(),
                tool_name: "sync_knowledge_vault".to_string(),
                arguments: json!({
                    "path": path,
                    "maxFiles": 60,
                }),
                input_schema: None,
                output_schema: None,
                system_timeout_ms: Some(crate::workflow_ir::LONG_TIMEOUT_MS),
            }),
            WorkflowNode::Output(OutputNode {
                id: "sync-result".to_string(),
                label: "Sync Result".to_string(),
                input_mapping: "{{nodes.sync-knowledge-vault.output}}".to_string(),
                output_schema: json!({ "type": "object" }),
                completion_kind: WorkflowCompletionKind::Result,
            }),
        ],
        edges: vec![
            WorkflowEdge {
                id: "edge-schedule-sync".to_string(),
                source_node_id: "schedule-trigger".to_string(),
                source_port: "out".to_string(),
                target_node_id: "sync-knowledge-vault".to_string(),
                target_port: None,
            },
            WorkflowEdge {
                id: "edge-sync-result".to_string(),
                source_node_id: "sync-knowledge-vault".to_string(),
                source_port: "out".to_string(),
                target_node_id: "sync-result".to_string(),
                target_port: None,
            },
        ],
    }
}

fn select_intents(connection: &Connection) -> rusqlite::Result<Vec<IntentRecord>> {
    let mut statement = connection.prepare(
        "SELECT id, plan_id, prompt, metadata, timestamp_ms FROM intents ORDER BY id DESC LIMIT 50",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(IntentRecord {
            id: row.get(0)?,
            plan_id: row.get(1)?,
            prompt: row.get(2)?,
            metadata: row.get(3)?,
            timestamp_ms: row.get(4)?,
        })
    })?;

    rows.collect()
}

fn select_actions(connection: &Connection) -> rusqlite::Result<Vec<ActionRecord>> {
    let mut statement = connection.prepare(
        "
        SELECT id, plan_id, tool, input, output, status, timestamp_ms
        FROM actions
        ORDER BY id DESC
        LIMIT 100
        ",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(ActionRecord {
            id: row.get(0)?,
            plan_id: row.get(1)?,
            tool: row.get(2)?,
            input: row.get(3)?,
            output: row.get(4)?,
            status: row.get(5)?,
            timestamp_ms: row.get(6)?,
        })
    })?;

    rows.collect()
}

fn select_certificates(connection: &Connection) -> rusqlite::Result<Vec<CertificateRecord>> {
    let mut statement = connection.prepare(
        "
        SELECT id, plan_id, action_id, mlc_path, mlc_content, timestamp_ms
        FROM certificates
        ORDER BY id DESC
        LIMIT 50
        ",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(CertificateRecord {
            id: row.get(0)?,
            plan_id: row.get(1)?,
            action_id: row.get(2)?,
            mlc_path: row.get(3)?,
            mlc_content: row.get(4)?,
            timestamp_ms: row.get(5)?,
        })
    })?;

    rows.collect()
}

fn select_plan_generation_states(
    connection: &Connection,
) -> rusqlite::Result<Vec<PlanGenerationStateRecord>> {
    let mut statement = connection.prepare(
        "
        SELECT id, plan_id, plan_json, current_step_index, status, generated_text, timestamp_ms
        FROM plan_generation_states
        ORDER BY id DESC
        LIMIT 50
        ",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(PlanGenerationStateRecord {
            id: row.get(0)?,
            plan_id: row.get(1)?,
            plan_json: row.get(2)?,
            current_step_index: row.get(3)?,
            status: row.get(4)?,
            generated_text: row.get(5)?,
            timestamp_ms: row.get(6)?,
        })
    })?;

    rows.collect()
}

fn select_recoverable_actions(connection: &Connection) -> rusqlite::Result<Vec<RecoverableAction>> {
    let mut statement = connection.prepare(
        "
        SELECT id, plan_id, tool, input, status
        FROM actions
        WHERE status IN ('recoverable', 'failed')
        ORDER BY id DESC
        LIMIT 20
        ",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(RecoverableAction {
            action_id: row.get(0)?,
            plan_id: row.get(1)?,
            tool: row.get(2)?,
            input: row.get(3)?,
            status: row.get(4)?,
        })
    })?;

    rows.collect()
}

fn keyword_terms(input: &str) -> Vec<String> {
    let mut terms = input
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|term| !keyword_stopword(term))
        .filter(|term| term.len() >= 3 || short_keyword_whitelist(term))
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms.truncate(8);
    terms
}

fn keyword_stopword(term: &str) -> bool {
    matches!(
        term.to_ascii_lowercase().as_str(),
        "and"
            | "are"
            | "but"
            | "for"
            | "how"
            | "not"
            | "the"
            | "this"
            | "that"
            | "was"
            | "were"
            | "what"
            | "when"
            | "where"
            | "with"
            | "you"
            | "your"
    )
}

fn short_keyword_whitelist(term: &str) -> bool {
    matches!(
        term.to_ascii_lowercase().as_str(),
        "ai" | "api" | "db" | "go" | "js" | "os" | "qa" | "ui"
    )
}

fn chat_memory_relevance(block: &RelevantChatMemoryBlock, terms: &[String]) -> f32 {
    let haystack = format!("{} {}", block.role, block.content).to_lowercase();
    let mut score = 0.0f32;
    for term in terms {
        if haystack.contains(term) {
            score += 1.0;
        }
    }
    if block.role.eq_ignore_ascii_case("user") {
        score += 0.25;
    }
    score + (block.created_at_ms.max(0) as f32 / 1_000_000_000_000_000.0)
}

fn project_root() -> PathBuf {
    crate::settings::app_data_root()
}

pub fn hash_arguments(args: &Value) -> String {
    let canonical = canonicalize_json(args);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_else(|_| b"null".to_vec());
    sha256_chunks([
        b"workflow-approval-arguments-v1:".as_slice(),
        bytes.as_slice(),
    ])
    .to_hex()
}

fn workflow_version_approval_subject(workflow_id: &str, workflow_version: u32) -> String {
    format!("saved-workflow:{workflow_id}:version:{workflow_version}")
}

fn workflow_version_approval_target(server_name: &str, tool_name: &str) -> String {
    serde_json::to_string(&[server_name.trim(), tool_name.trim()])
        .expect("two Workflow approval identifiers always serialize")
}

pub fn verify_step_approval(
    connection: &Connection,
    instance_id: &str,
    node_id: &str,
    tool_name: &str,
    args: &Value,
) -> Result<bool, String> {
    let args_hash = hash_arguments(args);
    let mut stmt = connection
        .prepare(
            "
            SELECT decision, expires_at
            FROM workflow_approvals
            WHERE workflow_instance_id = ?1
              AND node_id = ?2
              AND target_tool_name = ?3
              AND arguments_hash = ?4
            ORDER BY expires_at DESC
            LIMIT 1
            ",
        )
        .map_err(|error| error.to_string())?;
    let now = unix_time_seconds();
    let result = stmt
        .query_row(params![instance_id, node_id, tool_name, args_hash], |row| {
            let decision: String = row.get(0)?;
            let expires_at: i64 = row.get(1)?;
            Ok((decision, expires_at))
        })
        .optional()
        .map_err(|error| error.to_string())?;

    Ok(matches!(
        result,
        Some((decision, expires_at)) if decision == "approve" && expires_at > now
    ))
}

#[cfg(test)]
mod tests;
