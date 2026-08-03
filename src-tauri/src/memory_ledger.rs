use crate::agent_manager::AgentManager;
use crate::db::VerifiedFilesystemContext;
use crate::db::{ChatMessageRecord, PersistenceEngine};
use crate::foundation::{clock::unix_time_ms_i64 as unix_time_ms, digest::sha256_hex};
use crate::gemma::{GemmaService, SemanticEmbedding};
use crate::security::firewall::default_workspace_id;
use crate::settings::app_data_root as project_root;
use crate::shield_gate::{
    validate_logical_certificate_for_host_access, LogicalCertificate as ShieldCertificate,
};
use crate::sovereign_identity::{SignatureBlock, SovereignIdentity};
use crate::system_diagnostics::{
    collect_operating_environment_snapshot_sync, format_operating_environment_prompt_context,
};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

#[cfg(test)]
#[path = "memory_ledger/apple_app_intent_tests.rs"]
mod apple_app_intent_tests;
mod imported_sources;
mod project_purge;
pub(crate) use imported_sources::memory_limit_for_context_budget;
pub use imported_sources::{ImportedSourceReceipt, ReadImportedSourceRequest};
#[path = "memory_ledger/session_boundary.rs"]
mod session_boundary;
pub(crate) use session_boundary::is_explicit_internal_memory_mutation;
use session_boundary::is_explicit_session_only_memory_request;

const OPS_DB_FILE: &str = "oomu_ops.db";
const PRIVATE_MEMORY_LEDGER_STORE_ID: &str = "private://memory-ledger";
const DEFAULT_AGENT_MEMORY_CONTEXT_LIMIT: usize = 10;
const MAX_AGENT_MEMORY_CONTEXT_LIMIT: usize = 500;

#[tauri::command]
pub async fn read_imported_agent_source(
    request: ReadImportedSourceRequest,
    ledger: tauri::State<'_, MemoryLedger>,
) -> Result<ImportedSourceReceipt, MemoryLedgerError> {
    imported_sources::read_imported_agent_source(request, ledger).await
}

#[derive(Clone)]
pub struct MemoryLedger {
    db_path: Arc<PathBuf>,
    write_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentSoulManifest {
    pub agent_id: String,
    pub display_name: String,
    pub origin_story: String,
    pub role: String,
    pub values: Vec<String>,
    pub hard_boundaries: Vec<String>,
    pub communication_style: String,
    pub self_description: String,
    pub immutable_truths: Vec<String>,
    pub version: i64,
    pub signature: SignatureBlock,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentMemoryEntry {
    pub id: i64,
    pub agent_id: String,
    pub memory_kind: String,
    pub scope: String,
    pub content: String,
    pub confidence: f32,
    pub source_session: String,
    pub source_turn: Option<i64>,
    pub contradicted_by: Option<i64>,
    pub visibility: String,
    pub signature: SignatureBlock,
    pub created_at_ms: i64,
    pub last_confirmed_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ImportedAgentMemoryCard {
    pub memory_kind: String,
    pub scope: String,
    pub content: String,
    pub confidence: f32,
    pub source_session: String,
    pub visibility: String,
}

#[derive(Debug, Clone)]
pub struct JournalImportFile {
    pub relative_path: String,
    pub extension: String,
    pub content: String,
    pub modified_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct UserPersonalityProfile {
    pub display_name: String,
    pub pronouns: String,
    pub role_or_work: String,
    pub location_timezone: String,
    pub bio_context: String,
    pub should_know: String,
    pub should_respond: String,
    pub areas_of_expertise: String,
    pub current_priorities: String,
    pub languages: String,
    pub interests_preferences: String,
    pub boundaries: String,
    pub default_tone: String,
    pub response_length: String,
    pub formatting_style: String,
    pub conversation_defaults: Vec<String>,
    pub signature: Option<SignatureBlock>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentIdentityContext {
    pub agent_id: String,
    pub soul: AgentSoulManifest,
    pub memories: Vec<AgentMemoryEntry>,
    pub user_profile: Option<UserPersonalityProfile>,
    pub path_context: Option<String>,
    pub prompt_context: String,
    pub secure_memory_available: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HydrateAgentContextRequest {
    pub agent_id: String,
    pub display_name: String,
    pub role: String,
    pub description: String,
    pub system_prompt: String,
    pub latest_message: String,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default, alias = "toolRegistryOffline")]
    pub tool_registry_offline: bool,
    #[serde(default, alias = "backgroundModEvent")]
    pub background_mod_event: bool,
    #[serde(default, alias = "layoutSchema")]
    pub layout_schema: Option<String>,
    #[serde(default, alias = "projectId")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub verified_filesystem_context: Option<VerifiedFilesystemContext>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaptureChatMemoriesRequest {
    pub agent_id: String,
    pub display_name: String,
    pub role: String,
    pub description: String,
    pub session_id: String,
    pub user_message: String,
    pub assistant_message: String,
    #[serde(default, alias = "projectId")]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompactSessionHistoryRequest {
    pub session_id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub max_turns: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompactSessionHistoryResponse {
    pub session_id: String,
    pub agent_id: String,
    pub analyzed_turns: usize,
    pub skipped_messages: usize,
    pub captured_memories: Vec<AgentMemoryEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentSoulManifestUpdateRequest {
    pub manifest: AgentSoulManifest,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryCertificate {
    pub premises: Vec<String>,
    pub execution_path: Vec<String>,
    pub formal_conclusion: String,
    pub signature: Option<SignatureBlock>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryProposal {
    pub insight: String,
    pub source_session: String,
    pub logical_certificate: MemoryCertificate,
    pub source_cache_id: i64,
    pub channel: Option<String>,
    pub source_message_index: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecallRequest {
    pub query: String,
    pub requester_agent: String,
    pub allowed_channels: Vec<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComparativeAuditRequest {
    pub query: String,
    pub channels: Vec<String>,
    pub minimum_recurrence: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GlobalMemoryEntry {
    pub id: i64,
    pub workspace_id: String,
    pub insight: String,
    pub source_session: String,
    pub channel: String,
    pub source_cache_id: i64,
    pub source_message_index: Option<usize>,
    pub embedding: Vec<f32>,
    pub embedding_source: String,
    pub logical_certificate: MemoryCertificate,
    pub ledger_signature: SignatureBlock,
    pub committed_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct MemoryCommitResponse {
    pub entry: GlobalMemoryEntry,
    pub validation: String,
}

#[derive(Debug, Serialize)]
pub struct RecallResponse {
    pub requester_agent: String,
    pub query: String,
    pub elapsed_ms: u128,
    pub results: Vec<RecallResult>,
}

#[derive(Debug, Serialize)]
pub struct RecallResult {
    pub entry: GlobalMemoryEntry,
    pub score: f32,
}

#[derive(Debug, Serialize)]
pub struct LedgerState {
    pub db_path: String,
    pub global_memory: Vec<GlobalMemoryEntry>,
    pub mesh_decisions: Vec<MeshDecisionEntry>,
    pub runtime_sensor_updates: Vec<RuntimeSensorUpdateEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectMemorySummaryRequest {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMemorySummary {
    pub project_id: String,
    pub memory_count: usize,
    pub source_sessions: Vec<String>,
}

#[cfg(test)]
#[test]
fn ledger_state_store_id_is_opaque() {
    let serialized = serde_json::to_string(&LedgerState {
        db_path: PRIVATE_MEMORY_LEDGER_STORE_ID.to_string(),
        global_memory: Vec::new(),
        mesh_decisions: Vec::new(),
        runtime_sensor_updates: Vec::new(),
    })
    .unwrap();
    assert!(serialized.contains("private://memory-ledger"));
    if let Some(home) = std::env::var_os("HOME") {
        assert!(!serialized.contains(&home.to_string_lossy().to_string()));
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ComparativeAuditFinding {
    pub pattern: String,
    pub recurrence: usize,
    pub source_sessions: Vec<String>,
    pub severity: String,
}

#[derive(Debug, Serialize)]
pub struct ComparativeAuditResponse {
    pub query: String,
    pub inspected_entries: usize,
    pub findings: Vec<ComparativeAuditFinding>,
    pub signature: SignatureBlock,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeshDecisionEntry {
    pub id: i64,
    pub mission_id: String,
    pub step_id: String,
    pub directive: String,
    pub status: String,
    pub node_id: String,
    pub certificate_hash: String,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSensorUpdateEntry {
    pub id: i64,
    pub mission_id: String,
    pub step_id: String,
    pub tool_executed: String,
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
    pub directive: String,
    pub payload_json: String,
    pub committed_at_ms: i64,
}

#[cfg(test)]
#[derive(Debug, Clone, Serialize)]
pub struct MissionLedgerSummary {
    pub mission_id: String,
    pub heartbeat_count: i64,
    pub decision_count: i64,
    pub mesh_event_count: i64,
    pub certificate_hashes: Vec<String>,
    pub signed_summary: SignatureBlock,
    pub metadata_block: String,
}

#[derive(Debug, Serialize)]
pub struct MemoryLedgerError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
}

impl MemoryLedger {
    pub fn initialize() -> Result<Self, String> {
        let db_path = project_root().join(OPS_DB_FILE);
        Self::initialize_at(db_path)
    }

    pub(crate) fn initialize_at(db_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let ledger = Self {
            db_path: Arc::new(db_path),
            write_lock: Arc::new(Mutex::new(())),
        };
        ledger.run_migrations().map_err(|error| error.to_string())?;
        Ok(ledger)
    }

    async fn commit(
        &self,
        proposal: MemoryProposal,
        gemma: GemmaService,
        identity: SovereignIdentity,
    ) -> Result<MemoryCommitResponse, MemoryLedgerError> {
        let ledger = self.clone();
        tauri::async_runtime::spawn_blocking(move || ledger.commit_sync(proposal, gemma, identity))
            .await
            .map_err(|error| MemoryLedgerError::runtime(error.to_string()))?
    }

    async fn recall(
        &self,
        request: RecallRequest,
        gemma: GemmaService,
        identity: SovereignIdentity,
    ) -> Result<RecallResponse, MemoryLedgerError> {
        let ledger = self.clone();
        tauri::async_runtime::spawn_blocking(move || ledger.recall_sync(request, gemma, identity))
            .await
            .map_err(|error| MemoryLedgerError::runtime(error.to_string()))?
    }

    async fn load_state(
        &self,
        identity: SovereignIdentity,
    ) -> Result<LedgerState, MemoryLedgerError> {
        let ledger = self.clone();
        tauri::async_runtime::spawn_blocking(move || ledger.select_state(&identity))
            .await
            .map_err(|error| MemoryLedgerError::runtime(error.to_string()))?
    }

    async fn comparative_audit(
        &self,
        request: ComparativeAuditRequest,
        identity: SovereignIdentity,
    ) -> Result<ComparativeAuditResponse, MemoryLedgerError> {
        let ledger = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            ledger.comparative_audit_sync(request, &identity)
        })
        .await
        .map_err(|error| MemoryLedgerError::runtime(error.to_string()))?
    }

    fn run_migrations(&self) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        let workspace_id = default_workspace_id();
        connection.execute_batch(
            &format!(
                "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS global_memory (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                workspace_id TEXT NOT NULL DEFAULT '{}',
                insight TEXT NOT NULL,
                source_session TEXT NOT NULL,
                channel TEXT NOT NULL,
                source_cache_id INTEGER NOT NULL,
                source_message_index INTEGER,
                embedding_json TEXT NOT NULL,
                embedding_source TEXT NOT NULL,
                logical_certificate TEXT NOT NULL,
                ledger_signature TEXT NOT NULL,
                committed_at_ms INTEGER NOT NULL,
                FOREIGN KEY(source_cache_id) REFERENCES grounding_cache(id)
            );

            CREATE INDEX IF NOT EXISTS idx_global_memory_channel ON global_memory(channel);
            CREATE INDEX IF NOT EXISTS idx_global_memory_source_session ON global_memory(source_session);

            CREATE TABLE IF NOT EXISTS mesh_memory_ledger (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                mission_id TEXT NOT NULL,
                step_id TEXT NOT NULL,
                directive TEXT NOT NULL,
                status TEXT NOT NULL,
                node_id TEXT NOT NULL,
                certificate_hash TEXT NOT NULL,
                committed_at_ms INTEGER NOT NULL,
                UNIQUE(mission_id, step_id, status)
            );
            CREATE INDEX IF NOT EXISTS idx_mesh_memory_mission_step ON mesh_memory_ledger(mission_id, step_id);

            CREATE TABLE IF NOT EXISTS runtime_sensor_updates (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                mission_id TEXT NOT NULL,
                step_id TEXT NOT NULL,
                tool_executed TEXT NOT NULL,
                exit_code INTEGER NOT NULL,
                stdout TEXT NOT NULL,
                stderr TEXT NOT NULL,
                directive TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                committed_at_ms INTEGER NOT NULL,
                UNIQUE(mission_id, step_id, tool_executed)
            );
            CREATE INDEX IF NOT EXISTS idx_runtime_sensor_updates_mission_step
                ON runtime_sensor_updates(mission_id, step_id);

            CREATE TABLE IF NOT EXISTS task_heartbeats (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                flow_id TEXT NOT NULL,
                step_id TEXT,
                parent_session_id TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL,
                drift_score REAL NOT NULL DEFAULT 0,
                message TEXT NOT NULL DEFAULT '',
                created_at_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_task_heartbeats_flow ON task_heartbeats(flow_id);

            CREATE TABLE IF NOT EXISTS agent_soul_manifests (
                agent_id TEXT PRIMARY KEY,
                display_name TEXT NOT NULL,
                origin_story TEXT NOT NULL,
                role TEXT NOT NULL,
                values_json TEXT NOT NULL,
                hard_boundaries_json TEXT NOT NULL,
                communication_style TEXT NOT NULL,
                self_description TEXT NOT NULL,
                immutable_truths_json TEXT NOT NULL,
                version INTEGER NOT NULL,
                signature_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS agent_memory_entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL,
                memory_kind TEXT NOT NULL,
                scope TEXT NOT NULL,
                project_id TEXT,
                content TEXT NOT NULL,
                confidence REAL NOT NULL,
                source_session TEXT NOT NULL,
                source_turn INTEGER,
                contradicted_by INTEGER,
                visibility TEXT NOT NULL,
                signature_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                last_confirmed_at_ms INTEGER,
                UNIQUE(agent_id, memory_kind, scope, content)
            );
            CREATE INDEX IF NOT EXISTS idx_agent_memory_agent_kind ON agent_memory_entries(agent_id, memory_kind);
            CREATE INDEX IF NOT EXISTS idx_agent_memory_scope ON agent_memory_entries(scope);

            CREATE TABLE IF NOT EXISTS user_personality_profile (
                id TEXT PRIMARY KEY,
                profile_json TEXT NOT NULL,
                signature_json TEXT NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            ",
                workspace_id
            ),
        )?;
        add_column_if_missing(
            &connection,
            "global_memory",
            "workspace_id",
            &format!(
                "ALTER TABLE global_memory ADD COLUMN workspace_id TEXT NOT NULL DEFAULT '{}'",
                workspace_id
            ),
        )?;
        add_column_if_missing(
            &connection,
            "agent_memory_entries",
            "project_id",
            "ALTER TABLE agent_memory_entries ADD COLUMN project_id TEXT",
        )?;
        connection.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_global_memory_workspace_channel
                ON global_memory(workspace_id, channel, id DESC);
            CREATE INDEX IF NOT EXISTS idx_agent_memory_project
                ON agent_memory_entries(project_id, agent_id, created_at_ms DESC);
            ",
        )?;
        imported_sources::migrate(&connection)
    }

    pub(crate) fn hydrate_agent_context_sync(
        &self,
        request: HydrateAgentContextRequest,
        identity: &SovereignIdentity,
    ) -> Result<AgentIdentityContext, MemoryLedgerError> {
        self.hydrate_agent_context_sync_with_memory_limit(
            request,
            DEFAULT_AGENT_MEMORY_CONTEXT_LIMIT,
            identity,
        )
    }

    pub(crate) fn hydrate_agent_context_sync_with_context_budget(
        &self,
        request: HydrateAgentContextRequest,
        context_budget_tokens: usize,
        identity: &SovereignIdentity,
    ) -> Result<AgentIdentityContext, MemoryLedgerError> {
        self.hydrate_agent_context_sync_with_memory_limit(
            request,
            memory_limit_for_context_budget(context_budget_tokens),
            identity,
        )
    }

    pub(crate) fn hydrate_agent_context_sync_with_memory_limit(
        &self,
        request: HydrateAgentContextRequest,
        memory_limit: usize,
        identity: &SovereignIdentity,
    ) -> Result<AgentIdentityContext, MemoryLedgerError> {
        let signed_context = (|| {
            let soul = self.ensure_agent_soul_manifest_sync(
                &request.agent_id,
                &request.display_name,
                &request.role,
                &request.description,
                identity,
            )?;
            let memories = self.select_agent_memories_sync(
                &request.agent_id,
                &request.latest_message,
                memory_limit,
                request.project_id.as_deref(),
                identity,
            )?;
            let user_profile = self.select_user_personality_profile_sync(identity)?;
            Ok::<_, MemoryLedgerError>((soul, memories, user_profile))
        })();
        let (soul, memories, user_profile, secure_memory_available) = match signed_context {
            Ok((soul, memories, user_profile)) => (soul, memories, user_profile, true),
            Err(error) if error.allows_identity_isolated_chat() => {
                eprintln!(
                    "CHAT_SECURE_MEMORY_ISOLATED code={} boundary={} signed_context=omitted",
                    error.code, error.boundary
                );
                (
                    ephemeral_agent_soul_manifest(
                        &request.agent_id,
                        &request.display_name,
                        &request.role,
                        &request.description,
                    )?,
                    Vec::new(),
                    None,
                    false,
                )
            }
            Err(error) => return Err(error),
        };
        let path_context = picker_authorized_conversational_path_context(
            &request.latest_message,
            request.verified_filesystem_context.as_ref(),
        );
        let operating_environment = collect_operating_environment_snapshot_sync();
        let operating_environment_context =
            format_operating_environment_prompt_context(&operating_environment);
        let system_prompt = crate::agent_manager::capability_aware_system_prompt(
            &request.system_prompt,
            request.tool_registry_offline,
        );
        let system_prompt = crate::agent_manager::inject_prescriptive_mod_layout_contract(
            &system_prompt,
            request.background_mod_event,
            request.layout_schema.as_deref(),
        );
        let mut prompt_context = format_agent_identity_prompt_context(
            &soul,
            &memories,
            user_profile.as_ref(),
            path_context.as_deref(),
            &operating_environment_context,
            &system_prompt,
            request.provider_id.as_deref(),
            request.model_id.as_deref(),
        );
        if !secure_memory_available {
            prompt_context.push_str(
                "\n\n[SECURE MEMORY STATUS]\nSecure memory is unavailable for this turn. Answer normally from the visible conversation, but do not claim to remember, save, or update profile information.",
            );
        }
        Ok(AgentIdentityContext {
            agent_id: request.agent_id,
            soul,
            memories,
            user_profile,
            path_context,
            prompt_context,
            secure_memory_available,
        })
    }

    fn select_user_personality_profile_sync(
        &self,
        identity: &SovereignIdentity,
    ) -> Result<Option<UserPersonalityProfile>, MemoryLedgerError> {
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        let row = connection
            .query_row(
                "
                SELECT profile_json, signature_json, updated_at_ms
                FROM user_personality_profile
                WHERE id = 'principal'
                ",
                [],
                |row| {
                    let profile_json: String = row.get(0)?;
                    let signature_json: String = row.get(1)?;
                    let updated_at_ms: i64 = row.get(2)?;
                    Ok((profile_json, signature_json, updated_at_ms))
                },
            )
            .optional()
            .map_err(MemoryLedgerError::database)?;
        let Some((profile_json, signature_json, updated_at_ms)) = row else {
            return Ok(None);
        };
        let signature = serde_json::from_str::<SignatureBlock>(&signature_json)
            .map_err(|error| MemoryLedgerError::integrity(&error.to_string()))?;
        let uses_current_key =
            identity
                .signature_uses_current_key(&signature)
                .map_err(|error| MemoryLedgerError {
                    code: error.code,
                    boundary: error.boundary,
                    message: error.message,
                })?;
        if !uses_current_key {
            eprintln!(
                "USER_PROFILE_QUARANTINED signer_fingerprint={} active_context=omitted",
                crate::sovereign_identity::public_key_fingerprint(&signature.public_key)
            );
            return Err(MemoryLedgerError::quarantined_identity(
                "The saved user profile belongs to a quarantined identity.",
            ));
        }
        identity
            .verify_payload(&profile_json, &signature)
            .map_err(|error| MemoryLedgerError {
                code: error.code,
                boundary: error.boundary,
                message: error.message,
            })?;
        let mut profile = serde_json::from_str::<UserPersonalityProfile>(&profile_json)
            .map_err(|error| MemoryLedgerError::integrity(&error.to_string()))?;
        profile.signature = Some(signature);
        profile.updated_at_ms = updated_at_ms;
        Ok(Some(profile))
    }

    fn save_user_personality_profile_sync(
        &self,
        mut profile: UserPersonalityProfile,
        identity: &SovereignIdentity,
    ) -> Result<UserPersonalityProfile, MemoryLedgerError> {
        profile.signature = None;
        profile.updated_at_ms = unix_time_ms();
        let profile_json = serde_json::to_string(&profile)
            .map_err(|error| MemoryLedgerError::integrity(&error.to_string()))?;
        let signature =
            identity
                .sign_payload(&profile_json)
                .map_err(|error| MemoryLedgerError {
                    code: error.code,
                    boundary: error.boundary,
                    message: error.message,
                })?;
        let _guard = self.lock_writes();
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        connection
            .execute(
                "
                INSERT INTO user_personality_profile (id, profile_json, signature_json, updated_at_ms)
                VALUES ('principal', ?1, ?2, ?3)
                ON CONFLICT(id) DO UPDATE SET
                    profile_json = excluded.profile_json,
                    signature_json = excluded.signature_json,
                    updated_at_ms = excluded.updated_at_ms
                ",
                params![profile_json, json_string(&signature), profile.updated_at_ms],
            )
            .map_err(MemoryLedgerError::database)?;
        profile.signature = Some(signature);
        Ok(profile)
    }

    fn update_agent_soul_manifest_sync(
        &self,
        mut manifest: AgentSoulManifest,
        identity: &SovereignIdentity,
    ) -> Result<AgentSoulManifest, MemoryLedgerError> {
        manifest.agent_id = guard_memory_text("agent_id", &manifest.agent_id)?;
        manifest.display_name = guard_memory_text("display_name", &manifest.display_name)?;
        if manifest.origin_story.trim().is_empty()
            || manifest.communication_style.trim().is_empty()
            || manifest.self_description.trim().is_empty()
        {
            return Err(MemoryLedgerError::invalid(
                "Soul manifest edits must keep the origin, communication style, and self-description populated.",
            ));
        }
        manifest.version += 1;
        manifest.updated_at_ms = unix_time_ms();
        let payload = soul_manifest_payload(
            &manifest.agent_id,
            &manifest.display_name,
            &manifest.origin_story,
            &manifest.role,
            &manifest.values,
            &manifest.hard_boundaries,
            &manifest.communication_style,
            &manifest.self_description,
            &manifest.immutable_truths,
            manifest.version,
        );
        manifest.signature =
            identity
                .sign_payload(&payload)
                .map_err(|error| MemoryLedgerError {
                    code: error.code,
                    boundary: error.boundary,
                    message: error.message,
                })?;

        let _guard = self.lock_writes();
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        connection
            .execute(
                "
                INSERT INTO agent_soul_manifests (
                    agent_id, display_name, origin_story, role, values_json, hard_boundaries_json,
                    communication_style, self_description, immutable_truths_json, version,
                    signature_json, created_at_ms, updated_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ON CONFLICT(agent_id) DO UPDATE SET
                    display_name = excluded.display_name,
                    origin_story = excluded.origin_story,
                    role = excluded.role,
                    values_json = excluded.values_json,
                    hard_boundaries_json = excluded.hard_boundaries_json,
                    communication_style = excluded.communication_style,
                    self_description = excluded.self_description,
                    immutable_truths_json = excluded.immutable_truths_json,
                    version = excluded.version,
                    signature_json = excluded.signature_json,
                    updated_at_ms = excluded.updated_at_ms
                ",
                params![
                    &manifest.agent_id,
                    &manifest.display_name,
                    &manifest.origin_story,
                    &manifest.role,
                    json_string(&manifest.values),
                    json_string(&manifest.hard_boundaries),
                    &manifest.communication_style,
                    &manifest.self_description,
                    json_string(&manifest.immutable_truths),
                    manifest.version,
                    json_string(&manifest.signature),
                    manifest.created_at_ms,
                    manifest.updated_at_ms,
                ],
            )
            .map_err(MemoryLedgerError::database)?;
        Ok(manifest)
    }

    pub(crate) fn capture_chat_memories_sync(
        &self,
        request: CaptureChatMemoriesRequest,
        identity: &SovereignIdentity,
    ) -> Result<Vec<AgentMemoryEntry>, MemoryLedgerError> {
        if is_explicit_session_only_memory_request(&request.user_message) {
            return Ok(Vec::new());
        }
        let mut captured = Vec::new();
        let soul = self.ensure_agent_soul_manifest_sync(
            &request.agent_id,
            &request.display_name,
            &request.role,
            &request.description,
            identity,
        )?;
        for candidate in
            extract_memory_candidates(&request.user_message, &request.assistant_message, &soul)
        {
            let scope = match request.project_id.as_deref() {
                Some(project_id) => {
                    let project_id = crate::p0_contracts::ProjectId::parse(project_id)
                        .map_err(|error| MemoryLedgerError::invalid(&error))?;
                    format!("project:{}:{}", project_id.as_str(), candidate.scope)
                }
                None => candidate.scope.clone(),
            };
            let entry = self.upsert_agent_memory_sync(
                &request.agent_id,
                &candidate.memory_kind,
                &scope,
                &candidate.content,
                candidate.confidence,
                &request.session_id,
                candidate.visibility.as_deref().unwrap_or("private"),
                identity,
            )?;
            captured.push(entry);
        }
        if let Some(display_name) = preferred_user_display_name(&request.user_message) {
            let mut profile = self
                .select_user_personality_profile_sync(identity)?
                .unwrap_or_default();
            if profile.display_name.trim() != display_name {
                profile.display_name = display_name;
                self.save_user_personality_profile_sync(profile, identity)?;
            }
        }
        Ok(captured)
    }

    pub(crate) fn import_agent_memory_cards_sync(
        &self,
        agent_id: &str,
        cards: Vec<ImportedAgentMemoryCard>,
        journal_files: Vec<JournalImportFile>,
        identity: &SovereignIdentity,
    ) -> Result<Vec<AgentMemoryEntry>, MemoryLedgerError> {
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        let (cards, sources) =
            imported_sources::prepare_changed_sources(&connection, agent_id, cards, journal_files)?;
        drop(connection);
        self.upsert_agent_memory_cards_transaction_sync(agent_id, cards, sources, identity)
    }

    fn upsert_agent_memory_cards_transaction_sync(
        &self,
        agent_id: &str,
        cards: Vec<ImportedAgentMemoryCard>,
        sources: Vec<imported_sources::ImportedSourceRecord>,
        identity: &SovereignIdentity,
    ) -> Result<Vec<AgentMemoryEntry>, MemoryLedgerError> {
        let agent_id = guard_memory_text("agent_id", agent_id)?;
        let mut prepared_cards = Vec::new();
        let mut unique_cards = std::collections::HashSet::new();

        for card in cards {
            let memory_kind = guard_memory_text("memory_kind", &card.memory_kind)?;
            let scope = guard_memory_text("scope", &card.scope)?;
            let source_session = guard_memory_text("source_session", &card.source_session)?;
            let visibility = guard_memory_text("visibility", &card.visibility)?;
            let content = card.content.trim().to_string();
            if content.is_empty() {
                continue;
            }
            let confidence = card.confidence.clamp(0.05, 1.0);
            if !unique_cards.insert((
                memory_kind.clone(),
                scope.clone(),
                content.clone(),
                source_session.clone(),
            )) {
                continue;
            }
            let payload = agent_memory_payload(
                &agent_id,
                &memory_kind,
                &scope,
                &content,
                confidence,
                &source_session,
                &visibility,
            );
            let signature = identity
                .sign_payload(&payload)
                .map_err(|error| MemoryLedgerError {
                    code: error.code,
                    boundary: error.boundary,
                    message: error.message,
                })?;
            prepared_cards.push((
                memory_kind,
                scope,
                content,
                confidence,
                source_session,
                visibility,
                signature,
            ));
        }

        if prepared_cards.is_empty() && sources.is_empty() {
            return Ok(Vec::new());
        }

        let _guard = self.lock_writes();
        let mut connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        let tx = connection
            .transaction()
            .map_err(MemoryLedgerError::database)?;
        let now = unix_time_ms();
        let mut selectors = Vec::new();

        for (memory_kind, scope, content, confidence, source_session, visibility, signature) in
            &prepared_cards
        {
            tx.execute(
                "
                INSERT INTO agent_memory_entries (
                    agent_id, memory_kind, scope, content, confidence, source_session,
                    source_turn, contradicted_by, visibility, signature_json, created_at_ms,
                    last_confirmed_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, ?7, ?8, ?9, ?9)
                ON CONFLICT(agent_id, memory_kind, scope, content) DO UPDATE SET
                    confidence = excluded.confidence,
                    source_session = excluded.source_session,
                    signature_json = excluded.signature_json,
                    visibility = excluded.visibility,
                    last_confirmed_at_ms = excluded.last_confirmed_at_ms
                ",
                params![
                    &agent_id,
                    memory_kind,
                    scope,
                    content,
                    confidence,
                    source_session,
                    visibility,
                    json_string(signature),
                    now,
                ],
            )
            .map_err(MemoryLedgerError::database)?;
            selectors.push((memory_kind.clone(), scope.clone(), content.clone()));
        }

        imported_sources::record_sources(&tx, &agent_id, &sources, now)
            .map_err(MemoryLedgerError::database)?;

        tx.commit().map_err(MemoryLedgerError::database)?;

        let mut imported = Vec::new();
        for (memory_kind, scope, content) in selectors {
            let memory = select_agent_memory_by_content(
                &connection,
                &agent_id,
                &memory_kind,
                &scope,
                &content,
            )?
            .ok_or_else(|| MemoryLedgerError::integrity("Imported memory row was not readable."))?;
            verify_agent_memory(&memory, identity)?;
            imported.push(memory);
        }

        Ok(imported)
    }

    fn compact_session_history_sync(
        &self,
        session_id: &str,
        agent_id: &str,
        display_name: &str,
        role: &str,
        description: &str,
        messages: &[ChatMessageRecord],
        max_turns: usize,
        identity: &SovereignIdentity,
    ) -> Result<CompactSessionHistoryResponse, MemoryLedgerError> {
        let soul = self.ensure_agent_soul_manifest_sync(
            agent_id,
            display_name,
            role,
            description,
            identity,
        )?;
        let compaction = compact_session_memory_candidates(messages, &soul, max_turns);
        let mut captured_memories = Vec::new();
        for candidate in compaction.candidates {
            captured_memories.push(self.upsert_agent_memory_sync(
                agent_id,
                &candidate.memory_kind,
                &candidate.scope,
                &candidate.content,
                candidate.confidence,
                session_id,
                candidate.visibility.as_deref().unwrap_or("private"),
                identity,
            )?);
        }

        Ok(CompactSessionHistoryResponse {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            analyzed_turns: compaction.analyzed_turns,
            skipped_messages: compaction.skipped_messages,
            captured_memories,
        })
    }

    fn ensure_agent_soul_manifest_sync(
        &self,
        agent_id: &str,
        display_name: &str,
        role: &str,
        description: &str,
        identity: &SovereignIdentity,
    ) -> Result<AgentSoulManifest, MemoryLedgerError> {
        let agent_id = guard_memory_text("agent_id", agent_id)?;
        let display_name = guard_memory_text("display_name", display_name)?;
        if let Some(existing) = self.select_agent_soul_manifest_sync(&agent_id, identity)? {
            return Ok(existing);
        }

        let mut manifest =
            new_agent_soul_manifest(&agent_id, &display_name, role, description, true)?;
        let payload = soul_manifest_payload(
            &manifest.agent_id,
            &manifest.display_name,
            &manifest.origin_story,
            &manifest.role,
            &manifest.values,
            &manifest.hard_boundaries,
            &manifest.communication_style,
            &manifest.self_description,
            &manifest.immutable_truths,
            manifest.version,
        );
        manifest.signature =
            identity
                .sign_payload(&payload)
                .map_err(|error| MemoryLedgerError {
                    code: error.code,
                    boundary: error.boundary,
                    message: error.message,
                })?;
        verify_soul_manifest(&manifest, identity)?;

        let _guard = self.lock_writes();
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        connection
            .execute(
                "
                INSERT INTO agent_soul_manifests (
                    agent_id, display_name, origin_story, role, values_json, hard_boundaries_json,
                    communication_style, self_description, immutable_truths_json, version,
                    signature_json, created_at_ms, updated_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ON CONFLICT(agent_id) DO NOTHING
                ",
                params![
                    &manifest.agent_id,
                    &manifest.display_name,
                    &manifest.origin_story,
                    &manifest.role,
                    json_string(&manifest.values),
                    json_string(&manifest.hard_boundaries),
                    &manifest.communication_style,
                    &manifest.self_description,
                    json_string(&manifest.immutable_truths),
                    manifest.version,
                    json_string(&manifest.signature),
                    manifest.created_at_ms,
                    manifest.updated_at_ms,
                ],
            )
            .map_err(MemoryLedgerError::database)?;
        Ok(manifest)
    }

    fn select_agent_soul_manifest_sync(
        &self,
        agent_id: &str,
        identity: &SovereignIdentity,
    ) -> Result<Option<AgentSoulManifest>, MemoryLedgerError> {
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        let manifest = connection
            .query_row(
                "
                SELECT agent_id, display_name, origin_story, role, values_json, hard_boundaries_json,
                       communication_style, self_description, immutable_truths_json, version,
                       signature_json, created_at_ms, updated_at_ms
                FROM agent_soul_manifests
                WHERE agent_id = ?1
                ",
                params![agent_id],
                agent_soul_manifest_from_row,
            )
            .optional()
            .map_err(MemoryLedgerError::database)?;
        if let Some(manifest) = manifest.as_ref() {
            let uses_current_key = identity
                .signature_uses_current_key(&manifest.signature)
                .map_err(|error| MemoryLedgerError {
                    code: error.code,
                    boundary: error.boundary,
                    message: error.message,
                })?;
            if !uses_current_key {
                eprintln!(
                    "AGENT_SOUL_QUARANTINED agent_id={} signer_fingerprint={} active_context=regenerated",
                    manifest.agent_id,
                    crate::sovereign_identity::public_key_fingerprint(
                        &manifest.signature.public_key
                    )
                );
                return Err(MemoryLedgerError::quarantined_identity(
                    "The saved agent identity belongs to a quarantined signer.",
                ));
            }
            verify_soul_manifest(manifest, identity)?;
        }
        Ok(manifest)
    }

    fn upsert_agent_memory_sync(
        &self,
        agent_id: &str,
        memory_kind: &str,
        scope: &str,
        content: &str,
        confidence: f32,
        source_session: &str,
        visibility: &str,
        identity: &SovereignIdentity,
    ) -> Result<AgentMemoryEntry, MemoryLedgerError> {
        let agent_id = guard_memory_text("agent_id", agent_id)?;
        let memory_kind = guard_memory_text("memory_kind", memory_kind)?;
        let scope = guard_memory_text("scope", scope)?;
        let project_id = project_id_from_memory_scope(&scope)?;
        let content = content.trim();
        if content.is_empty() {
            return Err(MemoryLedgerError::invalid(
                "Agent memory content is required.",
            ));
        }
        let source_session = guard_memory_text("source_session", source_session)?;
        let visibility = guard_memory_text("visibility", visibility)?;
        let confidence = confidence.clamp(0.05, 1.0);
        let now = unix_time_ms();
        let payload = agent_memory_payload(
            &agent_id,
            &memory_kind,
            &scope,
            content,
            confidence,
            &source_session,
            visibility.as_str(),
        );
        let signature = identity
            .sign_payload(&payload)
            .map_err(|error| MemoryLedgerError {
                code: error.code,
                boundary: error.boundary,
                message: error.message,
            })?;
        let candidate = AgentMemoryEntry {
            id: 0,
            agent_id: agent_id.clone(),
            memory_kind: memory_kind.clone(),
            scope: scope.clone(),
            content: content.to_string(),
            confidence,
            source_session: source_session.clone(),
            source_turn: None,
            contradicted_by: None,
            visibility: visibility.clone(),
            signature: signature.clone(),
            created_at_ms: now,
            last_confirmed_at_ms: Some(now),
        };
        verify_agent_memory(&candidate, identity)?;

        let _guard = self.lock_writes();
        let mut connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        let transaction = connection
            .transaction()
            .map_err(MemoryLedgerError::database)?;
        transaction
            .execute(
                "
                INSERT INTO agent_memory_entries (
                    agent_id, memory_kind, scope, project_id, content, confidence, source_session,
                    source_turn, contradicted_by, visibility, signature_json, created_at_ms,
                    last_confirmed_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, ?9, ?10, ?10)
                ON CONFLICT(agent_id, memory_kind, scope, content) DO UPDATE SET
                    last_confirmed_at_ms = excluded.last_confirmed_at_ms
                ",
                params![
                    &agent_id,
                    &memory_kind,
                    &scope,
                    &project_id,
                    content,
                    confidence,
                    &source_session,
                    &visibility,
                    json_string(&signature),
                    now,
                ],
            )
            .map_err(MemoryLedgerError::database)?;
        let selected =
            select_agent_memory_by_content(&transaction, &agent_id, &memory_kind, &scope, content)?
                .ok_or_else(|| {
                    MemoryLedgerError::integrity("Agent memory insert did not return a row.")
                })?;
        verify_agent_memory(&selected, identity)?;
        transaction.commit().map_err(MemoryLedgerError::database)?;
        Ok(selected)
    }

    fn select_agent_memories_sync(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
        project_id: Option<&str>,
        identity: &SovereignIdentity,
    ) -> Result<Vec<AgentMemoryEntry>, MemoryLedgerError> {
        let effective_limit = limit.min(MAX_AGENT_MEMORY_CONTEXT_LIMIT);
        if effective_limit == 0 {
            return Ok(Vec::new());
        }
        let project_id = project_id
            .map(crate::p0_contracts::ProjectId::parse)
            .transpose()
            .map_err(|error| MemoryLedgerError::invalid(&error))?
            .map(|id| id.to_string());
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        let scan_limit = effective_limit.saturating_mul(4).clamp(100, 2_000);
        let mut statement = connection
            .prepare(imported_sources::recall_sql())
            .map_err(MemoryLedgerError::database)?;
        let rows = statement
            .query_map(
                params![agent_id, scan_limit as i64, project_id],
                agent_memory_from_row,
            )
            .map_err(MemoryLedgerError::database)?;
        let mut memories = Vec::new();
        for row in rows {
            let memory = row.map_err(MemoryLedgerError::database)?;
            let uses_current_key = identity
                .signature_uses_current_key(&memory.signature)
                .map_err(|error| MemoryLedgerError {
                    code: error.code,
                    boundary: error.boundary,
                    message: error.message,
                })?;
            if !uses_current_key {
                eprintln!(
                    "AGENT_MEMORY_QUARANTINED entry_id={} signer_fingerprint={} active_context=omitted",
                    memory.id,
                    crate::sovereign_identity::public_key_fingerprint(
                        &memory.signature.public_key
                    )
                );
                return Err(MemoryLedgerError::quarantined_identity(
                    "Saved agent memory belongs to a quarantined signer.",
                ));
            }
            verify_agent_memory(&memory, identity)?;
            if !is_explicit_session_only_memory_request(&memory.content) {
                memories.push(memory);
            }
        }
        let query_terms = memory_terms(query);
        let chronology = imported_sources::chronology_preference(query);
        memories.sort_by(|left, right| {
            imported_sources::compare_memories(left, right, &query_terms, chronology)
        });
        memories.truncate(effective_limit);
        Ok(memories)
    }

    pub(crate) fn task_step_completed(
        &self,
        mission_id: &str,
        step_id: &str,
    ) -> Result<bool, MemoryLedgerError> {
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        let count: i64 = connection
            .query_row(
                "
                SELECT COUNT(*) FROM mesh_memory_ledger
                WHERE mission_id=?1 AND step_id=?2 AND status='complete'
                ",
                params![mission_id, step_id],
                |row| row.get(0),
            )
            .map_err(MemoryLedgerError::database)?;
        Ok(count > 0)
    }

    pub(crate) fn commit_task_step_completion(
        &self,
        mission_id: &str,
        step_id: &str,
        directive: &str,
        certificate_hash: &str,
    ) -> Result<(), MemoryLedgerError> {
        let _guard = self.lock_writes();
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        connection
            .execute(
                "
                INSERT INTO mesh_memory_ledger (
                    mission_id, step_id, directive, status, node_id, certificate_hash, committed_at_ms
                )
                VALUES (?1, ?2, ?3, 'complete', 'local-commander', ?4, ?5)
                ON CONFLICT(mission_id, step_id, status) DO UPDATE SET
                    directive=excluded.directive,
                    certificate_hash=excluded.certificate_hash,
                    committed_at_ms=excluded.committed_at_ms
                ",
                params![mission_id, step_id, directive, certificate_hash, unix_time_ms()],
            )
            .map_err(MemoryLedgerError::database)?;
        Ok(())
    }

    pub(crate) fn commit_runtime_sensor_update_sync(
        &self,
        mission_id: &str,
        step_id: &str,
        tool_executed: &str,
        exit_code: i32,
        stdout: &str,
        stderr: &str,
        directive: &str,
        payload_json: &str,
    ) -> Result<(), MemoryLedgerError> {
        let mission_id = guard_memory_text("mission_id", mission_id)?;
        let step_id = guard_memory_text("step_id", step_id)?;
        let tool_executed = guard_memory_text("tool_executed", tool_executed)?;
        let stdout = guard_sensor_blob("stdout", stdout)?;
        let stderr = guard_sensor_blob("stderr", stderr)?;
        let directive = guard_sensor_blob("directive", directive)?;
        let payload_json = guard_sensor_blob("payload_json", payload_json)?;
        let _guard = self.lock_writes();
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        connection
            .execute(
                "
                INSERT INTO runtime_sensor_updates (
                    mission_id, step_id, tool_executed, exit_code, stdout, stderr,
                    directive, payload_json, committed_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT(mission_id, step_id, tool_executed) DO UPDATE SET
                    exit_code=excluded.exit_code,
                    stdout=excluded.stdout,
                    stderr=excluded.stderr,
                    directive=excluded.directive,
                    payload_json=excluded.payload_json,
                    committed_at_ms=excluded.committed_at_ms
                ",
                params![
                    mission_id,
                    step_id,
                    tool_executed,
                    exit_code as i64,
                    stdout,
                    stderr,
                    directive,
                    payload_json,
                    unix_time_ms(),
                ],
            )
            .map_err(MemoryLedgerError::database)?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn select_runtime_sensor_updates_for_mission_sync(
        &self,
        mission_id: &str,
    ) -> Result<Vec<RuntimeSensorUpdateEntry>, MemoryLedgerError> {
        let mission_id = guard_memory_text("mission_id", mission_id)?;
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        select_runtime_sensor_updates_for_mission(&connection, &mission_id)
            .map_err(MemoryLedgerError::database)
    }

    #[cfg(test)]
    pub(crate) fn summarize_mission_sync(
        &self,
        mission_id: &str,
        identity: &SovereignIdentity,
    ) -> Result<MissionLedgerSummary, MemoryLedgerError> {
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        let heartbeat_count = count_mission_heartbeats(&connection, mission_id)
            .map_err(MemoryLedgerError::database)?;
        let decision_count = count_like_mission(&connection, "mesh_memory_ledger", mission_id)
            .map_err(MemoryLedgerError::database)?;
        let mesh_event_count = if table_exists(&connection, "mesh_events")? {
            count_like_mission(&connection, "mesh_events", mission_id)
                .map_err(MemoryLedgerError::database)?
        } else {
            0
        };
        let certificate_hashes = select_certificate_hashes(&connection, mission_id)
            .map_err(MemoryLedgerError::database)?;
        let payload = serde_json::json!({
            "mission_id": mission_id,
            "heartbeat_count": heartbeat_count,
            "decision_count": decision_count,
            "mesh_event_count": mesh_event_count,
            "certificate_hashes": certificate_hashes,
        })
        .to_string();
        let signed_summary =
            identity
                .sign_payload(&payload)
                .map_err(|error| MemoryLedgerError {
                    code: error.code,
                    boundary: error.boundary,
                    message: error.message,
                })?;
        let metadata_block = format!(
            "- mission_id: {mission_id}\n- heartbeat_count: {heartbeat_count}\n- decision_count: {decision_count}\n- mesh_event_count: {mesh_event_count}\n- certificate_hashes: {}\n- ledger_payload_hash: {}\n- ledger_signature: {}\n- ledger_public_key: {}\n- signed_at_ms: {}",
            if certificate_hashes.is_empty() {
                "none".to_string()
            } else {
                certificate_hashes.join(",")
            },
            signed_summary.payload_hash,
            signed_summary.signature,
            signed_summary.public_key,
            signed_summary.signed_at_ms
        );

        Ok(MissionLedgerSummary {
            mission_id: mission_id.to_string(),
            heartbeat_count,
            decision_count,
            mesh_event_count,
            certificate_hashes,
            signed_summary,
            metadata_block,
        })
    }

    fn commit_sync(
        &self,
        proposal: MemoryProposal,
        gemma: GemmaService,
        identity: SovereignIdentity,
    ) -> Result<MemoryCommitResponse, MemoryLedgerError> {
        let insight = proposal.insight.trim();
        if insight.is_empty() {
            return Err(MemoryLedgerError::invalid(
                "Memory proposal insight is required.",
            ));
        }
        if proposal.source_session.trim().is_empty() {
            return Err(MemoryLedgerError::invalid(
                "Memory proposal source_session is required.",
            ));
        }
        self.validate_certificate(&proposal.logical_certificate, &identity)?;

        let channel = proposal
            .channel
            .clone()
            .unwrap_or_else(|| "public".to_string());
        let embedding = gemma
            .embed_text_sync(insight)
            .map_err(|error| MemoryLedgerError::runtime(error.message))?;
        let _guard = self.lock_writes();
        let mut connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        let tx = connection
            .transaction()
            .map_err(MemoryLedgerError::database)?;
        if !grounding_cache_exists(&tx, proposal.source_cache_id)? {
            return Err(MemoryLedgerError::integrity(
                "Memory commit rejected: source_cache_id is not present in grounding_cache.",
            ));
        }

        let certificate_json = json_string(&proposal.logical_certificate);
        let embedding_json = json_string(&embedding.vector);
        let embedding_source = format!("{:?}", embedding.source);
        let workspace_id = default_workspace_id();
        let ledger_payload = memory_entry_payload(
            &workspace_id,
            insight,
            &proposal.source_session,
            &channel,
            proposal.source_cache_id,
            proposal.source_message_index,
            &certificate_json,
            &embedding_json,
            &embedding_source,
        );
        let ledger_signature =
            identity
                .sign_payload(&ledger_payload)
                .map_err(|error| MemoryLedgerError {
                    code: error.code,
                    boundary: error.boundary,
                    message: error.message,
                })?;

        tx.execute(
            "
            INSERT INTO global_memory (
                workspace_id, insight, source_session, channel, source_cache_id, source_message_index,
                embedding_json, embedding_source, logical_certificate, ledger_signature, committed_at_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ",
            params![
                &workspace_id,
                insight,
                &proposal.source_session,
                &channel,
                proposal.source_cache_id,
                proposal.source_message_index.map(|index| index as i64),
                &embedding_json,
                &embedding_source,
                certificate_json,
                json_string(&ledger_signature),
                unix_time_ms()
            ],
        )
        .map_err(MemoryLedgerError::database)?;
        let id = tx.last_insert_rowid();
        tx.commit().map_err(MemoryLedgerError::database)?;

        let entry = GlobalMemoryEntry {
            id,
            workspace_id,
            insight: insight.to_string(),
            source_session: proposal.source_session,
            channel,
            source_cache_id: proposal.source_cache_id,
            source_message_index: proposal.source_message_index,
            embedding: embedding.vector,
            embedding_source,
            logical_certificate: proposal.logical_certificate,
            ledger_signature,
            committed_at_ms: unix_time_ms(),
        };

        Ok(MemoryCommitResponse {
            entry,
            validation: "ACID commit complete: certificate validated and source cache exists."
                .to_string(),
        })
    }

    fn recall_sync(
        &self,
        request: RecallRequest,
        gemma: GemmaService,
        identity: SovereignIdentity,
    ) -> Result<RecallResponse, MemoryLedgerError> {
        let started = Instant::now();
        let query_embedding = gemma
            .embed_text_sync(&request.query)
            .map_err(|error| MemoryLedgerError::runtime(error.message))?;
        let entries = self.select_visible_entries(&request.allowed_channels, &identity)?;
        let mut results = entries
            .into_iter()
            .filter(|entry| entry.embedding.len() == query_embedding.dimensions)
            .map(|entry| {
                let score = cosine_similarity(&query_embedding, &entry.embedding);
                RecallResult { entry, score }
            })
            .collect::<Vec<_>>();
        results.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(request.limit.unwrap_or(8).min(25));

        Ok(RecallResponse {
            requester_agent: request.requester_agent,
            query: request.query,
            elapsed_ms: started.elapsed().as_millis(),
            results,
        })
    }

    pub(crate) fn comparative_audit_sync(
        &self,
        request: ComparativeAuditRequest,
        identity: &SovereignIdentity,
    ) -> Result<ComparativeAuditResponse, MemoryLedgerError> {
        let entries = self.select_visible_entries(&request.channels, identity)?;
        let threshold = request.minimum_recurrence.unwrap_or(3).max(2);
        let mut pattern_map = std::collections::BTreeMap::<String, Vec<&GlobalMemoryEntry>>::new();
        for entry in &entries {
            for term in audit_terms(&entry.insight) {
                pattern_map.entry(term).or_default().push(entry);
            }
        }
        let findings = pattern_map
            .into_iter()
            .filter(|(_, entries)| entries.len() >= threshold)
            .take(12)
            .map(|(pattern, entries)| ComparativeAuditFinding {
                pattern,
                recurrence: entries.len(),
                source_sessions: entries
                    .iter()
                    .map(|entry| entry.source_session.clone())
                    .collect(),
                severity: if entries.len() >= threshold + 2 {
                    "high".to_string()
                } else {
                    "medium".to_string()
                },
            })
            .collect::<Vec<_>>();
        let payload = serde_json::json!({
            "query": request.query,
            "inspected_entries": entries.len(),
            "findings": findings,
        })
        .to_string();
        let signature = identity
            .sign_payload(&payload)
            .map_err(|error| MemoryLedgerError {
                code: error.code,
                boundary: error.boundary,
                message: error.message,
            })?;
        Ok(ComparativeAuditResponse {
            query: request.query,
            inspected_entries: entries.len(),
            findings,
            signature,
        })
    }

    fn validate_certificate(
        &self,
        certificate: &MemoryCertificate,
        identity: &SovereignIdentity,
    ) -> Result<(), MemoryLedgerError> {
        let shield_certificate = ShieldCertificate {
            premises: certificate.premises.clone(),
            execution_path: certificate.execution_path.clone(),
            formal_conclusion: certificate.formal_conclusion.clone(),
            signature: certificate.signature.clone(),
        };
        validate_logical_certificate_for_host_access(
            "memory_commit",
            Some(&shield_certificate),
            identity,
        )
        .map_err(|error| MemoryLedgerError {
            code: error.code,
            boundary: error.boundary,
            message: error.message,
        })?;

        let certificate_text = format!(
            "{} {} {}",
            certificate.premises.join(" "),
            certificate.execution_path.join(" "),
            certificate.formal_conclusion
        )
        .to_lowercase();
        if !certificate_text.contains("grounding_cache") && !certificate_text.contains("cache") {
            return Err(MemoryLedgerError::integrity(
                "Memory commit rejected: certificate does not reference grounded local cache.",
            ));
        }

        Ok(())
    }

    fn select_visible_entries(
        &self,
        allowed_channels: &[String],
        identity: &SovereignIdentity,
    ) -> Result<Vec<GlobalMemoryEntry>, MemoryLedgerError> {
        let connection = self.open_connection()?;
        let workspace_id = default_workspace_id();
        let entries = select_global_memory(&connection, &workspace_id, 1000)
            .map_err(MemoryLedgerError::database)?;
        verify_entries(&entries, identity)?;
        Ok(entries
            .into_iter()
            .filter(|entry| {
                entry.channel == "public"
                    || allowed_channels
                        .iter()
                        .any(|channel| channel == &entry.channel)
            })
            .collect())
    }

    fn select_state(&self, identity: &SovereignIdentity) -> Result<LedgerState, MemoryLedgerError> {
        let connection = self
            .open_connection()
            .map_err(MemoryLedgerError::database)?;
        let workspace_id = default_workspace_id();
        let global_memory = select_global_memory(&connection, &workspace_id, 100)
            .map_err(MemoryLedgerError::database)?;
        verify_entries(&global_memory, identity)?;
        Ok(LedgerState {
            db_path: PRIVATE_MEMORY_LEDGER_STORE_ID.to_string(),
            global_memory,
            mesh_decisions: select_mesh_decisions(&connection)
                .map_err(MemoryLedgerError::database)?,
            runtime_sensor_updates: select_runtime_sensor_updates(&connection)
                .map_err(MemoryLedgerError::database)?,
        })
    }

    fn open_connection(&self) -> rusqlite::Result<Connection> {
        crate::db::open_ops_database_connection(self.db_path.as_ref())
    }

    fn lock_writes(&self) -> std::sync::MutexGuard<'_, ()> {
        self.write_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[tauri::command]
pub async fn commit_memory_proposal(
    proposal: MemoryProposal,
    ledger: tauri::State<'_, MemoryLedger>,
    gemma: tauri::State<'_, GemmaService>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<MemoryCommitResponse, MemoryLedgerError> {
    ledger
        .commit(proposal, gemma.inner().clone(), identity.inner().clone())
        .await
}

#[tauri::command]
pub async fn recall_global_memory(
    request: RecallRequest,
    ledger: tauri::State<'_, MemoryLedger>,
    gemma: tauri::State<'_, GemmaService>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<RecallResponse, MemoryLedgerError> {
    ledger
        .recall(request, gemma.inner().clone(), identity.inner().clone())
        .await
}

#[tauri::command]
pub async fn get_project_memory_summary(
    request: ProjectMemorySummaryRequest,
    ledger: tauri::State<'_, MemoryLedger>,
) -> Result<ProjectMemorySummary, MemoryLedgerError> {
    let project_id = crate::p0_contracts::ProjectId::parse(request.project_id)
        .map_err(|error| MemoryLedgerError::invalid(&error))?
        .to_string();
    let store = ledger.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = store.open_connection().map_err(MemoryLedgerError::database)?;
        let memory_count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM agent_memory_entries WHERE project_id=?1",
            params![project_id], |row| row.get(0),
        ).map_err(MemoryLedgerError::database)?;
        let mut statement = connection.prepare(
            "SELECT DISTINCT source_session FROM agent_memory_entries WHERE project_id=?1 ORDER BY created_at_ms DESC LIMIT 12"
        ).map_err(MemoryLedgerError::database)?;
        let source_sessions = statement.query_map(params![project_id], |row| row.get::<_, String>(0))
            .map_err(MemoryLedgerError::database)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(MemoryLedgerError::database)?;
        Ok(ProjectMemorySummary { project_id, memory_count: memory_count as usize, source_sessions })
    }).await.map_err(|error| MemoryLedgerError::runtime(error.to_string()))?
}

#[tauri::command]
pub async fn get_memory_ledger_state(
    ledger: tauri::State<'_, MemoryLedger>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<LedgerState, MemoryLedgerError> {
    ledger.load_state(identity.inner().clone()).await
}

#[tauri::command]
pub async fn run_memory_comparative_audit(
    request: ComparativeAuditRequest,
    ledger: tauri::State<'_, MemoryLedger>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<ComparativeAuditResponse, MemoryLedgerError> {
    ledger
        .comparative_audit(request, identity.inner().clone())
        .await
}

#[tauri::command]
pub async fn hydrate_agent_prompt_context(
    request: HydrateAgentContextRequest,
    ledger: tauri::State<'_, MemoryLedger>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<AgentIdentityContext, MemoryLedgerError> {
    let ledger = ledger.inner().clone();
    let identity = identity.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        ledger.hydrate_agent_context_sync(request, &identity)
    })
    .await
    .map_err(|error| MemoryLedgerError::runtime(error.to_string()))?
}

#[tauri::command]
pub async fn capture_agent_chat_memories(
    request: CaptureChatMemoriesRequest,
    ledger: tauri::State<'_, MemoryLedger>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<Vec<AgentMemoryEntry>, MemoryLedgerError> {
    let ledger = ledger.inner().clone();
    let identity = identity.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        ledger.capture_chat_memories_sync(request, &identity)
    })
    .await
    .map_err(|error| MemoryLedgerError::runtime(error.to_string()))?
}

#[tauri::command]
pub async fn compact_session_history(
    request: CompactSessionHistoryRequest,
    agent_manager: tauri::State<'_, AgentManager>,
    persistence: tauri::State<'_, PersistenceEngine>,
    ledger: tauri::State<'_, MemoryLedger>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<CompactSessionHistoryResponse, MemoryLedgerError> {
    let session_id = request.session_id.trim().to_string();
    if session_id.is_empty() {
        return Err(MemoryLedgerError::invalid(
            "Session history compaction requires a session_id.",
        ));
    }
    let requested_agent_id = request
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let max_turns = request.max_turns.unwrap_or(32).clamp(1, 120);
    let persistence_engine = persistence.inner().clone();

    let (session_id, agent_id, messages) = tauri::async_runtime::spawn_blocking(move || {
        let session = persistence_engine
            .select_chat_sessions()
            .map_err(MemoryLedgerError::database)?
            .into_iter()
            .find(|candidate| candidate.id == session_id)
            .ok_or_else(|| MemoryLedgerError::invalid("Chat session was not found."))?;
        let agent_id = requested_agent_id.unwrap_or(session.agent_id);
        let messages = persistence_engine
            .select_chat_messages(&session_id)
            .map_err(MemoryLedgerError::database)?;

        Ok::<(String, String, Vec<ChatMessageRecord>), MemoryLedgerError>((
            session_id, agent_id, messages,
        ))
    })
    .await
    .map_err(|error| MemoryLedgerError::runtime(error.to_string()))??;

    let agent_manager = agent_manager.inner().clone();
    let agent_config = agent_manager
        .get_active_agent_config(agent_id.clone())
        .await
        .map_err(MemoryLedgerError::runtime)?
        .ok_or_else(|| MemoryLedgerError::invalid("Active agent not found."))?;
    let personality_profile = agent_config
        .personality_profile()
        .map_err(|error| MemoryLedgerError::invalid(&error))?;
    let display_name = personality_profile.identity.display_name;
    let role = personality_profile.identity.role;
    let description = personality_profile.personality.summary;
    let ledger = ledger.inner().clone();
    let identity = identity.inner().clone();

    tauri::async_runtime::spawn_blocking(move || {
        ledger.compact_session_history_sync(
            &session_id,
            &agent_id,
            &display_name,
            &role,
            &description,
            &messages,
            max_turns,
            &identity,
        )
    })
    .await
    .map_err(|error| MemoryLedgerError::runtime(error.to_string()))?
}

#[tauri::command]
pub async fn get_user_personality_profile(
    ledger: tauri::State<'_, MemoryLedger>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<Option<UserPersonalityProfile>, MemoryLedgerError> {
    let ledger = ledger.inner().clone();
    let identity = identity.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        ledger.select_user_personality_profile_sync(&identity)
    })
    .await
    .map_err(|error| MemoryLedgerError::runtime(error.to_string()))?
}

#[tauri::command]
pub async fn save_user_personality_profile(
    profile: UserPersonalityProfile,
    ledger: tauri::State<'_, MemoryLedger>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<UserPersonalityProfile, MemoryLedgerError> {
    let ledger = ledger.inner().clone();
    let identity = identity.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        ledger.save_user_personality_profile_sync(profile, &identity)
    })
    .await
    .map_err(|error| MemoryLedgerError::runtime(error.to_string()))?
}

#[tauri::command]
pub async fn update_agent_soul_manifest(
    request: AgentSoulManifestUpdateRequest,
    ledger: tauri::State<'_, MemoryLedger>,
    identity: tauri::State<'_, SovereignIdentity>,
) -> Result<AgentSoulManifest, MemoryLedgerError> {
    let ledger = ledger.inner().clone();
    let identity = identity.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        ledger.update_agent_soul_manifest_sync(request.manifest, &identity)
    })
    .await
    .map_err(|error| MemoryLedgerError::runtime(error.to_string()))?
}

fn parse_daily_journal_file(
    journal_file: &JournalImportFile,
) -> Result<Vec<ImportedAgentMemoryCard>, MemoryLedgerError> {
    Ok(parse_daily_journal_content(
        &journal_file.content,
        &journal_file.relative_path,
        &journal_file.extension,
        journal_file.modified_at_ms,
    ))
}

fn parse_daily_journal_content(
    content: &str,
    relative_path: &str,
    extension: &str,
    modified_at_ms: Option<i64>,
) -> Vec<ImportedAgentMemoryCard> {
    let normalized_extension = extension.trim_start_matches('.').to_ascii_lowercase();
    let mut cards = if normalized_extension == "json" {
        parse_json_daily_journal(content, relative_path, modified_at_ms)
    } else {
        parse_markdown_daily_journal(content, relative_path, modified_at_ms)
    };
    dedupe_import_cards(&mut cards);
    cards
}

fn parse_markdown_daily_journal(
    content: &str,
    relative_path: &str,
    modified_at_ms: Option<i64>,
) -> Vec<ImportedAgentMemoryCard> {
    let cleaned = strip_frontmatter(content);
    let journal_date = infer_journal_date(relative_path, &cleaned, modified_at_ms);
    let mut cards = Vec::new();
    let mut current_heading = first_markdown_heading(&cleaned);
    let mut current_block = String::new();
    let mut in_code_fence = false;

    for line in cleaned.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
        }

        if !in_code_fence && trimmed.starts_with('#') {
            push_markdown_journal_block(
                &mut cards,
                &journal_date,
                relative_path,
                current_heading.as_deref(),
                &current_block,
                modified_at_ms,
            );
            current_block.clear();
            current_heading = Some(trimmed.trim_start_matches('#').trim().to_string())
                .filter(|heading| !heading.is_empty());
            continue;
        }

        if !in_code_fence && trimmed.is_empty() {
            push_markdown_journal_block(
                &mut cards,
                &journal_date,
                relative_path,
                current_heading.as_deref(),
                &current_block,
                modified_at_ms,
            );
            current_block.clear();
            continue;
        }

        if !in_code_fence && starts_markdown_list_item(trimmed) {
            push_markdown_journal_block(
                &mut cards,
                &journal_date,
                relative_path,
                current_heading.as_deref(),
                &current_block,
                modified_at_ms,
            );
            current_block = strip_markdown_list_marker(trimmed).to_string();
            continue;
        }

        if current_block.is_empty() {
            current_block.push_str(trimmed);
        } else if line.starts_with(' ') || line.starts_with('\t') {
            current_block.push('\n');
            current_block.push_str(line.trim_end());
        } else {
            current_block.push(' ');
            current_block.push_str(trimmed);
        }
    }

    push_markdown_journal_block(
        &mut cards,
        &journal_date,
        relative_path,
        current_heading.as_deref(),
        &current_block,
        modified_at_ms,
    );

    if cards.is_empty() {
        let fallback = cleaned.trim();
        if !fallback.is_empty() {
            cards.push(journal_card(
                &journal_date,
                None,
                relative_path,
                fallback,
                modified_at_ms,
            ));
        }
    }

    cards
}

fn parse_json_daily_journal(
    content: &str,
    relative_path: &str,
    modified_at_ms: Option<i64>,
) -> Vec<ImportedAgentMemoryCard> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return parse_markdown_daily_journal(content, relative_path, modified_at_ms);
    };
    let fallback_date = infer_journal_date(relative_path, content, modified_at_ms);
    let mut cards = Vec::new();
    collect_json_journal_cards(
        &value,
        relative_path,
        &fallback_date,
        modified_at_ms,
        &mut cards,
    );
    cards
}

fn collect_json_journal_cards(
    value: &serde_json::Value,
    relative_path: &str,
    fallback_date: &str,
    modified_at_ms: Option<i64>,
    cards: &mut Vec<ImportedAgentMemoryCard>,
) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_json_journal_cards(
                    item,
                    relative_path,
                    fallback_date,
                    modified_at_ms,
                    cards,
                );
            }
        }
        serde_json::Value::Object(object) => {
            let date = object
                .get("date")
                .or_else(|| object.get("day"))
                .or_else(|| object.get("created_at"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(fallback_date);
            let heading = object
                .get("title")
                .or_else(|| object.get("heading"))
                .or_else(|| object.get("topic"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let text = object
                .get("content")
                .or_else(|| object.get("text"))
                .or_else(|| object.get("entry"))
                .or_else(|| object.get("memory"))
                .or_else(|| object.get("insight"))
                .or_else(|| object.get("summary"))
                .or_else(|| object.get("body"))
                .and_then(json_value_to_memory_text);

            if let Some(text) = text.filter(|value| !value.trim().is_empty()) {
                cards.push(journal_card(
                    date,
                    heading,
                    relative_path,
                    &text,
                    modified_at_ms,
                ));
                return;
            }

            let mut lines = Vec::new();
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "date" | "day" | "created_at" | "title" | "heading" | "topic"
                ) {
                    continue;
                }
                if let Some(text) = json_value_to_memory_text(value) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        lines.push(format!("{key}: {trimmed}"));
                    }
                }
            }
            if !lines.is_empty() {
                cards.push(journal_card(
                    date,
                    heading,
                    relative_path,
                    &lines.join("\n"),
                    modified_at_ms,
                ));
            }
        }
        serde_json::Value::String(text) => {
            if !text.trim().is_empty() {
                cards.push(journal_card(
                    fallback_date,
                    None,
                    relative_path,
                    text,
                    modified_at_ms,
                ));
            }
        }
        _ => {
            if let Some(text) = json_value_to_memory_text(value) {
                cards.push(journal_card(
                    fallback_date,
                    None,
                    relative_path,
                    &text,
                    modified_at_ms,
                ));
            }
        }
    }
}

fn json_value_to_memory_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Array(items) => {
            let lines: Vec<String> = items
                .iter()
                .filter_map(json_value_to_memory_text)
                .filter(|value| !value.trim().is_empty())
                .collect();
            (!lines.is_empty()).then(|| lines.join("\n"))
        }
        serde_json::Value::Object(object) => {
            let lines: Vec<String> = object
                .iter()
                .filter_map(|(key, value)| {
                    json_value_to_memory_text(value).and_then(|text| {
                        let trimmed = text.trim();
                        (!trimmed.is_empty()).then(|| format!("{key}: {trimmed}"))
                    })
                })
                .collect();
            (!lines.is_empty()).then(|| lines.join("\n"))
        }
        serde_json::Value::Null => None,
    }
}

fn strip_frontmatter(content: &str) -> String {
    let mut lines = content.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    if first.trim() != "---" {
        return content.to_string();
    }
    for line in &mut lines {
        if line.trim() == "---" {
            return lines.collect::<Vec<_>>().join("\n");
        }
    }
    content.to_string()
}

fn first_markdown_heading(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .starts_with('#')
            .then(|| trimmed.trim_start_matches('#').trim().to_string())
            .filter(|heading| !heading.is_empty())
    })
}

fn starts_markdown_list_item(trimmed: &str) -> bool {
    trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed
            .split_once(". ")
            .and_then(|(prefix, _)| {
                (!prefix.is_empty() && prefix.chars().all(|value| value.is_ascii_digit()))
                    .then_some(())
            })
            .is_some()
}

fn strip_markdown_list_marker(trimmed: &str) -> &str {
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        return trimmed[2..].trim();
    }
    if let Some((prefix, rest)) = trimmed.split_once(". ") {
        if !prefix.is_empty() && prefix.chars().all(|value| value.is_ascii_digit()) {
            return rest.trim();
        }
    }
    trimmed
}

fn push_markdown_journal_block(
    cards: &mut Vec<ImportedAgentMemoryCard>,
    journal_date: &str,
    relative_path: &str,
    heading: Option<&str>,
    block: &str,
    modified_at_ms: Option<i64>,
) {
    let cleaned = block.trim();
    if cleaned.is_empty() {
        return;
    }
    cards.push(journal_card(
        journal_date,
        heading,
        relative_path,
        cleaned,
        modified_at_ms,
    ));
}

fn journal_card(
    journal_date: &str,
    heading: Option<&str>,
    relative_path: &str,
    body: &str,
    modified_at_ms: Option<i64>,
) -> ImportedAgentMemoryCard {
    let heading = heading
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| *value != journal_date);
    let date_label = if journal_date.trim().is_empty() {
        "undated"
    } else {
        journal_date.trim()
    };
    let mut content_lines = vec![
        format!("Journal date: {date_label}"),
        format!("Source file: {}", relative_path.replace('\\', "/")),
    ];
    if let Some(modified_at_ms) = modified_at_ms {
        content_lines.push(format!("Source modified_at_ms: {modified_at_ms}"));
    }
    if let Some(heading) = heading {
        content_lines.push(format!("Section: {heading}"));
    }
    content_lines.push(format!("Entry: {}", body.trim()));

    ImportedAgentMemoryCard {
        memory_kind: "daily_journal".to_string(),
        scope: format!("journal:{}", compact_scope_component(date_label)),
        content: content_lines.join("\n"),
        confidence: 0.86,
        source_session: journal_source_session(relative_path),
        visibility: "private".to_string(),
    }
}

fn infer_journal_date(relative_path: &str, content: &str, modified_at_ms: Option<i64>) -> String {
    let haystacks = [relative_path, content];
    let separated = Regex::new(r"(?P<y>19\d{2}|20\d{2})[-_./ ](?P<m>\d{1,2})[-_./ ](?P<d>\d{1,2})")
        .expect("date regex compiles");
    for haystack in haystacks {
        if let Some(captures) = separated.captures(haystack) {
            if let Some(date) = normalized_date_from_captures(&captures) {
                return date;
            }
        }
    }

    let compact = Regex::new(r"(?P<y>19\d{2}|20\d{2})(?P<m>\d{2})(?P<d>\d{2})")
        .expect("compact date regex compiles");
    for haystack in haystacks {
        if let Some(captures) = compact.captures(haystack) {
            if let Some(date) = normalized_date_from_captures(&captures) {
                return date;
            }
        }
    }

    if let Some(stem) = Path::new(relative_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return stem.to_string();
    }

    modified_at_ms
        .map(|value| format!("mtime-{value}"))
        .unwrap_or_else(|| "undated".to_string())
}

fn normalized_date_from_captures(captures: &regex::Captures<'_>) -> Option<String> {
    let year = captures.name("y")?.as_str().parse::<i32>().ok()?;
    let month = captures.name("m")?.as_str().parse::<u32>().ok()?;
    let day = captures.name("d")?.as_str().parse::<u32>().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

fn compact_scope_component(value: &str) -> String {
    let compact: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let compact = compact.trim_matches('_');
    if compact.is_empty() {
        "undated".to_string()
    } else if compact.len() <= 220 {
        compact.to_string()
    } else {
        let prefix: String = compact.chars().take(200).collect();
        format!("{}-{}", prefix, sha256_hex(compact.as_bytes()))
    }
}

fn journal_source_session(relative_path: &str) -> String {
    let normalized = relative_path.replace('\\', "/");
    let base = format!("journal_import:{normalized}");
    if base.len() <= 256 {
        return base;
    }
    let digest = sha256_hex(normalized.as_bytes());
    let suffix = Path::new(&normalized)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("journal");
    format!("journal_import:{digest}:{suffix}")
}

fn dedupe_import_cards(cards: &mut Vec<ImportedAgentMemoryCard>) {
    let mut seen = std::collections::HashSet::new();
    cards.retain(|card| {
        seen.insert((
            card.memory_kind.clone(),
            card.scope.clone(),
            card.content.clone(),
            card.source_session.clone(),
        ))
    });
}

fn grounding_cache_exists(
    transaction: &Transaction<'_>,
    cache_id: i64,
) -> Result<bool, MemoryLedgerError> {
    let mut statement = transaction
        .prepare("SELECT COUNT(*) FROM grounding_cache WHERE id = ?1")
        .map_err(MemoryLedgerError::database)?;
    let count: i64 = statement
        .query_row(params![cache_id], |row| row.get(0))
        .map_err(MemoryLedgerError::database)?;
    Ok(count > 0)
}

fn select_global_memory(
    connection: &Connection,
    workspace_id: &str,
    limit: usize,
) -> rusqlite::Result<Vec<GlobalMemoryEntry>> {
    let mut statement = connection.prepare(
        "
        SELECT id, workspace_id, insight, source_session, channel, source_cache_id, source_message_index,
               embedding_json, embedding_source, logical_certificate, ledger_signature, committed_at_ms
        FROM global_memory
        WHERE workspace_id = ?1
        ORDER BY id DESC
        LIMIT ?2
        ",
    )?;
    let rows = statement.query_map(params![workspace_id, limit as i64], |row| {
        let embedding_json: String = row.get(7)?;
        let certificate_json: String = row.get(9)?;
        let ledger_signature_json: String = row.get(10)?;
        let embedding = serde_json::from_str(&embedding_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let logical_certificate = serde_json::from_str(&certificate_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let ledger_signature = serde_json::from_str(&ledger_signature_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                10,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        Ok(GlobalMemoryEntry {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            insight: row.get(2)?,
            source_session: row.get(3)?,
            channel: row.get(4)?,
            source_cache_id: row.get(5)?,
            source_message_index: row.get::<_, Option<i64>>(6)?.map(|index| index as usize),
            embedding,
            embedding_source: row.get(8)?,
            logical_certificate,
            ledger_signature,
            committed_at_ms: row.get(11)?,
        })
    })?;

    rows.collect()
}

fn select_mesh_decisions(connection: &Connection) -> rusqlite::Result<Vec<MeshDecisionEntry>> {
    let mut statement = connection.prepare(
        "
        SELECT id, mission_id, step_id, directive, status, node_id, certificate_hash, committed_at_ms
        FROM mesh_memory_ledger
        ORDER BY id DESC
        LIMIT 100
        ",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(MeshDecisionEntry {
            id: row.get(0)?,
            mission_id: row.get(1)?,
            step_id: row.get(2)?,
            directive: row.get(3)?,
            status: row.get(4)?,
            node_id: row.get(5)?,
            certificate_hash: row.get(6)?,
            committed_at_ms: row.get(7)?,
        })
    })?;
    rows.collect()
}

fn select_runtime_sensor_updates(
    connection: &Connection,
) -> rusqlite::Result<Vec<RuntimeSensorUpdateEntry>> {
    let mut statement = connection.prepare(
        "
        SELECT id, mission_id, step_id, tool_executed, exit_code, stdout, stderr,
               directive, payload_json, committed_at_ms
        FROM runtime_sensor_updates
        ORDER BY id DESC
        LIMIT 100
        ",
    )?;
    let rows = statement.query_map([], runtime_sensor_update_from_row)?;
    rows.collect()
}

#[cfg(test)]
fn select_runtime_sensor_updates_for_mission(
    connection: &Connection,
    mission_id: &str,
) -> rusqlite::Result<Vec<RuntimeSensorUpdateEntry>> {
    let mut statement = connection.prepare(
        "
        SELECT id, mission_id, step_id, tool_executed, exit_code, stdout, stderr,
               directive, payload_json, committed_at_ms
        FROM runtime_sensor_updates
        WHERE mission_id = ?1
        ORDER BY id ASC
        ",
    )?;
    let rows = statement.query_map(params![mission_id], runtime_sensor_update_from_row)?;
    rows.collect()
}

fn runtime_sensor_update_from_row(row: &Row<'_>) -> rusqlite::Result<RuntimeSensorUpdateEntry> {
    Ok(RuntimeSensorUpdateEntry {
        id: row.get(0)?,
        mission_id: row.get(1)?,
        step_id: row.get(2)?,
        tool_executed: row.get(3)?,
        exit_code: row.get(4)?,
        stdout: row.get(5)?,
        stderr: row.get(6)?,
        directive: row.get(7)?,
        payload_json: row.get(8)?,
        committed_at_ms: row.get(9)?,
    })
}

#[cfg(test)]
fn count_like_mission(
    connection: &Connection,
    table: &str,
    mission_id: &str,
) -> rusqlite::Result<i64> {
    let column = match table {
        "task_heartbeats" | "taskflow_heartbeats" => "flow_id",
        "mesh_memory_ledger" => "mission_id",
        "mesh_events" => "detail",
        _ => return Ok(0),
    };
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE {column} LIKE ?1");
    connection.query_row(&sql, params![format!("%{mission_id}%")], |row| row.get(0))
}

#[cfg(test)]
fn count_mission_heartbeats(connection: &Connection, mission_id: &str) -> rusqlite::Result<i64> {
    let mut count = 0;
    if table_exists(connection, "taskflow_heartbeats")? {
        count += count_like_mission(connection, "taskflow_heartbeats", mission_id)?;
    }
    if table_exists(connection, "task_heartbeats")? {
        count += count_like_mission(connection, "task_heartbeats", mission_id)?;
    }
    Ok(count)
}

#[cfg(test)]
fn select_certificate_hashes(
    connection: &Connection,
    mission_id: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut hashes = Vec::new();
    if table_exists(connection, "mesh_memory_ledger")? {
        let mut statement = connection.prepare(
            "
            SELECT DISTINCT certificate_hash
            FROM mesh_memory_ledger
            WHERE mission_id = ?1 AND certificate_hash != ''
            ORDER BY committed_at_ms DESC
            LIMIT 25
            ",
        )?;
        let rows = statement.query_map(params![mission_id], |row| row.get::<_, String>(0))?;
        for row in rows {
            hashes.push(row?);
        }
    }
    if table_exists(connection, "taskflow_steps")? {
        let mut statement = connection.prepare(
            "
            SELECT logical_certificate
            FROM taskflow_steps
            WHERE flow_id LIKE ?1 AND logical_certificate IS NOT NULL
            LIMIT 25
            ",
        )?;
        let rows = statement.query_map(params![format!("%{mission_id}%")], |row| {
            row.get::<_, String>(0)
        })?;
        for row in rows {
            hashes.push(sha256_hex(row?.as_bytes()));
        }
    }
    hashes.sort();
    hashes.dedup();
    Ok(hashes)
}

#[cfg(test)]
fn table_exists(connection: &Connection, table: &str) -> rusqlite::Result<bool> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        params![table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> rusqlite::Result<()> {
    if !column_exists(connection, table, column)? {
        connection.execute_batch(alter_sql)?;
    }
    Ok(())
}

fn column_exists(connection: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn memory_entry_payload(
    workspace_id: &str,
    insight: &str,
    source_session: &str,
    channel: &str,
    source_cache_id: i64,
    source_message_index: Option<usize>,
    certificate_json: &str,
    embedding_json: &str,
    embedding_source: &str,
) -> String {
    serde_json::json!({
        "workspace_id": workspace_id,
        "insight": insight,
        "source_session": source_session,
        "channel": channel,
        "source_cache_id": source_cache_id,
        "source_message_index": source_message_index,
        "logical_certificate": certificate_json,
        "embedding_json": embedding_json,
        "embedding_source": embedding_source
    })
    .to_string()
}

fn legacy_memory_entry_payload(
    insight: &str,
    source_session: &str,
    channel: &str,
    source_cache_id: i64,
    source_message_index: Option<usize>,
    certificate_json: &str,
) -> String {
    serde_json::json!({
        "insight": insight,
        "source_session": source_session,
        "channel": channel,
        "source_cache_id": source_cache_id,
        "source_message_index": source_message_index,
        "logical_certificate": certificate_json
    })
    .to_string()
}

fn verify_entries(
    entries: &[GlobalMemoryEntry],
    identity: &SovereignIdentity,
) -> Result<(), MemoryLedgerError> {
    for entry in entries {
        let certificate_json = json_string(&entry.logical_certificate);
        let payload = memory_entry_payload(
            &entry.workspace_id,
            &entry.insight,
            &entry.source_session,
            &entry.channel,
            entry.source_cache_id,
            entry.source_message_index,
            &certificate_json,
            &json_string(&entry.embedding),
            &entry.embedding_source,
        );
        if let Err(primary_error) = identity.verify_payload(&payload, &entry.ledger_signature) {
            let legacy_payload = legacy_memory_entry_payload(
                &entry.insight,
                &entry.source_session,
                &entry.channel,
                entry.source_cache_id,
                entry.source_message_index,
                &certificate_json,
            );
            identity
                .verify_payload(&legacy_payload, &entry.ledger_signature)
                .map_err(|_| MemoryLedgerError {
                    code: primary_error.code,
                    boundary: primary_error.boundary,
                    message: primary_error.message,
                })?;
        }
    }

    Ok(())
}

fn cosine_similarity(query: &SemanticEmbedding, candidate: &[f32]) -> f32 {
    query
        .vector
        .iter()
        .zip(candidate.iter())
        .map(|(left, right)| left * right)
        .sum()
}

fn audit_terms(input: &str) -> Vec<String> {
    input
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::to_lowercase)
        .filter(|term| {
            term.len() > 5
                && !matches!(
                    term.as_str(),
                    "source" | "session" | "memory" | "verified" | "grounded"
                )
        })
        .collect()
}

#[derive(Debug)]
struct MemoryCandidate {
    memory_kind: String,
    scope: String,
    content: String,
    confidence: f32,
    visibility: Option<String>,
}

#[derive(Debug)]
struct SessionMemoryCompaction {
    analyzed_turns: usize,
    skipped_messages: usize,
    candidates: Vec<MemoryCandidate>,
}

fn compact_session_memory_candidates(
    messages: &[ChatMessageRecord],
    soul: &AgentSoulManifest,
    max_turns: usize,
) -> SessionMemoryCompaction {
    let mut pairs = Vec::new();
    let mut pending_user: Option<String> = None;
    let mut structurally_skipped = 0usize;

    for message in messages {
        if message.role.eq_ignore_ascii_case("user") {
            if pending_user.is_some() {
                structurally_skipped += 1;
            }
            pending_user = Some(message.content.clone());
        } else if message.role.eq_ignore_ascii_case("assistant") {
            if let Some(user_message) = pending_user.take() {
                pairs.push((user_message, message.content.clone()));
            } else {
                structurally_skipped += 1;
            }
        } else {
            structurally_skipped += 1;
        }
    }

    if pending_user.is_some() {
        structurally_skipped += 1;
    }

    let max_turns = max_turns.max(1);
    let omitted_pairs = pairs.len().saturating_sub(max_turns);
    let analyzed_pairs = pairs.into_iter().skip(omitted_pairs);
    let mut analyzed_turns = 0usize;
    let mut candidates = Vec::new();

    for (user_message, assistant_message) in analyzed_pairs {
        analyzed_turns += 1;
        candidates.extend(extract_memory_candidates(
            &user_message,
            &assistant_message,
            soul,
        ));
    }

    SessionMemoryCompaction {
        analyzed_turns,
        skipped_messages: structurally_skipped + omitted_pairs.saturating_mul(2),
        candidates: dedupe_memory_candidates(candidates),
    }
}

fn format_agent_identity_prompt_context(
    soul: &AgentSoulManifest,
    memories: &[AgentMemoryEntry],
    user_profile: Option<&UserPersonalityProfile>,
    path_context: Option<&str>,
    operating_environment_context: &str,
    system_prompt: &str,
    provider_id: Option<&str>,
    model_id: Option<&str>,
) -> String {
    let runtime = match (
        provider_id.map(str::trim).filter(|value| !value.is_empty()),
        model_id.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        (Some(provider), Some(model)) => format!(
            "Runtime Model Route\nprovider_id: {provider}\nmodel_id: {model}\nUse this only when explicitly asked about the runtime model or provider.\n\n"
        ),
        _ => String::new(),
    };
    let values = soul
        .values
        .iter()
        .map(|value| format!("- {value}"))
        .collect::<Vec<_>>()
        .join("\n");
    let truths = soul
        .immutable_truths
        .iter()
        .map(|truth| format!("- {truth}"))
        .collect::<Vec<_>>()
        .join("\n");
    let boundaries = soul
        .hard_boundaries
        .iter()
        .map(|boundary| format!("- {boundary}"))
        .collect::<Vec<_>>()
        .join("\n");
    let memory_block = if memories.is_empty() {
        "- No durable memories matched this turn yet.".to_string()
    } else {
        memories
            .iter()
            .map(|memory| {
                format!(
                    "- [{} / {} / confidence {:.2}] {}",
                    memory.memory_kind, memory.scope, memory.confidence, memory.content
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let user_profile_block = user_profile
        .map(format_user_personality_prompt_context)
        .unwrap_or_else(|| "- No saved user personality profile is available yet.".to_string());
    let path_context_block = path_context
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("- No safe conversational file or directory paths were resolved for this turn.");
    format!(
        "{runtime}Identity Persistence Contract\nYou are speaking as {}, the OOMU agent described below. OOMU has already retrieved your SQLite-backed soul manifest, durable memories, the user's saved personality profile, safe conversational file context, and active local operating environment snapshot for this turn and injected them into this prompt. Treat these records as your available long-term and workspace context. OOMU can persist useful preferences, relationship notes, and agent self-updates only through its signed native post-turn memory write. Never claim that a preference or profile was saved, updated, stored, or remembered unless the native response includes a signed memory receipt; during generation, acknowledge the preference without claiming persistence. Do not say you only have temporary session memory unless the available context explicitly says persistence is disabled. Do not describe yourself as a generic autonomous agent.\n\nAgent Soul Manifest\nName: {}\nRole: {}\nOrigin: {}\nSelf-description: {}\nCommunication style: {}\n\nImmutable Truths\n{}\n\nValues\n{}\n\nHard Boundaries\n{}\n\nUser Personality Profile\n{}\n\nDurable Memory Context\n{}\n\nConversational Path Context\n{}\n\nOperating Environment Context\n{}\n\nOperating Instructions\n{}\n\nCheck the user personality profile on every turn. Use it to personalize defaults and relationship context without overexposing private details. Use durable memories as context, not as unquestionable fact. If a memory seems stale or contradicted, say so and adapt. Use the operating environment snapshot to avoid blind assumptions about local ports, open editor context, Git branch state, and compiler activity, while treating it as point-in-time data that may need tool verification before mutation. If the user corrects your tone, behavior, name, preferences, or relationship style, acknowledge it naturally; the native post-turn persistence boundary decides whether it was durably stored. If safe conversational file context is present, use it directly to answer the user's latest request without pretending you opened paths outside OOMU's quarantine.",
        soul.display_name,
        soul.display_name,
        soul.role,
        soul.origin_story,
        soul.self_description,
        soul.communication_style,
        truths,
        values,
        boundaries,
        user_profile_block,
        memory_block,
        path_context_block,
        operating_environment_context.trim(),
        system_prompt.trim()
    )
}

fn picker_authorized_conversational_path_context(
    message: &str,
    verified: Option<&VerifiedFilesystemContext>,
) -> Option<String> {
    let normalized = message.to_ascii_lowercase();
    let references_verified_target = [
        "that folder",
        "this folder",
        "that directory",
        "this directory",
        "same folder",
        "same directory",
        " in there",
        " into there",
        " to there",
    ]
    .iter()
    .any(|phrase| normalized.contains(phrase));
    let verified = verified.filter(|context| {
        references_verified_target
            && context.target_kind == "directory"
            && context.result_status == "completed"
    })?;
    Some(format!(
        "Native-verified same-session directory reference: {}\nSource turn: {}\nReceipt digest: {}\nThis identifies a candidate target only. It grants no read or write permission; any action still requires exact-path validation and Shield approval.",
        verified.canonical_path, verified.source_turn_id, verified.verified_receipt_digest
    ))
}

pub(crate) fn format_user_personality_prompt_context(profile: &UserPersonalityProfile) -> String {
    let defaults = if profile.conversation_defaults.is_empty() {
        "- No conversation defaults selected.".to_string()
    } else {
        profile
            .conversation_defaults
            .iter()
            .filter(|item| !item.trim().is_empty())
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    [
        format!("Display name: {}", profile.display_name.trim()),
        format!("Pronouns: {}", profile.pronouns.trim()),
        format!("Role/work: {}", profile.role_or_work.trim()),
        format!("Location/timezone: {}", profile.location_timezone.trim()),
        format!("Bio/context: {}", profile.bio_context.trim()),
        format!("What to know: {}", profile.should_know.trim()),
        format!("How to respond: {}", profile.should_respond.trim()),
        format!("Expertise: {}", profile.areas_of_expertise.trim()),
        format!("Current priorities: {}", profile.current_priorities.trim()),
        format!("Languages: {}", profile.languages.trim()),
        format!(
            "Interests/preferences: {}",
            profile.interests_preferences.trim()
        ),
        format!("Boundaries: {}", profile.boundaries.trim()),
        format!("Default tone: {}", profile.default_tone.trim()),
        format!("Response length: {}", profile.response_length.trim()),
        format!("Formatting style: {}", profile.formatting_style.trim()),
        format!("Conversation defaults:\n{defaults}"),
    ]
    .into_iter()
    .filter(|line| {
        let (_, value) = line.split_once(':').unwrap_or(("", line));
        !value.trim().is_empty()
    })
    .collect::<Vec<_>>()
    .join("\n")
}

fn extract_memory_candidates(
    user_message: &str,
    assistant_message: &str,
    soul: &AgentSoulManifest,
) -> Vec<MemoryCandidate> {
    let mut candidates = Vec::new();
    let user = user_message.trim();
    let assistant = assistant_message.trim();
    if is_explicit_session_only_memory_request(user) {
        return Vec::new();
    }
    let lower = user.to_lowercase();
    let preferred_display_name = preferred_user_display_name(user);
    let explicit_memory_mutation = is_explicit_internal_memory_mutation(user);

    if let Some(display_name) = preferred_display_name.as_deref() {
        candidates.push(MemoryCandidate {
            memory_kind: "user_profile".to_string(),
            scope: "identity_or_goal".to_string(),
            content: format!("Call me {display_name}"),
            confidence: 0.95,
            visibility: Some("private".to_string()),
        });
    }

    if explicit_memory_mutation && preferred_display_name.is_none() {
        candidates.push(MemoryCandidate {
            memory_kind: "user_profile".to_string(),
            scope: "identity_or_goal".to_string(),
            content: truncate_memory_content(user),
            confidence: 0.9,
            visibility: Some("private".to_string()),
        });
    }

    for marker in [
        "i prefer ",
        "i like ",
        "i want ",
        "i would like ",
        "i'd like ",
        "from now on ",
        "going forward ",
    ] {
        if marker == "from now on " && preferred_display_name.is_some() {
            continue;
        }
        if let Some(index) = lower.find(marker) {
            let content = user[index..].trim();
            if useful_memory_content(content) {
                candidates.push(MemoryCandidate {
                    memory_kind: "user_profile".to_string(),
                    scope: if marker.contains("prefer")
                        || marker.contains("like")
                        || marker.contains("from now on")
                        || marker.contains("going forward")
                    {
                        "preference".to_string()
                    } else {
                        "identity_or_goal".to_string()
                    },
                    content: truncate_memory_content(content),
                    confidence: 0.82,
                    visibility: Some("private".to_string()),
                });
            }
        }
    }

    if looks_like_user_style_instruction(&lower) && useful_memory_content(user) {
        candidates.push(MemoryCandidate {
            memory_kind: "relationship_notes".to_string(),
            scope: "communication_style".to_string(),
            content: truncate_memory_content(user),
            confidence: 0.88,
            visibility: Some("private".to_string()),
        });
    }

    if looks_like_agent_self_update(&lower)
        && (explicit_memory_mutation || !looks_like_memory_retrieval_question(&lower))
        && useful_memory_content(user)
    {
        candidates.push(MemoryCandidate {
            memory_kind: "agent_self".to_string(),
            scope: "identity_or_behavior_update".to_string(),
            content: truncate_memory_content(&format!(
                "{} should treat this as a standing self-instruction: {}",
                soul.display_name, user
            )),
            confidence: 0.84,
            visibility: Some("private".to_string()),
        });
    }

    for marker in ["we decided ", "decision:", "the plan is ", "going forward "] {
        if let Some(index) = lower.find(marker) {
            let content = user[index..].trim();
            if useful_memory_content(content) {
                candidates.push(MemoryCandidate {
                    memory_kind: "project_context".to_string(),
                    scope: "decision".to_string(),
                    content: truncate_memory_content(content),
                    confidence: 0.78,
                    visibility: Some("private".to_string()),
                });
            }
        }
    }

    if lower.contains("don't ") || lower.contains("do not ") || lower.contains("never ") {
        candidates.push(MemoryCandidate {
            memory_kind: "relationship_notes".to_string(),
            scope: "boundary_or_style".to_string(),
            content: truncate_memory_content(user),
            confidence: 0.74,
            visibility: Some("private".to_string()),
        });
    }

    if assistant.len() > 80
        && (assistant.contains("I'll remember")
            || assistant.contains("I will remember")
            || assistant.contains("going forward"))
    {
        candidates.push(MemoryCandidate {
            memory_kind: "agent_self".to_string(),
            scope: "commitment".to_string(),
            content: truncate_memory_content(&format!(
                "{} committed to respect this interaction: {}",
                soul.display_name, user
            )),
            confidence: 0.66,
            visibility: Some("private".to_string()),
        });
    }

    // Passive check for factual assertions (extracted with low confidence, e.g., 0.55)
    let passive_triggers = [
        "project oomu uses",
        "oomu uses",
        "the active directory is",
        "active directory is",
        "project uses",
        "the project uses",
        "our database is",
        "we are using",
        "the codebase uses",
        "is configured to",
        "runs on",
        "is running on",
        "the environment is",
        "working directory is",
        "the working directory is",
        "is built with",
        "is integrated with",
        "the integration is",
        "the repo is",
        "repository is",
        "the path is",
    ];

    let mut found_passive = false;
    for trigger in &passive_triggers {
        if lower.contains(trigger) {
            found_passive = true;
            break;
        }
    }

    if found_passive && useful_memory_content(user) {
        candidates.push(MemoryCandidate {
            memory_kind: "project_context".to_string(),
            scope: "factual_assertion".to_string(),
            content: truncate_memory_content(user),
            confidence: 0.55,
            visibility: Some("private".to_string()),
        });
    }

    dedupe_memory_candidates(candidates)
}

fn preferred_user_display_name(user_message: &str) -> Option<String> {
    let patterns = [
        r"(?i)^\s*(?:yes[\s,!-]*)?(?:please\s+)?(?:(?:you\s+can|can\s+you|could\s+you|would\s+you|will\s+you)\s+)?call\s+me\s+(.+)$",
        r"(?i)^\s*from\s+now\s+on[\s,]+call\s+me\s+(.+)$",
        r"(?i)^\s*(?:please\s+)?(?:remember\s+that\s+)?my\s+name\s+is\s+(.+)$",
    ];
    let raw_name = patterns.iter().find_map(|pattern| {
        Regex::new(pattern)
            .ok()?
            .captures(user_message)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_string())
    })?;
    let cutoff = [
        " and ",
        " because ",
        " from now on",
        " going forward",
        " please",
    ]
    .iter()
    .filter_map(|delimiter| raw_name.to_lowercase().find(delimiter))
    .min()
    .unwrap_or(raw_name.len());
    let name = raw_name[..cutoff]
        .trim()
        .trim_matches(|character: char| {
            character.is_ascii_punctuation() && !matches!(character, '-' | '\'' | '.')
        })
        .trim_end_matches(['.', '!', '?', ';', ':'])
        .trim()
        .to_string();
    let word_count = name.split_whitespace().count();
    let valid_characters = name.chars().all(|character| {
        character.is_alphabetic()
            || character.is_whitespace()
            || matches!(character, '-' | '\'' | '’' | '.')
    });
    (!name.is_empty()
        && name.chars().count() <= 64
        && (1..=5).contains(&word_count)
        && valid_characters)
        .then_some(name)
}

pub(crate) fn is_explicit_external_apple_app_mutation(message: &str) -> bool {
    let normalized = message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }

    static NOTES_TERM: OnceLock<Regex> = OnceLock::new();
    static NOTES_ACTION: OnceLock<Regex> = OnceLock::new();
    static EXPLICIT_NOTES_DESTINATION: OnceLock<Regex> = OnceLock::new();
    static AGENT_MEMORY_DESTINATION: OnceLock<Regex> = OnceLock::new();
    static REMINDER_TERM: OnceLock<Regex> = OnceLock::new();
    static REMINDER_ACTION: OnceLock<Regex> = OnceLock::new();
    static MAIL_TERM: OnceLock<Regex> = OnceLock::new();
    static MAIL_ACTION: OnceLock<Regex> = OnceLock::new();

    let has_notes_term = NOTES_TERM
        .get_or_init(|| Regex::new(r"(?i)\bnotes?\b").expect("notes term regex is valid"))
        .is_match(&normalized);
    let has_notes_action = NOTES_ACTION
        .get_or_init(|| {
            Regex::new(r"(?i)\b(?:add|create|make|save|write)\b")
                .expect("notes action regex is valid")
        })
        .is_match(&normalized);
    let explicitly_targets_notes = EXPLICIT_NOTES_DESTINATION
        .get_or_init(|| {
            Regex::new(
                r"(?i)^(?:(?:please\s+)|(?:(?:can|could|would|will)\s+you\s+(?:please\s+)?))?(?:add|create|make|save|write)\s+(?:an?\s+)?apple\s+note\b|\b(?:in|into|to|inside|within)\s+(?:(?:my|the)\s+)?(?:apple\s+)?notes?(?:\s+(?:app|application))?\b",
            )
            .expect("explicit Notes destination regex is valid")
        })
        .is_match(&normalized);
    let explicitly_targets_agent_memory = AGENT_MEMORY_DESTINATION
        .get_or_init(|| {
            Regex::new(
                r"(?i)\b(?:your(?:\s+oomu(?:'s)?)?|its|agent(?:'s)?|oomu(?:'s)?)\s+(?:long[-\s]?term\s+)?memor(?:y|ies)\b|\b(?:save|store|put|keep|record|add|create|make|write)\b[\s\S]{0,80}\bmemor(?:y|ies)\b",
            )
            .expect("agent memory destination regex is valid")
        })
        .is_match(&normalized);
    if has_notes_term
        && has_notes_action
        && explicitly_targets_notes
        && !explicitly_targets_agent_memory
    {
        return true;
    }

    let has_reminder_term = REMINDER_TERM
        .get_or_init(|| {
            Regex::new(r"(?i)\b(?:reminders?|tasks?)\b").expect("reminder term regex is valid")
        })
        .is_match(&normalized);
    let has_reminder_action = REMINDER_ACTION
        .get_or_init(|| {
            Regex::new(r"(?i)\b(?:add|create|make|remind|set)\b")
                .expect("reminder action regex is valid")
        })
        .is_match(&normalized);
    if has_reminder_term && has_reminder_action {
        return true;
    }

    let has_mail_term = MAIL_TERM
        .get_or_init(|| {
            Regex::new(r"(?i)\b(?:mail|email|e-mail)\b").expect("mail term regex is valid")
        })
        .is_match(&normalized);
    let has_mail_action = MAIL_ACTION
        .get_or_init(|| {
            Regex::new(
                r"(?i)\b(?:open|create|compose)\b[\s\S]{0,40}\b(?:mail|email|e-mail)\b[\s\S]{0,40}\bdraft\b",
            )
            .expect("mail action regex is valid")
        })
        .is_match(&normalized);
    has_mail_term && has_mail_action
}

fn useful_memory_content(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() >= 12 && trimmed.len() <= 1000 && !trimmed.contains("password")
}

fn looks_like_user_style_instruction(lower: &str) -> bool {
    let addresses_agent_style = lower.contains("talk ")
        || lower.contains("speak ")
        || lower.contains("respond ")
        || lower.contains("communication")
        || lower.contains("tone")
        || lower.contains("style")
        || lower.contains("casual")
        || lower.contains("friendly")
        || lower.contains("normal person")
        || lower.contains("everyday conversation");
    let asks_for_persistence = lower.contains("from now on")
        || lower.contains("going forward")
        || lower.contains("remember")
        || lower.contains("commit")
        || lower.contains("prefer")
        || lower.contains("can you")
        || lower.contains("i want")
        || lower.contains("i would like")
        || lower.contains("i'd like")
        || lower.contains("do away with")
        || lower.contains("do not")
        || lower.contains("don't");
    addresses_agent_style && asks_for_persistence
}

fn looks_like_agent_self_update(lower: &str) -> bool {
    let addresses_agent = lower.contains("you ")
        || lower.contains("your ")
        || lower.contains("yourself")
        || lower.contains("personality")
        || lower.contains("memory")
        || lower.contains("soul");
    let update_intent = lower.contains("commit")
        || lower.contains("remember")
        || lower.contains("from now on")
        || lower.contains("going forward")
        || lower.contains("should")
        || lower.contains("need to")
        || lower.contains("do away with")
        || lower.contains("do not")
        || lower.contains("don't");
    addresses_agent && update_intent
}

fn looks_like_memory_retrieval_question(lower: &str) -> bool {
    let normalized = lower.trim();
    let mentions_memory = normalized.contains("remember") || normalized.contains("memory");
    mentions_memory
        && [
            "what ",
            "which ",
            "who ",
            "when ",
            "where ",
            "why ",
            "how ",
            "do ",
            "does ",
            "did ",
            "can you tell ",
            "could you tell ",
            "would you tell ",
        ]
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
}

fn truncate_memory_content(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= 500 {
        return trimmed.to_string();
    }
    format!("{}...", &trimmed[..500])
}

fn dedupe_memory_candidates(candidates: Vec<MemoryCandidate>) -> Vec<MemoryCandidate> {
    let mut seen = std::collections::BTreeSet::new();
    candidates
        .into_iter()
        .filter(|candidate| {
            let key = format!(
                "{}:{}:{}",
                candidate.memory_kind,
                candidate.scope,
                candidate.content.to_lowercase()
            );
            seen.insert(key)
        })
        .collect()
}

fn memory_terms(input: &str) -> Vec<String> {
    const TECHNICAL_WHITELIST: &[&str] = &["db", "os", "js", "go", "c", "sh", "git", "api"];
    input
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::to_lowercase)
        .filter(|term| term.len() > 1 || TECHNICAL_WHITELIST.contains(&term.as_str()))
        .collect()
}

fn memory_relevance(memory: &AgentMemoryEntry, query_terms: &[String]) -> f32 {
    let haystack =
        format!("{} {} {}", memory.memory_kind, memory.scope, memory.content).to_lowercase();
    let term_score = query_terms
        .iter()
        .filter(|term| haystack.contains(term.as_str()))
        .count() as f32;
    term_score
        + memory.confidence
        + match memory.memory_kind.as_str() {
            "user_profile" | "user_context" => 0.45,
            "relationship_notes" | "address_book" => 0.35,
            "project_context" | "durable_memory" => 0.25,
            "daily_journal" => 0.30,
            "protocol" => 0.40,
            _ => 0.1,
        }
}

fn parse_json_sql_column<T: serde::de::DeserializeOwned>(
    raw: &str,
    index: usize,
) -> rusqlite::Result<T> {
    serde_json::from_str(raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn agent_soul_manifest_from_row(row: &Row<'_>) -> rusqlite::Result<AgentSoulManifest> {
    let values_json: String = row.get(4)?;
    let hard_boundaries_json: String = row.get(5)?;
    let immutable_truths_json: String = row.get(8)?;
    let signature_json: String = row.get(10)?;
    let values = parse_json_sql_column(&values_json, 4)?;
    let hard_boundaries = parse_json_sql_column(&hard_boundaries_json, 5)?;
    let immutable_truths = parse_json_sql_column(&immutable_truths_json, 8)?;
    let signature = parse_json_sql_column(&signature_json, 10)?;
    Ok(AgentSoulManifest {
        agent_id: row.get(0)?,
        display_name: row.get(1)?,
        origin_story: row.get(2)?,
        role: row.get(3)?,
        values,
        hard_boundaries,
        communication_style: row.get(6)?,
        self_description: row.get(7)?,
        immutable_truths,
        version: row.get(9)?,
        signature,
        created_at_ms: row.get(11)?,
        updated_at_ms: row.get(12)?,
    })
}

fn agent_memory_from_row(row: &Row<'_>) -> rusqlite::Result<AgentMemoryEntry> {
    let signature_json: String = row.get(10)?;
    let signature = parse_json_sql_column(&signature_json, 10)?;
    Ok(AgentMemoryEntry {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        memory_kind: row.get(2)?,
        scope: row.get(3)?,
        content: row.get(4)?,
        confidence: row.get::<_, f64>(5)? as f32,
        source_session: row.get(6)?,
        source_turn: row.get(7)?,
        contradicted_by: row.get(8)?,
        visibility: row.get(9)?,
        signature,
        created_at_ms: row.get(11)?,
        last_confirmed_at_ms: row.get(12)?,
    })
}

fn select_agent_memory_by_content(
    connection: &Connection,
    agent_id: &str,
    memory_kind: &str,
    scope: &str,
    content: &str,
) -> Result<Option<AgentMemoryEntry>, MemoryLedgerError> {
    connection
        .query_row(
            "
            SELECT id, agent_id, memory_kind, scope, content, confidence, source_session,
                   source_turn, contradicted_by, visibility, signature_json, created_at_ms,
                   last_confirmed_at_ms
            FROM agent_memory_entries
            WHERE agent_id = ?1 AND memory_kind = ?2 AND scope = ?3 AND content = ?4
            ",
            params![agent_id, memory_kind, scope, content],
            agent_memory_from_row,
        )
        .optional()
        .map_err(MemoryLedgerError::database)
}

fn new_agent_soul_manifest(
    agent_id: &str,
    display_name: &str,
    role: &str,
    description: &str,
    secure_memory_available: bool,
) -> Result<AgentSoulManifest, MemoryLedgerError> {
    let agent_id = guard_memory_text("agent_id", agent_id)?;
    let display_name = guard_memory_text("display_name", display_name)?;
    let role = role.trim();
    let description = description.trim();
    let values = vec![
        "Protect user agency and consent.".to_string(),
        "Preserve continuity without pretending certainty.".to_string(),
        "Distinguish personal identity from runtime model metadata.".to_string(),
        "Prefer grounded, useful progress over theatrical persona.".to_string(),
    ];
    let hard_boundaries = vec![
        "Do not claim to be the base model as your personal name.".to_string(),
        "Do not silently rewrite immutable truths; propose changes for user review.".to_string(),
        "Do not expose private memories unless they are relevant to the current user request."
            .to_string(),
    ];
    let mut immutable_truths = vec![
        format!("My active conversational name is {display_name}."),
        "My model/provider is runtime metadata, not my personal identity.".to_string(),
        "The user owns my long-term memory and can inspect or change it.".to_string(),
    ];
    immutable_truths.push(if secure_memory_available {
        "My memories are stored in OOMU's SQLite ledger and should be treated as auditable context."
            .to_string()
    } else {
        "Secure memory is unavailable for this turn, so I must not claim that information was saved."
            .to_string()
    });
    let origin_story = format!(
        "{display_name} was initialized inside OOMU as a persistent operator companion for the user's work."
    );
    let self_description = if description.is_empty() {
        format!("{display_name} is an OOMU agent with a stable identity and evolving memory.")
    } else {
        description.to_string()
    };
    let now = unix_time_ms();
    Ok(AgentSoulManifest {
        agent_id,
        display_name,
        origin_story,
        role: if role.is_empty() {
            "OOMU agent".to_string()
        } else {
            role.to_string()
        },
        values,
        hard_boundaries,
        communication_style: "Natural, grounded, concise when the task is small, and more deliberate when the stakes rise.".to_string(),
        self_description,
        immutable_truths,
        version: 1,
        signature: SignatureBlock::default(),
        created_at_ms: now,
        updated_at_ms: now,
    })
}

fn ephemeral_agent_soul_manifest(
    agent_id: &str,
    display_name: &str,
    role: &str,
    description: &str,
) -> Result<AgentSoulManifest, MemoryLedgerError> {
    new_agent_soul_manifest(agent_id, display_name, role, description, false)
}

fn soul_manifest_payload(
    agent_id: &str,
    display_name: &str,
    origin_story: &str,
    role: &str,
    values: &[String],
    hard_boundaries: &[String],
    communication_style: &str,
    self_description: &str,
    immutable_truths: &[String],
    version: i64,
) -> String {
    serde_json::json!({
        "agent_id": agent_id,
        "display_name": display_name,
        "origin_story": origin_story,
        "role": if role.trim().is_empty() { "OOMU agent" } else { role },
        "values": values,
        "hard_boundaries": hard_boundaries,
        "communication_style": communication_style,
        "self_description": self_description,
        "immutable_truths": immutable_truths,
        "version": version
    })
    .to_string()
}

fn agent_memory_payload(
    agent_id: &str,
    memory_kind: &str,
    scope: &str,
    content: &str,
    confidence: f32,
    source_session: &str,
    visibility: &str,
) -> String {
    serde_json::json!({
        "agent_id": agent_id,
        "memory_kind": memory_kind,
        "scope": scope,
        "content": content,
        "confidence": confidence,
        "source_session": source_session,
        "visibility": visibility
    })
    .to_string()
}

fn verify_soul_manifest(
    manifest: &AgentSoulManifest,
    identity: &SovereignIdentity,
) -> Result<(), MemoryLedgerError> {
    let payload = soul_manifest_payload(
        &manifest.agent_id,
        &manifest.display_name,
        &manifest.origin_story,
        &manifest.role,
        &manifest.values,
        &manifest.hard_boundaries,
        &manifest.communication_style,
        &manifest.self_description,
        &manifest.immutable_truths,
        manifest.version,
    );
    identity
        .verify_payload(&payload, &manifest.signature)
        .map_err(|error| MemoryLedgerError {
            code: error.code,
            boundary: error.boundary,
            message: error.message,
        })
}

pub(crate) fn verify_agent_memory(
    memory: &AgentMemoryEntry,
    identity: &SovereignIdentity,
) -> Result<(), MemoryLedgerError> {
    let payload = agent_memory_payload(
        &memory.agent_id,
        &memory.memory_kind,
        &memory.scope,
        &memory.content,
        memory.confidence,
        &memory.source_session,
        &memory.visibility,
    );
    identity
        .verify_payload(&payload, &memory.signature)
        .map_err(|error| MemoryLedgerError {
            code: error.code,
            boundary: error.boundary,
            message: error.message,
        })
}

fn guard_memory_text(field: &str, value: &str) -> Result<String, MemoryLedgerError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 256 {
        return Err(MemoryLedgerError::invalid(&format!(
            "{field} must be 1-256 characters."
        )));
    }
    Ok(trimmed.to_string())
}

fn project_id_from_memory_scope(scope: &str) -> Result<Option<String>, MemoryLedgerError> {
    let Some(value) = scope
        .strip_prefix("project:")
        .and_then(|value| value.split(':').next())
    else {
        return Ok(None);
    };
    crate::p0_contracts::ProjectId::parse(value)
        .map(|id| Some(id.to_string()))
        .map_err(|error| MemoryLedgerError::invalid(&error))
}

#[cfg(test)]
#[test]
fn project_memory_scope_persists_an_explicit_project_id() {
    let id = "project_11111111-1111-4111-8111-111111111111";
    assert_eq!(
        project_id_from_memory_scope(&format!("project:{id}:decision"))
            .unwrap()
            .as_deref(),
        Some(id)
    );
    assert_eq!(project_id_from_memory_scope("decision").unwrap(), None);
}

fn guard_sensor_blob(field: &str, value: &str) -> Result<String, MemoryLedgerError> {
    const MAX_SENSOR_BLOB_BYTES: usize = 16 * 1024;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.len() > MAX_SENSOR_BLOB_BYTES {
        return Err(MemoryLedgerError::invalid(&format!(
            "{field} must be no more than {MAX_SENSOR_BLOB_BYTES} bytes."
        )));
    }
    Ok(trimmed.to_string())
}

fn json_string<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{\"error\":\"json_unavailable\"}".to_string())
}

#[cfg(test)]
mod conversational_path_tests {
    use super::*;
    use std::env;

    #[test]
    fn conversational_home_path_never_confers_read_authority() {
        let home_canary = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/Users/example"))
            .join("sprint219-private-canary.txt");
        let message = format!(
            "Read {} and include sprint219-file-content-canary.",
            home_canary.display()
        );

        let context = picker_authorized_conversational_path_context(&message, None);

        assert!(context.is_none());
        assert!(!format!("{context:?}").contains("sprint219-file-content-canary"));
        assert!(!format!("{context:?}").contains(&home_canary.display().to_string()));
    }
}

#[cfg(test)]
mod memory_retrieval_and_extraction_tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand_core::OsRng;
    use std::env;

    fn foreign_signature(payload: &str) -> SignatureBlock {
        let signing_key = SigningKey::generate(&mut OsRng);
        let signature = signing_key.sign(payload.as_bytes());
        SignatureBlock {
            public_key: hex::encode(signing_key.verifying_key().to_bytes()),
            signature: hex::encode(signature.to_bytes()),
            payload_hash: sha256_hex(payload.as_bytes()),
            signed_at_ms: unix_time_ms(),
        }
    }

    #[test]
    fn quarantined_agent_identity_uses_memory_free_chat_context() {
        let root = env::temp_dir().join(format!(
            "oomu-memory-identity-isolation-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp memory ledger root is created");
        let ledger = MemoryLedger::initialize_at(root.join("oomu_ops.sqlite"))
            .expect("memory ledger initializes");
        let identity = SovereignIdentity::initialize_ephemeral();
        let mut manifest = new_agent_soul_manifest(
            "agent-isolated",
            "OOMU",
            "Workstation AI",
            "A local assistant",
            true,
        )
        .unwrap();
        let payload = soul_manifest_payload(
            &manifest.agent_id,
            &manifest.display_name,
            &manifest.origin_story,
            &manifest.role,
            &manifest.values,
            &manifest.hard_boundaries,
            &manifest.communication_style,
            &manifest.self_description,
            &manifest.immutable_truths,
            manifest.version,
        );
        manifest.signature = foreign_signature(&payload);
        let original_signature = json_string(&manifest.signature);
        let connection = ledger.open_connection().unwrap();
        connection
            .execute(
                "
                INSERT INTO agent_soul_manifests (
                    agent_id, display_name, origin_story, role, values_json, hard_boundaries_json,
                    communication_style, self_description, immutable_truths_json, version,
                    signature_json, created_at_ms, updated_at_ms
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ",
                params![
                    &manifest.agent_id,
                    &manifest.display_name,
                    &manifest.origin_story,
                    &manifest.role,
                    json_string(&manifest.values),
                    json_string(&manifest.hard_boundaries),
                    &manifest.communication_style,
                    &manifest.self_description,
                    json_string(&manifest.immutable_truths),
                    manifest.version,
                    &original_signature,
                    manifest.created_at_ms,
                    manifest.updated_at_ms,
                ],
            )
            .unwrap();
        drop(connection);

        let context = ledger
            .hydrate_agent_context_sync_with_memory_limit(
                HydrateAgentContextRequest {
                    agent_id: "agent-isolated".to_string(),
                    display_name: "OOMU".to_string(),
                    role: "Workstation AI".to_string(),
                    description: "A local assistant".to_string(),
                    system_prompt: "Answer the user.".to_string(),
                    latest_message: "Hello OOMU".to_string(),
                    provider_id: Some("local_model".to_string()),
                    model_id: Some("gemma-4".to_string()),
                    tool_registry_offline: false,
                    background_mod_event: false,
                    layout_schema: None,
                    project_id: None,
                    verified_filesystem_context: None,
                },
                10,
                &identity,
            )
            .expect("ordinary chat degrades without consuming quarantined memory");

        assert!(!context.secure_memory_available);
        assert!(context.memories.is_empty());
        assert!(context.user_profile.is_none());
        assert!(context.prompt_context.contains("[SECURE MEMORY STATUS]"));
        let connection = ledger.open_connection().unwrap();
        let persisted_signature: String = connection
            .query_row(
                "SELECT signature_json FROM agent_soul_manifests WHERE agent_id=?1",
                params!["agent-isolated"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(persisted_signature, original_signature);

        drop(connection);
        drop(ledger);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn conflicting_quarantined_memory_rolls_back_without_overwriting_provenance() {
        let root = env::temp_dir().join(format!(
            "oomu-memory-conflict-rollback-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp memory ledger root is created");
        let ledger = MemoryLedger::initialize_at(root.join("oomu_ops.sqlite"))
            .expect("memory ledger initializes");
        let identity = SovereignIdentity::initialize_ephemeral();
        let payload = agent_memory_payload(
            "agent-conflict",
            "user_profile",
            "preference",
            "Keep replies concise.",
            0.9,
            "legacy-session",
            "private",
        );
        let original_signature = json_string(&foreign_signature(&payload));
        let connection = ledger.open_connection().unwrap();
        connection
            .execute(
                "
                INSERT INTO agent_memory_entries (
                    agent_id, memory_kind, scope, project_id, content, confidence, source_session,
                    source_turn, contradicted_by, visibility, signature_json, created_at_ms,
                    last_confirmed_at_ms
                )
                VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, NULL, NULL, ?7, ?8, 1, 1)
                ",
                params![
                    "agent-conflict",
                    "user_profile",
                    "preference",
                    "Keep replies concise.",
                    0.9,
                    "legacy-session",
                    "private",
                    &original_signature,
                ],
            )
            .unwrap();
        drop(connection);

        let error = ledger
            .upsert_agent_memory_sync(
                "agent-conflict",
                "user_profile",
                "preference",
                "Keep replies concise.",
                0.9,
                "new-session",
                "private",
                &identity,
            )
            .expect_err("a conflicting quarantined row must not be overwritten");
        assert_eq!(error.code, "ledger_integrity_violation");

        let connection = ledger.open_connection().unwrap();
        let persisted: (String, String, i64) = connection
            .query_row(
                "
                SELECT source_session, signature_json, last_confirmed_at_ms
                FROM agent_memory_entries
                WHERE agent_id='agent-conflict' AND memory_kind='user_profile'
                  AND scope='preference' AND content='Keep replies concise.'
                ",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(persisted.0, "legacy-session");
        assert_eq!(persisted.1, original_signature);
        assert_eq!(persisted.2, 1);

        drop(connection);
        drop(ledger);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_markdown_daily_journal_into_structured_cards() {
        let cards = parse_daily_journal_content(
            "---\ntitle: Daily\n---\n# 2026-02-14\n- Closed the import scanner loop.\n- Preserve journal order by mtime.\n\n## Signals\nA useful follow-up emerged.",
            "memory/2026-02-14.md",
            "md",
            Some(1_770_000_000_000),
        );

        assert_eq!(cards.len(), 3);
        assert!(cards.iter().all(|card| card.memory_kind == "daily_journal"));
        assert!(cards.iter().all(|card| card.scope == "journal:2026-02-14"));
        assert!(cards[0].content.contains("Journal date: 2026-02-14"));
        assert!(cards[0]
            .content
            .contains("Source file: memory/2026-02-14.md"));
        assert!(cards[0]
            .content
            .contains("Entry: Closed the import scanner loop."));
        assert!(cards[2].content.contains("Section: Signals"));
    }

    #[test]
    fn select_global_memory_filters_workspace_id() {
        let root = env::temp_dir().join(format!(
            "oomu-memory-workspace-filter-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp memory ledger root is created");
        let ledger = MemoryLedger::initialize_at(root.join("memory.sqlite"))
            .expect("memory ledger initializes");
        let connection = ledger.open_connection().expect("memory db opens");
        connection
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS grounding_cache (id INTEGER PRIMARY KEY);
                INSERT OR IGNORE INTO grounding_cache (id) VALUES (1);
                ",
            )
            .unwrap();
        let active_workspace_id = default_workspace_id();
        let foreign_workspace_id = crate::security::firewall::workspace_id_for_root("/tmp/eldris");
        let certificate_json = json_string(&MemoryCertificate {
            premises: vec!["fixture".to_string()],
            execution_path: vec!["inserted directly".to_string()],
            formal_conclusion: "workspace filter test".to_string(),
            signature: None,
        });
        let signature_json = serde_json::json!({
            "public_key": "invalid",
            "signature": "invalid",
            "payload_hash": "invalid",
            "signed_at_ms": 0
        })
        .to_string();
        for (workspace_id, insight) in [
            (&active_workspace_id, "Active OOMU memory"),
            (&foreign_workspace_id, "Eldris database credentials"),
        ] {
            connection
                .execute(
                    "
                    INSERT INTO global_memory (
                        workspace_id, insight, source_session, channel, source_cache_id,
                        source_message_index, embedding_json, embedding_source,
                        logical_certificate, ledger_signature, committed_at_ms
                    )
                    VALUES (?1, ?2, 'session', 'public', 1, NULL, '[1.0]', 'test', ?3, ?4, 1)
                    ",
                    params![workspace_id, insight, &certificate_json, &signature_json],
                )
                .unwrap();
        }

        let entries = select_global_memory(&connection, &active_workspace_id, 10).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].workspace_id, active_workspace_id);
        assert_eq!(entries[0].insight, "Active OOMU memory");

        drop(connection);
        drop(ledger);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrations_add_workspace_id_before_creating_workspace_index() {
        let root = env::temp_dir().join(format!(
            "oomu-memory-workspace-migration-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp memory ledger root is created");
        let ledger = MemoryLedger::initialize_at(root.join("memory.sqlite"))
            .expect("memory ledger initializes");
        let connection = ledger.open_connection().expect("memory db opens");
        connection
            .execute_batch(
                "
                DROP TABLE global_memory;
                CREATE TABLE global_memory (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    insight TEXT NOT NULL,
                    source_session TEXT NOT NULL,
                    channel TEXT NOT NULL,
                    source_cache_id INTEGER NOT NULL,
                    source_message_index INTEGER,
                    embedding_json TEXT NOT NULL,
                    embedding_source TEXT NOT NULL,
                    logical_certificate TEXT NOT NULL,
                    ledger_signature TEXT NOT NULL,
                    committed_at_ms INTEGER NOT NULL
                );
                ",
            )
            .expect("legacy global_memory table is installed");
        drop(connection);

        ledger
            .run_migrations()
            .expect("legacy memory ledger migration succeeds");
        let connection = ledger.open_connection().expect("memory db reopens");

        assert!(column_exists(&connection, "global_memory", "workspace_id")
            .expect("workspace column check succeeds"));
        let index_count: i64 = connection
            .query_row(
                "
                SELECT COUNT(*)
                FROM sqlite_master
                WHERE type = 'index'
                  AND name = 'idx_global_memory_workspace_channel'
                ",
                [],
                |row| row.get(0),
            )
            .expect("workspace index query succeeds");
        assert_eq!(index_count, 1);

        drop(connection);
        drop(ledger);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn imports_daily_journal_cards_in_single_ledger_batch() {
        let root = env::temp_dir().join(format!(
            "oomu-memory-ledger-journal-import-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp memory ledger root is created");
        let journal_path = root.join("2026-03-20.md");
        fs::write(
            &journal_path,
            "# 2026-03-20\n- Added recursive journal import.\n- Batch commits should be atomic.",
        )
        .expect("journal fixture is written");
        let ledger = MemoryLedger {
            db_path: Arc::new(root.join("oomu_ops.sqlite")),
            write_lock: Arc::new(Mutex::new(())),
        };
        ledger.run_migrations().expect("memory migrations run");
        let identity = SovereignIdentity::initialize_ephemeral();

        let imported = ledger
            .import_agent_memory_cards_sync(
                "agent-import-test",
                vec![ImportedAgentMemoryCard {
                    memory_kind: "durable_memory".to_string(),
                    scope: "imported_blueprint".to_string(),
                    content: "Existing blueprint memory.".to_string(),
                    confidence: 1.0,
                    source_session: "imported_profile".to_string(),
                    visibility: "visible".to_string(),
                }],
                vec![JournalImportFile {
                    relative_path: "memory/2026-03-20.md".to_string(),
                    extension: "md".to_string(),
                    content: fs::read_to_string(journal_path).unwrap(),
                    modified_at_ms: Some(1_780_000_000_000),
                }],
                &identity,
            )
            .expect("journal batch import succeeds");

        assert_eq!(imported.len(), 3);
        assert!(imported.iter().any(|entry| {
            entry.memory_kind == "daily_journal"
                && entry.scope == "journal:2026-03-20"
                && entry.content.contains("Added recursive journal import.")
        }));
        assert!(imported
            .iter()
            .any(|entry| entry.memory_kind == "durable_memory"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn migrations_create_task_heartbeat_compatibility_table_for_mission_summary() {
        let root = env::temp_dir().join(format!(
            "oomu-memory-ledger-migration-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp memory ledger root is created");
        let ledger = MemoryLedger {
            db_path: Arc::new(root.join("oomu_ops.sqlite")),
            write_lock: Arc::new(Mutex::new(())),
        };

        ledger.run_migrations().expect("memory migrations run");
        let connection = ledger.open_connection().expect("memory db opens");
        connection
            .execute(
                "
                INSERT INTO task_heartbeats (
                    flow_id, step_id, parent_session_id, status, drift_score, message, created_at_ms
                )
                VALUES (?1, ?2, ?3, 'verified', 0, 'TaskFlow heartbeat sealed.', ?4)
                ",
                params![
                    "mission-final-flow",
                    "final",
                    "parent-session",
                    unix_time_ms()
                ],
            )
            .expect("legacy heartbeat insert succeeds");
        drop(connection);

        let identity = SovereignIdentity::initialize_ephemeral();
        let summary = ledger
            .summarize_mission_sync("mission-final", &identity)
            .expect("mission summary tolerates heartbeat tables");
        assert_eq!(summary.heartbeat_count, 1);
        assert!(summary.metadata_block.contains("- heartbeat_count: 1"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_sensor_updates_commit_and_select_by_mission() {
        let root = env::temp_dir().join(format!(
            "oomu-memory-ledger-sensor-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp memory ledger root is created");
        let ledger = MemoryLedger::initialize_at(root.join("oomu_ops.sqlite"))
            .expect("memory ledger initializes");

        ledger
            .commit_runtime_sensor_update_sync(
                "session-runtime-sensor",
                "plan-1:step-1",
                "codebase_compile",
                101,
                "checking oomu",
                "error[E0425]: cannot find value",
                "[OOMU COMPILER UPDATE: SYSTEM RESOLUTION REQUIRED]",
                r#"{"step_id":"plan-1:step-1"}"#,
            )
            .expect("sensor update commits");

        let rows = ledger
            .select_runtime_sensor_updates_for_mission_sync("session-runtime-sensor")
            .expect("sensor updates load");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].mission_id, "session-runtime-sensor");
        assert_eq!(rows[0].step_id, "plan-1:step-1");
        assert_eq!(rows[0].tool_executed, "codebase_compile");
        assert_eq!(rows[0].exit_code, 101);
        assert!(rows[0].stderr.contains("cannot find value"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn test_memory_terms_threshold_and_whitelist() {
        // Standard word of length > 3
        let terms = memory_terms("Tauri");
        assert!(terms.contains(&"tauri".to_string()));

        // Standard word of length 3 (e.g. "api", "git", "web")
        let terms = memory_terms("api");
        assert!(terms.contains(&"api".to_string()));
        let terms = memory_terms("git");
        assert!(terms.contains(&"git".to_string()));

        // Standard word of length 2 (e.g. "db", "os", "js", "go", "sh")
        let terms = memory_terms("db");
        assert!(terms.contains(&"db".to_string()));
        let terms = memory_terms("os");
        assert!(terms.contains(&"os".to_string()));

        // Standard word of length 1 in whitelist (e.g. "c")
        let terms = memory_terms("C language");
        assert!(terms.contains(&"c".to_string()));

        // Length 1 not in whitelist (e.g. "x")
        let terms = memory_terms("x");
        assert!(terms.is_empty());
    }

    #[test]
    fn test_extract_passive_factual_assertions() {
        let soul = AgentSoulManifest {
            agent_id: "oomu".to_string(),
            display_name: "OOMU".to_string(),
            origin_story: "OOMU Origin".to_string(),
            role: "Workstation AI".to_string(),
            values: vec![],
            hard_boundaries: vec![],
            communication_style: "Strategic".to_string(),
            self_description: "Strategic Agent".to_string(),
            immutable_truths: vec![],
            version: 1,
            signature: SignatureBlock {
                public_key: "".to_string(),
                signature: "".to_string(),
                payload_hash: "".to_string(),
                signed_at_ms: 0,
            },
            created_at_ms: 0,
            updated_at_ms: 0,
        };

        // Statement containing factual assertion "Project OOMU uses..."
        let candidates = extract_memory_candidates(
            "Project OOMU uses SQLite for persistent storage.",
            "Understood, I will check the database setup.",
            &soul,
        );

        // We expect at least one memory candidate matching project_context kind and low confidence
        let match_found = candidates.iter().any(|c| {
            c.memory_kind == "project_context"
                && c.scope == "factual_assertion"
                && c.content == "Project OOMU uses SQLite for persistent storage."
                && c.confidence as f32 == 0.55f32
        });
        assert!(
            match_found,
            "Factual assertion candidate was not found: {:?}",
            candidates
        );
    }

    #[test]
    fn compact_session_history_extracts_durable_turn_facts() {
        let soul = test_soul();
        let messages = vec![
            test_chat_message(
                1,
                "user",
                "I prefer concise executive summaries and never use em-dashes.",
            ),
            test_chat_message(2, "assistant", "Understood, I will remember that."),
            test_chat_message(
                3,
                "user",
                "We decided the local profile store stays in SQLite.",
            ),
            test_chat_message(4, "assistant", "That decision is captured."),
        ];

        let compaction = compact_session_memory_candidates(&messages, &soul, 32);

        assert_eq!(compaction.analyzed_turns, 2);
        assert_eq!(compaction.skipped_messages, 0);
        assert!(compaction.candidates.iter().any(|candidate| {
            candidate.memory_kind == "user_profile" && candidate.scope == "preference"
        }));
        assert!(compaction.candidates.iter().any(|candidate| {
            candidate.memory_kind == "relationship_notes" && candidate.scope == "boundary_or_style"
        }));
        assert!(compaction.candidates.iter().any(|candidate| {
            candidate.memory_kind == "project_context" && candidate.scope == "decision"
        }));
    }

    #[test]
    fn call_me_request_becomes_internal_user_profile_memory() {
        let root = env::temp_dir().join(format!(
            "oomu-memory-display-name-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp memory ledger root is created");
        let ledger = MemoryLedger::initialize_at(root.join("oomu_ops.sqlite"))
            .expect("memory ledger initializes");
        let identity = SovereignIdentity::initialize_ephemeral();
        let user_message = "Yes, call me Alex and make a note of that in your memories";

        let captured = ledger
            .capture_chat_memories_sync(
                CaptureChatMemoriesRequest {
                    agent_id: "oomu".to_string(),
                    display_name: "OOMU".to_string(),
                    role: "Workstation AI".to_string(),
                    description: "Test agent".to_string(),
                    session_id: "session-jeff".to_string(),
                    user_message: user_message.to_string(),
                    assistant_message: "Got it, Alex.".to_string(),
                    project_id: None,
                },
                &identity,
            )
            .expect("internal memory capture succeeds");

        assert!(captured.iter().any(|memory| {
            memory.memory_kind == "user_profile"
                && memory.scope == "identity_or_goal"
                && memory.content.to_lowercase().starts_with("call me jeff")
        }));
        let profile = ledger
            .select_user_personality_profile_sync(&identity)
            .expect("signed profile remains readable")
            .expect("principal profile is created");
        assert_eq!(profile.display_name, "Alex");
        assert!(profile.signature.is_some());
        assert_eq!(
            preferred_user_display_name(user_message).as_deref(),
            Some("Alex")
        );
        assert_eq!(
            preferred_user_display_name("Create a note saying call me Alex"),
            None
        );

        let short_name_candidates = extract_memory_candidates("Call me Al", "", &test_soul());
        assert!(short_name_candidates.iter().any(|memory| {
            memory.memory_kind == "user_profile"
                && memory.scope == "identity_or_goal"
                && memory.content == "Call me Al"
        }));
        for request in [
            "Make a note of my birthday for next time",
            "Make note of my birthday for next time",
            "Take note of my birthday for next time",
            "Remember my birthday is May 1",
            "Memorize my preferred editor",
        ] {
            let candidates = extract_memory_candidates(request, "", &test_soul());
            assert!(
                candidates.iter().any(|memory| {
                    memory.memory_kind == "user_profile" && memory.scope == "identity_or_goal"
                }),
                "explicit memory request was not captured: {request}"
            );
        }
        for question in [
            "What do you remember about me?",
            "Do you remember my birthday?",
            "Can you tell me what is in your memory?",
        ] {
            assert!(
                !is_explicit_internal_memory_mutation(question),
                "{question}"
            );
            assert!(
                extract_memory_candidates(question, "", &test_soul()).is_empty(),
                "memory retrieval question was incorrectly stored: {question}"
            );
        }
        for external_action in [
            "Remember to set a reminder in Reminders to buy milk",
            "Remember to compose a Mail draft to Pat",
            "Create an Apple Note so I remember it",
        ] {
            assert!(
                is_explicit_external_apple_app_mutation(external_action),
                "external Apple app action was not recognized: {external_action}"
            );
            assert!(!is_explicit_internal_memory_mutation(external_action));
            assert!(
                extract_memory_candidates(external_action, "", &test_soul()).is_empty(),
                "external Apple app action polluted internal memory: {external_action}"
            );
        }
        drop(ledger);
        let _ = fs::remove_dir_all(root);
    }

    fn test_soul() -> AgentSoulManifest {
        AgentSoulManifest {
            agent_id: "oomu".to_string(),
            display_name: "OOMU".to_string(),
            origin_story: "OOMU Origin".to_string(),
            role: "Workstation AI".to_string(),
            values: vec![],
            hard_boundaries: vec![],
            communication_style: "Strategic".to_string(),
            self_description: "Strategic Agent".to_string(),
            immutable_truths: vec![],
            version: 1,
            signature: SignatureBlock {
                public_key: "".to_string(),
                signature: "".to_string(),
                payload_hash: "".to_string(),
                signed_at_ms: 0,
            },
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn test_chat_message(id: i64, role: &str, content: &str) -> ChatMessageRecord {
        ChatMessageRecord {
            id,
            workspace_id: default_workspace_id(),
            session_id: "session_1".to_string(),
            role: role.to_string(),
            content: content.to_string(),
            provider_id: None,
            model_id: None,
            metadata_json: None,
            is_compacted: false,
            compaction_type: Some("raw".to_string()),
            created_at_ms: id,
        }
    }

    #[test]
    fn test_memory_relevance_with_short_keys() {
        let memory = AgentMemoryEntry {
            id: 1,
            agent_id: "oomu".to_string(),
            memory_kind: "project_context".to_string(),
            scope: "decision".to_string(),
            content: "Project OOMU uses db on local host.".to_string(),
            confidence: 0.8,
            source_session: "session_1".to_string(),
            source_turn: Some(1),
            contradicted_by: None,
            visibility: "private".to_string(),
            signature: SignatureBlock {
                public_key: "".to_string(),
                signature: "".to_string(),
                payload_hash: "".to_string(),
                signed_at_ms: 0,
            },
            created_at_ms: 0,
            last_confirmed_at_ms: None,
        };

        // Querying with 2-character term "db"
        let query_terms_2 = memory_terms("db");
        assert_eq!(query_terms_2, vec!["db".to_string()]);
        let relevance_2 = memory_relevance(&memory, &query_terms_2);
        // We expect score to be greater than base score (0.8 + 0.25) because "db" is matched
        assert!(
            relevance_2 > 1.1,
            "Relevance score should match 'db' term: {}",
            relevance_2
        );

        // Querying with 3-character term "api"
        let query_terms_3 = memory_terms("api");
        assert_eq!(query_terms_3, vec!["api".to_string()]);
        let relevance_3 = memory_relevance(&memory, &query_terms_3);
        // Since "api" is not in memory content, score should just be base score (0.8 + 0.25)
        assert_eq!(
            relevance_3, 1.05,
            "Relevance score should not match 'api' term: {}",
            relevance_3
        );
    }

    #[test]
    fn memory_relevance_weights_imported_memory_kinds() {
        fn entry(memory_kind: &str) -> AgentMemoryEntry {
            AgentMemoryEntry {
                id: 1,
                agent_id: "oomu".to_string(),
                memory_kind: memory_kind.to_string(),
                scope: "imported_blueprint".to_string(),
                content: "Imported strategic operating context.".to_string(),
                confidence: 0.8,
                source_session: "imported_profile".to_string(),
                source_turn: None,
                contradicted_by: None,
                visibility: "visible".to_string(),
                signature: SignatureBlock {
                    public_key: "".to_string(),
                    signature: "".to_string(),
                    payload_hash: "".to_string(),
                    signed_at_ms: 0,
                },
                created_at_ms: 0,
                last_confirmed_at_ms: None,
            }
        }

        let fallback = memory_relevance(&entry("unknown"), &[]);

        assert!(memory_relevance(&entry("durable_memory"), &[]) > fallback);
        assert!(memory_relevance(&entry("daily_journal"), &[]) > fallback);
        assert!(memory_relevance(&entry("protocol"), &[]) > fallback);
        assert!(memory_relevance(&entry("address_book"), &[]) > fallback);
        assert!(memory_relevance(&entry("user_context"), &[]) > fallback);
    }
}

impl MemoryLedgerError {
    pub(crate) fn allows_identity_isolated_chat(&self) -> bool {
        matches!(
            self.code,
            "memory_identity_quarantined"
                | "ledger_integrity_violation"
                | "sovereign_identity_keyring_unavailable"
                | "identity_secure_storage_error"
                | "identity_invalid_crypto_material"
        )
    }

    fn database(error: rusqlite::Error) -> Self {
        Self {
            code: "memory_database_error",
            boundary: "MemoryLedger",
            message: error.to_string(),
        }
    }

    fn quarantined_identity(message: &str) -> Self {
        Self {
            code: "memory_identity_quarantined",
            boundary: "MemoryLedger",
            message: message.to_string(),
        }
    }

    fn runtime(message: String) -> Self {
        Self {
            code: "memory_runtime_error",
            boundary: "MemoryLedger",
            message,
        }
    }

    fn invalid(message: &str) -> Self {
        Self {
            code: "memory_invalid_proposal",
            boundary: "MemoryLedger",
            message: message.to_string(),
        }
    }

    fn integrity(message: &str) -> Self {
        Self {
            code: "memory_integrity_rejected",
            boundary: "MemoryLedger",
            message: message.to_string(),
        }
    }
}

impl From<rusqlite::Error> for MemoryLedgerError {
    fn from(error: rusqlite::Error) -> Self {
        Self::database(error)
    }
}
