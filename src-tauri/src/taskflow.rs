use crate::db::{ChatTurnPersistenceContext, PersistenceEngine};
use crate::foundation::clock::unix_time_ms_i64 as unix_time_ms;
use crate::foundation::digest::sha256_hex;
use crate::gemma::{LocalDecisionDirective, LocalWorkflowDecision};
use crate::memory_ledger::MemoryLedger;
use crate::sovereign_identity::{SignatureBlock, SovereignIdentity};
use regex::Regex;
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::Emitter;

const OPS_DB_FILE: &str = "oomu_state.sqlite";
const PRIVATE_TASKFLOW_STORE_ID: &str = "private://taskflow";
const MAX_SEARCH_FILES: usize = 500;
const MAX_SEARCH_RESULTS: usize = 20;
const MAX_SEARCH_FILE_BYTES: u64 = 2_000_000;
const MAX_CONTEXT_CHARS_PER_FILE: usize = 4_000;
const MAX_SUMMARY_SOURCE_CHARS: usize = 32_000;

#[derive(Clone)]
pub struct TaskFlowEngine {
    db_path: Arc<PathBuf>,
    write_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateTaskFlowRequest {
    pub directive: String,
    pub parent_session_id: String,
    #[serde(flatten)]
    pub turn_context: TaskFlowTurnContextRequest,
    pub workflow_id: Option<String>,
    pub workflow_version: Option<i64>,
    pub workflow_name: Option<String>,
    pub workflow_nodes: Option<Vec<TaskFlowVisualNode>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecuteTaskFlowRequest {
    pub flow_id: String,
    #[serde(flatten)]
    pub turn_context: TaskFlowTurnContextRequest,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TaskFlowTurnContextRequest {
    pub turn_id: Option<String>,
    pub generation_token: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub parent_turn_id: Option<String>,
    pub root_turn_id: Option<String>,
    pub turn_kind: Option<String>,
}

impl TaskFlowTurnContextRequest {
    fn is_empty(&self) -> bool {
        self.turn_id.is_none()
            && self.generation_token.is_none()
            && self.session_id.is_none()
            && self.agent_id.is_none()
            && self.provider_id.is_none()
            && self.model_id.is_none()
            && self.parent_turn_id.is_none()
            && self.root_turn_id.is_none()
            && self.turn_kind.is_none()
    }

    fn to_persistence_context(
        &self,
        expected_session_id: Option<&str>,
    ) -> Result<Option<ChatTurnPersistenceContext>, TaskFlowError> {
        if self.is_empty() {
            return Ok(None);
        }
        let required = |field: &str, value: Option<&str>| -> Result<String, TaskFlowError> {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    TaskFlowError::invalid(&format!(
                        "Chat-delegated TaskFlow requires immutable {field}."
                    ))
                })
        };
        let session_id = required("session_id", self.session_id.as_deref())?;
        if expected_session_id.is_some_and(|expected| expected.trim() != session_id) {
            return Err(TaskFlowError::invalid(
                "TaskFlow parent_session_id does not match its immutable session_id.",
            ));
        }
        Ok(Some(ChatTurnPersistenceContext {
            turn_id: required("turn_id", self.turn_id.as_deref())?,
            generation_token: required("generation_token", self.generation_token.as_deref())?,
            session_id,
            agent_id: required("agent_id", self.agent_id.as_deref())?,
            provider_id: required("provider_id", self.provider_id.as_deref())?,
            model_id: required("model_id", self.model_id.as_deref())?,
            parent_turn_id: self
                .parent_turn_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            root_turn_id: required("root_turn_id", self.root_turn_id.as_deref())?,
            turn_kind: required("turn_kind", self.turn_kind.as_deref())?,
        }))
    }

    fn from_persistence_context(context: &ChatTurnPersistenceContext) -> Self {
        Self {
            turn_id: Some(context.turn_id.clone()),
            generation_token: Some(context.generation_token.clone()),
            session_id: Some(context.session_id.clone()),
            agent_id: Some(context.agent_id.clone()),
            provider_id: Some(context.provider_id.clone()),
            model_id: Some(context.model_id.clone()),
            parent_turn_id: context.parent_turn_id.clone(),
            root_turn_id: Some(context.root_turn_id.clone()),
            turn_kind: Some(context.turn_kind.clone()),
        }
    }
}

fn chat_turn_contexts_match(
    left: &ChatTurnPersistenceContext,
    right: &ChatTurnPersistenceContext,
) -> bool {
    left.turn_id == right.turn_id
        && left.generation_token == right.generation_token
        && left.session_id == right.session_id
        && left.agent_id == right.agent_id
        && left.provider_id == right.provider_id
        && left.model_id == right.model_id
        && left.parent_turn_id == right.parent_turn_id
        && left.root_turn_id == right.root_turn_id
        && left.turn_kind == right.turn_kind
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManualOverrideRequest {
    pub flow_id: String,
    pub step_id: String,
    pub new_premise: String,
    pub corrective_action: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartMonitorRequest {
    pub flow_id: String,
    pub parent_session_id: String,
    pub monitor_label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskFlow {
    pub flow_id: String,
    pub mission_id: String,
    pub parent_session_id: String,
    pub directive: String,
    pub status: TaskFlowStatus,
    pub steps: Vec<TaskFlowStep>,
    pub decision_nodes: Vec<DecisionNode>,
    pub heartbeats: Vec<TaskHeartbeat>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskFlowStatus {
    Queued,
    Active,
    Verified,
    Failed,
    Diagnostic,
    Paused,
    SecurePause,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskFlowStep {
    pub step_id: String,
    pub sequence: i64,
    pub status: TaskStepStatus,
    pub pre_conditions: Vec<String>,
    pub action: TaskAction,
    pub post_conditions: Vec<String>,
    pub logical_certificate: Option<LogicalCertificate>,
    pub output: Option<String>,
    pub decision_node: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStepStatus {
    Queued,
    Active,
    Verified,
    Failed,
    Skipped,
    Cancelled,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskAction {
    Search {
        query: String,
    },
    Extract {
        source: String,
    },
    Summarize {
        topic: String,
    },
    VisualNode {
        node_id: String,
        node_kind: String,
        label: String,
        detail: String,
        connector: Option<String>,
        configuration: Value,
        notes: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskFlowVisualNode {
    pub node_id: String,
    pub node_kind: String,
    pub label: String,
    pub detail: String,
    pub connector: Option<String>,
    #[serde(default)]
    pub configuration: Value,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogicalCertificate {
    pub premises: Vec<String>,
    pub execution_path: Vec<String>,
    pub formal_conclusion: String,
    pub signature: Option<SignatureBlock>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct VerifiedContentBlock {
    source: String,
    content: String,
    content_hash: String,
    byte_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StepExecutionOutput {
    action_kind: String,
    summary: String,
    content_blocks: Vec<VerifiedContentBlock>,
    model_path: Option<String>,
    completed_at_ms: i64,
}

struct StepExecutionResult {
    output: StepExecutionOutput,
    certificate: LogicalCertificate,
    thoughts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionNode {
    pub id: i64,
    pub flow_id: String,
    pub failed_step_id: String,
    pub reason: String,
    pub suggested_fix: String,
    pub status: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskHeartbeat {
    pub id: i64,
    pub flow_id: String,
    pub step_id: Option<String>,
    pub parent_session_id: String,
    pub status: String,
    pub drift_score: f32,
    pub message: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct TaskFlowState {
    pub db_path: String,
    pub flows: Vec<TaskFlow>,
}

#[cfg(test)]
#[test]
fn taskflow_state_store_id_is_opaque() {
    let serialized = serde_json::to_string(&TaskFlowState {
        db_path: PRIVATE_TASKFLOW_STORE_ID.to_string(),
        flows: Vec::new(),
    })
    .unwrap();
    assert!(serialized.contains("private://taskflow"));
    if let Some(home) = std::env::var_os("HOME") {
        assert!(!serialized.contains(&home.to_string_lossy().to_string()));
    }
}

#[derive(Debug, Serialize)]
pub struct TaskFlowExecutionResponse {
    pub flow: TaskFlow,
    pub completed_steps: usize,
    pub halted: bool,
    pub diagnostic: Option<DecisionNode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskFlowProgressEvent {
    pub flow_id: String,
    pub mission_id: String,
    pub parent_session_id: String,
    pub step_id: Option<String>,
    pub step_index: usize,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskFlowThoughtEvent {
    pub flow_id: String,
    pub mission_id: String,
    pub parent_session_id: String,
    pub step_id: String,
    pub step_index: usize,
    pub phase: String,
    pub thought: String,
}

#[derive(Debug, Serialize)]
pub struct TaskFlowError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
}

impl TaskFlowEngine {
    pub fn initialize() -> Result<Self, String> {
        let db_path = project_root().join(OPS_DB_FILE);
        Self::initialize_at(db_path)
    }

    pub(crate) fn initialize_at(db_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let engine = Self {
            db_path: Arc::new(db_path),
            write_lock: Arc::new(Mutex::new(())),
        };
        engine.run_migrations().map_err(|error| error.to_string())?;
        Ok(engine)
    }

    pub fn audit_orphans(&self) {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = engine.mark_orphaned_monitors() {
                eprintln!("TASKFLOW_ORPHAN_AUDIT_FAILED {error}");
            }
        });
    }

    async fn create_flow(
        &self,
        request: CreateTaskFlowRequest,
        persistence: PersistenceEngine,
        turn_context: ChatTurnPersistenceContext,
    ) -> Result<TaskFlow, TaskFlowError> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            persistence
                .validate_chat_turn_generation(&turn_context)
                .map_err(|error| {
                    TaskFlowError::stale(format!(
                        "TaskFlow creation rejected because its originating turn is stale: {error}"
                    ))
                })?;
            engine.create_flow_sync(request)
        })
        .await
        .map_err(|error| TaskFlowError::runtime(error.to_string()))?
    }

    async fn execute_flow(
        &self,
        request: ExecuteTaskFlowRequest,
        identity: SovereignIdentity,
        ledger: MemoryLedger,
        gemma: crate::gemma::GemmaService,
        persistence: PersistenceEngine,
        app: Option<tauri::AppHandle>,
    ) -> Result<TaskFlowExecutionResponse, TaskFlowError> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            engine.execute_flow_sync(
                request,
                identity,
                Some(ledger),
                &gemma,
                Some(&persistence),
                app,
            )
        })
        .await
        .map_err(|error| TaskFlowError::runtime(error.to_string()))?
    }

    async fn override_step(
        &self,
        request: ManualOverrideRequest,
        persistence: PersistenceEngine,
    ) -> Result<TaskFlow, TaskFlowError> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            engine.override_step_sync(request, Some(&persistence))
        })
        .await
        .map_err(|error| TaskFlowError::runtime(error.to_string()))?
    }

    async fn start_monitor(
        &self,
        request: StartMonitorRequest,
        persistence: PersistenceEngine,
    ) -> Result<TaskHeartbeat, TaskFlowError> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            engine.start_monitor_sync(request, Some(&persistence))
        })
        .await
        .map_err(|error| TaskFlowError::runtime(error.to_string()))?
    }

    async fn load_state(&self) -> Result<TaskFlowState, TaskFlowError> {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || engine.select_state())
            .await
            .map_err(|error| TaskFlowError::runtime(error.to_string()))?
            .map_err(TaskFlowError::database)
    }

    fn run_migrations(&self) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                parent_session_id TEXT,
                agent_kind TEXT NOT NULL,
                task TEXT NOT NULL,
                status TEXT NOT NULL,
                restricted_context TEXT NOT NULL,
                message_history TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS intel_ledger (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                insight TEXT NOT NULL,
                logical_certificate TEXT NOT NULL,
                committed_at_ms INTEGER NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(session_id)
            );

            CREATE TABLE IF NOT EXISTS taskflows (
                flow_id TEXT PRIMARY KEY,
                mission_id TEXT NOT NULL DEFAULT '',
                parent_session_id TEXT NOT NULL,
                directive TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                ark_path TEXT,
                final_seal_path TEXT,
                chat_turn_id TEXT,
                chat_generation_token TEXT,
                chat_session_id TEXT,
                chat_agent_id TEXT,
                chat_provider_id TEXT,
                chat_model_id TEXT,
                chat_parent_turn_id TEXT,
                chat_root_turn_id TEXT,
                chat_turn_kind TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_taskflows_mission ON taskflows(mission_id);

            CREATE TABLE IF NOT EXISTS taskflow_steps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                flow_id TEXT NOT NULL,
                step_id TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                status TEXT NOT NULL,
                pre_conditions TEXT NOT NULL,
                action_json TEXT NOT NULL,
                post_conditions TEXT NOT NULL,
                logical_certificate TEXT,
                output TEXT,
                decision_node TEXT,
                UNIQUE(flow_id, step_id),
                FOREIGN KEY(flow_id) REFERENCES taskflows(flow_id)
            );

            CREATE TABLE IF NOT EXISTS taskflow_heartbeats (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                flow_id TEXT NOT NULL,
                step_id TEXT,
                parent_session_id TEXT NOT NULL,
                status TEXT NOT NULL,
                drift_score REAL NOT NULL,
                message TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                FOREIGN KEY(flow_id) REFERENCES taskflows(flow_id)
            );

            CREATE TABLE IF NOT EXISTS taskflow_decisions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                flow_id TEXT NOT NULL,
                failed_step_id TEXT NOT NULL,
                reason TEXT NOT NULL,
                suggested_fix TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                FOREIGN KEY(flow_id) REFERENCES taskflows(flow_id)
            );

            CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);
            CREATE INDEX IF NOT EXISTS idx_intel_session_id ON intel_ledger(session_id);
            CREATE INDEX IF NOT EXISTS idx_taskflow_steps_flow ON taskflow_steps(flow_id);
            CREATE INDEX IF NOT EXISTS idx_taskflow_heartbeats_flow ON taskflow_heartbeats(flow_id);
            CREATE INDEX IF NOT EXISTS idx_taskflow_decisions_flow ON taskflow_decisions(flow_id);
            ",
        )?;
        add_column_if_missing(
            &connection,
            "taskflows",
            "mission_id",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        add_column_if_missing(&connection, "taskflows", "ark_path", "TEXT")?;
        add_column_if_missing(&connection, "taskflows", "final_seal_path", "TEXT")?;
        add_column_if_missing(&connection, "taskflows", "chat_turn_id", "TEXT")?;
        add_column_if_missing(&connection, "taskflows", "chat_generation_token", "TEXT")?;
        add_column_if_missing(&connection, "taskflows", "chat_session_id", "TEXT")?;
        add_column_if_missing(&connection, "taskflows", "chat_agent_id", "TEXT")?;
        add_column_if_missing(&connection, "taskflows", "chat_provider_id", "TEXT")?;
        add_column_if_missing(&connection, "taskflows", "chat_model_id", "TEXT")?;
        add_column_if_missing(&connection, "taskflows", "chat_parent_turn_id", "TEXT")?;
        add_column_if_missing(&connection, "taskflows", "chat_root_turn_id", "TEXT")?;
        add_column_if_missing(&connection, "taskflows", "chat_turn_kind", "TEXT")?;
        connection.execute(
            "UPDATE taskflows SET mission_id = flow_id WHERE mission_id = ''",
            [],
        )?;
        Ok(())
    }

    fn create_flow_sync(&self, request: CreateTaskFlowRequest) -> Result<TaskFlow, TaskFlowError> {
        let directive = request.directive.trim();
        let parent_session_id = request.parent_session_id.trim();
        if directive.is_empty() {
            return Err(TaskFlowError::invalid("TaskFlow directive is required."));
        }
        if parent_session_id.is_empty() {
            return Err(TaskFlowError::invalid(
                "No Zombies: TaskFlow requires a parent_session_id.",
            ));
        }
        let steps = build_steps(directive, request.workflow_nodes.as_deref().unwrap_or(&[]))?;
        let turn_context = request
            .turn_context
            .to_persistence_context(Some(parent_session_id))?
            .ok_or_else(|| {
                TaskFlowError::invalid(
                    "TaskFlow creation requires an immutable originating chat turn context.",
                )
            })?;

        let _guard = self.lock_writes();
        let mut connection = self.open_connection().map_err(TaskFlowError::database)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(TaskFlowError::database)?;
        validate_chat_turn_generation_on_connection(&tx, &turn_context)?;
        ensure_parent_session(&tx, parent_session_id, directive)?;
        let flow_id = format!("flow-{}", unix_time_ms());
        let mission_id = format!("mission-{}", unix_time_ms());
        let now = unix_time_ms();
        tx.execute(
            "
            INSERT INTO taskflows (
                flow_id, mission_id, parent_session_id, directive, status, created_at_ms, updated_at_ms,
                chat_turn_id, chat_generation_token, chat_session_id, chat_agent_id,
                chat_provider_id, chat_model_id, chat_parent_turn_id, chat_root_turn_id,
                chat_turn_kind
            )
            VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ",
            params![
                &flow_id,
                &mission_id,
                parent_session_id,
                directive,
                now,
                now,
                turn_context.turn_id,
                turn_context.generation_token,
                turn_context.session_id,
                turn_context.agent_id,
                turn_context.provider_id,
                turn_context.model_id,
                turn_context.parent_turn_id,
                turn_context.root_turn_id,
                turn_context.turn_kind,
            ],
        )
        .map_err(TaskFlowError::database)?;

        for step in steps {
            insert_step(&tx, &flow_id, &step)?;
        }
        let creation_message = taskflow_creation_message(&request);
        insert_heartbeat(
            &tx,
            &flow_id,
            None,
            parent_session_id,
            "queued",
            0.0,
            &creation_message,
        )?;
        tx.commit().map_err(TaskFlowError::database)?;
        self.select_flow(&flow_id)
    }

    fn execute_flow_sync(
        &self,
        request: ExecuteTaskFlowRequest,
        identity: SovereignIdentity,
        ledger: Option<MemoryLedger>,
        gemma: &crate::gemma::GemmaService,
        persistence: Option<&PersistenceEngine>,
        app: Option<tauri::AppHandle>,
    ) -> Result<TaskFlowExecutionResponse, TaskFlowError> {
        let _guard = self.lock_writes();
        let connection = self.open_connection().map_err(TaskFlowError::database)?;
        let mut flow = select_flow_tx(&connection, &request.flow_id)?;
        let stored_turn_context = select_taskflow_turn_context(&connection, &request.flow_id)?;
        let supplied_turn_context = request
            .turn_context
            .to_persistence_context(Some(&flow.parent_session_id))?
            .ok_or_else(|| {
                TaskFlowError::invalid(
                    "TaskFlow execution requires its immutable originating chat turn context.",
                )
            })?;
        if !chat_turn_contexts_match(&stored_turn_context, &supplied_turn_context) {
            return Err(TaskFlowError::invalid(
                "TaskFlow execution context does not match the context stored at creation.",
            ));
        }
        let persistence = persistence.ok_or_else(|| {
            TaskFlowError::stale(
                "TaskFlow cannot validate its originating chat turn persistence.".to_string(),
            )
        })?;
        validate_taskflow_turn_generation(&connection, &flow, &stored_turn_context)?;
        if matches!(
            flow.status,
            TaskFlowStatus::Diagnostic
                | TaskFlowStatus::Paused
                | TaskFlowStatus::SecurePause
                | TaskFlowStatus::Cancelled
        ) {
            return Ok(TaskFlowExecutionResponse {
                flow,
                completed_steps: 0,
                halted: true,
                diagnostic: None,
            });
        }

        self.with_guarded_turn_transition(&stored_turn_context, |transaction| {
            update_flow_status(transaction, &flow.flow_id, "active")?;
            insert_heartbeat(
                transaction,
                &flow.flow_id,
                None,
                &flow.parent_session_id,
                "active",
                0.0,
                "TaskFlow entered ACTIVE.",
            )?;
            Ok(())
        })?;
        emit_taskflow_progress(
            app.as_ref(),
            &flow,
            None,
            "active",
            "TaskFlow entered ACTIVE.",
        );
        let mut completed_steps = 0;
        let mut diagnostic = None;
        let mut verified_outputs = flow
            .steps
            .iter()
            .filter(|step| matches!(step.status, TaskStepStatus::Verified))
            .filter_map(|step| step.output.as_deref())
            .map(parse_step_output)
            .collect::<Result<Vec<_>, _>>()?;
        for step in flow.steps.clone() {
            if !matches!(step.status, TaskStepStatus::Queued) {
                continue;
            }
            validate_taskflow_turn_generation(&connection, &flow, &stored_turn_context)?;
            if let Some(ledger) = &ledger {
                if ledger
                    .task_step_completed(&flow.mission_id, &step.step_id)
                    .map_err(|error| TaskFlowError {
                        code: error.code,
                        boundary: error.boundary,
                        message: error.message,
                    })?
                {
                    let certificate = redundancy_certificate_for(&step, &identity)?;
                    let message = format!(
                        "Step {} skipped: mesh ledger already contains completion.",
                        step.step_id
                    );
                    self.with_guarded_turn_transition(&stored_turn_context, |transaction| {
                        update_step_skipped(
                            transaction,
                            &flow.flow_id,
                            &step.step_id,
                            "Skipped by mesh memory ledger redundancy check.",
                            &certificate,
                        )?;
                        insert_heartbeat(
                            transaction,
                            &flow.flow_id,
                            Some(&step.step_id),
                            &flow.parent_session_id,
                            "skipped",
                            0.0,
                            &message,
                        )?;
                        Ok(())
                    })?;
                    emit_taskflow_progress(app.as_ref(), &flow, Some(&step), "skipped", &message);
                    continue;
                }
            }
            if let Err(error) = verify_preconditions(&connection, &flow.flow_id, &step) {
                let decision =
                    self.with_guarded_turn_transition(&stored_turn_context, |transaction| {
                        let decision = create_decision(
                            transaction,
                            &flow.flow_id,
                            &step.step_id,
                            &error.message,
                            "Wait for prior Logical Certificate or inject a new premise.",
                        )?;
                        update_step_failed(
                            transaction,
                            &flow.flow_id,
                            &step.step_id,
                            &error.message,
                        )?;
                        update_flow_status(transaction, &flow.flow_id, "diagnostic")?;
                        Ok(decision)
                    })?;
                emit_taskflow_progress(app.as_ref(), &flow, Some(&step), "failed", &error.message);
                diagnostic = Some(decision);
                break;
            }

            let active_message = format!("Step {} entered ACTIVE.", step.step_id);
            self.with_guarded_turn_transition(&stored_turn_context, |transaction| {
                update_step_status(transaction, &flow.flow_id, &step.step_id, "active")?;
                insert_heartbeat(
                    transaction,
                    &flow.flow_id,
                    Some(&step.step_id),
                    &flow.parent_session_id,
                    "active",
                    0.0,
                    &active_message,
                )?;
                Ok(())
            })?;
            emit_taskflow_progress(app.as_ref(), &flow, Some(&step), "active", &active_message);

            let action_result =
                self.with_guarded_turn_transition(&stored_turn_context, |_transaction| {
                    execute_step_action(
                        &flow.flow_id,
                        &step,
                        &flow.directive,
                        &verified_outputs,
                        &identity,
                        gemma,
                        Some((persistence, &stored_turn_context)),
                    )
                });
            match action_result {
                Ok(result) => {
                    let output = serde_json::to_string_pretty(&result.output)
                        .map_err(|error| TaskFlowError::runtime(error.to_string()))?;
                    let verified_message =
                        format!("Step {} produced Logical Certificate.", step.step_id);
                    let intel_message = format!(
                        "TaskFlow {} step {} verified: {}",
                        flow.flow_id, step.step_id, result.output.summary
                    );
                    self.with_guarded_turn_transition(&stored_turn_context, |transaction| {
                        for thought in &result.thoughts {
                            insert_heartbeat(
                                transaction,
                                &flow.flow_id,
                                Some(&step.step_id),
                                &flow.parent_session_id,
                                "thought",
                                0.0,
                                thought,
                            )?;
                        }
                        update_step_verified(
                            transaction,
                            &flow.flow_id,
                            &step.step_id,
                            &output,
                            &result.certificate,
                        )?;
                        insert_intel_heartbeat(
                            transaction,
                            &flow.parent_session_id,
                            &intel_message,
                            &result.certificate,
                        )?;
                        if let Some(ledger) = &ledger {
                            ledger
                                .commit_task_step_completion(
                                    &flow.mission_id,
                                    &step.step_id,
                                    &flow.directive,
                                    &hash_certificate(&result.certificate)?,
                                )
                                .map_err(|error| TaskFlowError {
                                    code: error.code,
                                    boundary: error.boundary,
                                    message: error.message,
                                })?;
                        }
                        insert_heartbeat(
                            transaction,
                            &flow.flow_id,
                            Some(&step.step_id),
                            &flow.parent_session_id,
                            "verified",
                            0.0,
                            &verified_message,
                        )?;
                        Ok(())
                    })?;
                    for thought in &result.thoughts {
                        emit_taskflow_thought(app.as_ref(), &flow, &step, thought);
                    }
                    emit_taskflow_progress(
                        app.as_ref(),
                        &flow,
                        Some(&step),
                        "verified",
                        &verified_message,
                    );
                    verified_outputs.push(result.output);
                    completed_steps += 1;
                }
                Err(error) => {
                    if error.code == "taskflow_chat_turn_stale" {
                        mark_taskflow_cancelled(&connection, &flow, &error.message)?;
                        return Err(error);
                    }
                    let failed_message = "Diagnostic Mode entered; downstream steps halted.";
                    let decision = self.with_guarded_turn_transition(
                        &stored_turn_context,
                        |transaction| {
                            let decision = create_decision(
                                transaction,
                                &flow.flow_id,
                                &step.step_id,
                                &error.message,
                                "Replace the failed source, retry Extract, then resume dependent steps.",
                            )?;
                            update_step_failed(
                                transaction,
                                &flow.flow_id,
                                &step.step_id,
                                &error.message,
                            )?;
                            update_flow_status(transaction, &flow.flow_id, "diagnostic")?;
                            insert_heartbeat(
                                transaction,
                                &flow.flow_id,
                                Some(&step.step_id),
                                &flow.parent_session_id,
                                "failed",
                                0.0,
                                failed_message,
                            )?;
                            Ok(decision)
                        },
                    )?;
                    emit_taskflow_progress(
                        app.as_ref(),
                        &flow,
                        Some(&step),
                        "failed",
                        &error.message,
                    );
                    diagnostic = Some(decision);
                    break;
                }
            }
        }

        if diagnostic.is_none() {
            self.with_guarded_turn_transition(&stored_turn_context, |transaction| {
                update_flow_status(transaction, &flow.flow_id, "verified")
            })?;
            flow = self.select_flow(&request.flow_id)?;
            emit_taskflow_progress(
                app.as_ref(),
                &flow,
                None,
                "verified",
                "TaskFlow verified all steps without creating unapproved external artifacts.",
            );
        }
        flow = self.select_flow(&request.flow_id)?;
        Ok(TaskFlowExecutionResponse {
            flow,
            completed_steps,
            halted: diagnostic.is_some(),
            diagnostic,
        })
    }

    fn override_step_sync(
        &self,
        request: ManualOverrideRequest,
        persistence: Option<&PersistenceEngine>,
    ) -> Result<TaskFlow, TaskFlowError> {
        let _guard = self.lock_writes();
        let connection = self.open_connection().map_err(TaskFlowError::database)?;
        let _flow = select_flow_tx(&connection, &request.flow_id)?;
        let context = select_taskflow_turn_context(&connection, &request.flow_id)?;
        let _persistence = persistence.ok_or_else(|| {
            TaskFlowError::stale("TaskFlow override requires chat turn persistence.".to_string())
        })?;
        let action = TaskAction::Extract {
            source: request.corrective_action.clone(),
        };
        let action_json = json_string(&action)?;
        self.with_guarded_turn_transition(&context, |transaction| {
            transaction
                .execute(
                "
                UPDATE taskflow_steps
                SET status = 'queued',
                    action_json = ?1,
                    decision_node = ?2,
                    output = NULL,
                    logical_certificate = NULL
                WHERE flow_id = ?3 AND step_id = ?4
                ",
                params![
                    action_json,
                    request.new_premise,
                    request.flow_id,
                    request.step_id
                ],
            )
            .map_err(TaskFlowError::database)?;
            transaction
                .execute(
                "UPDATE taskflows SET status = 'queued', updated_at_ms = ?1 WHERE flow_id = ?2",
                params![unix_time_ms(), request.flow_id],
            )
            .map_err(TaskFlowError::database)?;
            transaction
                .execute(
                "UPDATE taskflow_decisions SET status = 'resolved' WHERE flow_id = ?1 AND failed_step_id = ?2",
                params![request.flow_id, request.step_id],
            )
            .map_err(TaskFlowError::database)?;
            Ok(())
        })?;
        self.select_flow(&request.flow_id)
    }

    fn start_monitor_sync(
        &self,
        request: StartMonitorRequest,
        _persistence: Option<&PersistenceEngine>,
    ) -> Result<TaskHeartbeat, TaskFlowError> {
        let _requested_monitor = (
            request.flow_id.as_str(),
            request.parent_session_id.as_str(),
            request.monitor_label.as_str(),
        );
        Err(TaskFlowError::unavailable(
            "taskflow_monitor_unavailable",
            "TaskFlow monitoring is unavailable because no production watcher is implemented; no synthetic heartbeat was recorded.",
        ))
    }

    fn mark_orphaned_monitors(&self) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_connection()?;
        connection.execute(
            "
            UPDATE taskflow_heartbeats
            SET status = 'orphaned'
            WHERE status = 'active'
            ",
            [],
        )?;
        Ok(())
    }

    pub fn spawn_recovery(
        &self,
        identity: SovereignIdentity,
        ledger: MemoryLedger,
        gemma: crate::gemma::GemmaService,
        persistence: PersistenceEngine,
    ) {
        let engine = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) =
                engine.resume_interrupted_missions(identity, ledger, gemma, persistence)
            {
                eprintln!("TASKFLOW_RECOVERY_FAILED {}", error.message);
            }
        });
    }

    fn resume_interrupted_missions(
        &self,
        identity: SovereignIdentity,
        ledger: MemoryLedger,
        gemma: crate::gemma::GemmaService,
        persistence: PersistenceEngine,
    ) -> Result<(), TaskFlowError> {
        let flow_ids = {
            let connection = self.open_connection().map_err(TaskFlowError::database)?;
            let mut statement = connection
                .prepare(
                    "
                    SELECT flow_id FROM taskflows
                    WHERE status IN ('queued', 'active', 'failed')
                    ORDER BY updated_at_ms ASC
                    LIMIT 10
                    ",
                )
                .map_err(TaskFlowError::database)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(TaskFlowError::database)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(TaskFlowError::database)?;
            rows
        };
        for flow_id in flow_ids {
            let stored_context = {
                let connection = self.open_connection().map_err(TaskFlowError::database)?;
                match select_taskflow_turn_context(&connection, &flow_id) {
                    Ok(context) => context,
                    Err(error) => {
                        if let Ok(flow) = select_flow_tx(&connection, &flow_id) {
                            let _ = mark_taskflow_cancelled(
                                &connection,
                                &flow,
                                "TaskFlow recovery cancelled a flow without immutable origin context.",
                            );
                        }
                        eprintln!(
                            "TASKFLOW_RECOVERY_SKIPPED flow_id={} error={}",
                            flow_id, error.message
                        );
                        continue;
                    }
                }
            };
            let result = self.execute_flow_sync(
                ExecuteTaskFlowRequest {
                    flow_id: flow_id.clone(),
                    turn_context: TaskFlowTurnContextRequest::from_persistence_context(
                        &stored_context,
                    ),
                },
                identity.clone(),
                Some(ledger.clone()),
                &gemma,
                Some(&persistence),
                None,
            );
            if let Err(error) = result {
                if error.code == "taskflow_chat_turn_stale" {
                    eprintln!(
                        "TASKFLOW_RECOVERY_CANCELLED flow_id={} error={}",
                        flow_id, error.message
                    );
                    continue;
                }
                return Err(error);
            }
        }
        Ok(())
    }

    fn select_state(&self) -> rusqlite::Result<TaskFlowState> {
        let connection = self.open_connection()?;
        let mut statement = connection
            .prepare("SELECT flow_id FROM taskflows ORDER BY updated_at_ms DESC LIMIT 25")?;
        let flow_ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut flows = Vec::with_capacity(flow_ids.len());
        for flow_id in flow_ids {
            flows.push(select_flow_connection(&connection, &flow_id)?);
        }

        Ok(TaskFlowState {
            db_path: PRIVATE_TASKFLOW_STORE_ID.to_string(),
            flows,
        })
    }

    fn select_flow(&self, flow_id: &str) -> Result<TaskFlow, TaskFlowError> {
        let connection = self.open_connection().map_err(TaskFlowError::database)?;
        select_flow_connection(&connection, flow_id).map_err(TaskFlowError::database)
    }

    fn with_guarded_turn_transition<T>(
        &self,
        context: &ChatTurnPersistenceContext,
        transition: impl FnOnce(&Connection) -> Result<T, TaskFlowError>,
    ) -> Result<T, TaskFlowError> {
        let mut connection = self.open_connection().map_err(TaskFlowError::database)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(TaskFlowError::database)?;
        validate_chat_turn_generation_on_connection(&transaction, context)?;
        let result = transition(&transaction)?;
        transaction.commit().map_err(TaskFlowError::database)?;
        Ok(result)
    }

    fn open_connection(&self) -> rusqlite::Result<Connection> {
        crate::db::open_state_database_connection(self.db_path.as_ref())
    }

    fn lock_writes(&self) -> std::sync::MutexGuard<'_, ()> {
        self.write_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn emit_taskflow_progress(
    app: Option<&tauri::AppHandle>,
    flow: &TaskFlow,
    step: Option<&TaskFlowStep>,
    status: &str,
    message: &str,
) {
    let Some(app) = app else {
        return;
    };
    let payload = TaskFlowProgressEvent {
        flow_id: flow.flow_id.clone(),
        mission_id: flow.mission_id.clone(),
        parent_session_id: flow.parent_session_id.clone(),
        step_id: step.map(|step| step.step_id.clone()),
        step_index: step
            .map(|step| step.sequence.saturating_sub(1).max(0) as usize)
            .unwrap_or(0),
        status: status.to_string(),
        message: message.to_string(),
    };
    if let Err(error) = app.emit("taskflow://progress", payload) {
        eprintln!(
            "TASKFLOW_PROGRESS_NOTIFICATION_FAILED flow_id={} error={}",
            flow.flow_id, error
        );
    }
}

fn emit_taskflow_thought(
    app: Option<&tauri::AppHandle>,
    flow: &TaskFlow,
    step: &TaskFlowStep,
    thought: &str,
) {
    let Some(app) = app else {
        return;
    };
    let (phase, content) = thought
        .split_once(':')
        .map(|(phase, content)| (phase.trim(), content.trim()))
        .unwrap_or(("thought", thought.trim()));
    let payload = TaskFlowThoughtEvent {
        flow_id: flow.flow_id.clone(),
        mission_id: flow.mission_id.clone(),
        parent_session_id: flow.parent_session_id.clone(),
        step_id: step.step_id.clone(),
        step_index: step.sequence.saturating_sub(1).max(0) as usize,
        phase: phase.to_string(),
        thought: content.to_string(),
    };
    if let Err(error) = app.emit("taskflow://thought", payload) {
        eprintln!(
            "TASKFLOW_THOUGHT_NOTIFICATION_FAILED flow_id={} step_id={} error={}",
            flow.flow_id, step.step_id, error
        );
    }
}

#[tauri::command]
pub async fn create_taskflow(
    request: CreateTaskFlowRequest,
    engine: tauri::State<'_, TaskFlowEngine>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<TaskFlow, TaskFlowError> {
    let turn_context = request
        .turn_context
        .to_persistence_context(Some(&request.parent_session_id))?
        .ok_or_else(|| {
            TaskFlowError::invalid(
                "TaskFlow creation requires an immutable originating chat turn context.",
            )
        })?;
    let persistence = persistence.inner().clone();
    persistence
        .begin_or_validate_running_chat_turn(&turn_context)
        .map_err(|error| {
            TaskFlowError::stale(format!(
                "TaskFlow creation rejected its originating turn: {error}"
            ))
        })?;
    let result = engine
        .create_flow(request, persistence.clone(), turn_context.clone())
        .await;
    if result.is_err() {
        let _ = persistence.finish_chat_turn(&turn_context, "failed");
    }
    result
}

#[tauri::command]
pub async fn execute_taskflow(
    request: ExecuteTaskFlowRequest,
    engine: tauri::State<'_, TaskFlowEngine>,
    ledger: tauri::State<'_, MemoryLedger>,
    identity: tauri::State<'_, SovereignIdentity>,
    gemma: tauri::State<'_, crate::gemma::GemmaService>,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<TaskFlowExecutionResponse, TaskFlowError> {
    persistence
        .require_durable_store("TaskFlow execution")
        .map_err(|message| TaskFlowError {
            code: "taskflow_volatile_persistence_blocked",
            boundary: "PersistentStateEngine",
            message,
        })?;
    engine
        .execute_flow(
            request,
            identity.inner().clone(),
            ledger.inner().clone(),
            gemma.inner().clone(),
            persistence.inner().clone(),
            Some(app),
        )
        .await
}

#[tauri::command]
pub async fn get_taskflow_state(
    engine: tauri::State<'_, TaskFlowEngine>,
) -> Result<TaskFlowState, TaskFlowError> {
    engine.load_state().await
}

#[tauri::command]
pub async fn inject_taskflow_override(
    request: ManualOverrideRequest,
    engine: tauri::State<'_, TaskFlowEngine>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<TaskFlow, TaskFlowError> {
    engine
        .override_step(request, persistence.inner().clone())
        .await
}

#[tauri::command]
pub async fn start_taskflow_monitor(
    request: StartMonitorRequest,
    engine: tauri::State<'_, TaskFlowEngine>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<TaskHeartbeat, TaskFlowError> {
    engine
        .start_monitor(request, persistence.inner().clone())
        .await
}

fn build_steps(
    directive: &str,
    workflow_nodes: &[TaskFlowVisualNode],
) -> Result<Vec<TaskFlowStep>, TaskFlowError> {
    if !workflow_nodes.is_empty() {
        return Err(TaskFlowError::unavailable(
            "taskflow_visual_node_execution_unavailable",
            "Visual workflow nodes must execute through the compiled Workflow runtime; TaskFlow does not emulate node execution.",
        ));
    }
    validate_default_taskflow_directive(directive)?;

    Ok(vec![
        TaskFlowStep {
            step_id: "search".to_string(),
            sequence: 1,
            status: TaskStepStatus::Queued,
            pre_conditions: vec!["Parent session exists in oomu_ops.db.".to_string()],
            action: TaskAction::Search {
                query: directive.to_string(),
            },
            post_conditions: vec!["Search Logical Certificate stored.".to_string()],
            logical_certificate: None,
            output: None,
            decision_node: None,
        },
        TaskFlowStep {
            step_id: "extract".to_string(),
            sequence: 2,
            status: TaskStepStatus::Queued,
            pre_conditions: vec!["search step has a Logical Certificate.".to_string()],
            action: TaskAction::Extract {
                source: "grounding_cache:latest".to_string(),
            },
            post_conditions: vec!["Extract Logical Certificate stored.".to_string()],
            logical_certificate: None,
            output: None,
            decision_node: None,
        },
        TaskFlowStep {
            step_id: "summarize".to_string(),
            sequence: 3,
            status: TaskStepStatus::Queued,
            pre_conditions: vec!["extract step has a Logical Certificate.".to_string()],
            action: TaskAction::Summarize {
                topic: directive.to_string(),
            },
            post_conditions: vec!["Summary Logical Certificate stored.".to_string()],
            logical_certificate: None,
            output: None,
            decision_node: None,
        },
    ])
}

fn validate_default_taskflow_directive(directive: &str) -> Result<(), TaskFlowError> {
    let normalized = directive.to_ascii_lowercase();
    let is_evidence_synthesis = [
        "search",
        "research",
        "review",
        "analyze",
        "analyse",
        "summarize",
        "summarise",
        "extract",
        "inspect",
        "audit",
    ]
    .iter()
    .any(|keyword| normalized.contains(keyword));
    let requests_actuation = [
        "write ",
        "create a file",
        "create the file",
        "delete ",
        "remove ",
        "rename ",
        "move ",
        "modify ",
        "edit ",
        "patch ",
        "install ",
        "uninstall ",
        "send ",
        "email ",
        "launch ",
        "run command",
        "execute command",
        "compile ",
    ]
    .iter()
    .any(|keyword| normalized.contains(keyword));

    if !is_evidence_synthesis || requests_actuation {
        return Err(TaskFlowError::unavailable(
            "taskflow_directive_unsupported",
            "The built-in TaskFlow pipeline only performs read-only local evidence synthesis. Route file writes, commands, and other actuations through an authorized execution tool.",
        ));
    }
    Ok(())
}

fn taskflow_creation_message(request: &CreateTaskFlowRequest) -> String {
    let workflow_name = request
        .workflow_name
        .as_deref()
        .map(str::trim)
        .unwrap_or("");
    let workflow_id = request.workflow_id.as_deref().map(str::trim).unwrap_or("");
    let workflow_version = request.workflow_version;
    if workflow_name.is_empty() && workflow_id.is_empty() && workflow_version.is_none() {
        return "TaskFlow DAG created and linked to parent session.".to_string();
    }

    let mut parts = vec!["TaskFlow DAG created with supplied workflow metadata.".to_string()];
    if !workflow_name.is_empty() {
        parts.push(format!("Name: {workflow_name}."));
    }
    if !workflow_id.is_empty() {
        parts.push(format!("Workflow ID: {workflow_id}."));
    }
    if let Some(version) = workflow_version {
        parts.push(format!("Compiled version: {version}."));
    }
    parts.join(" ")
}

fn execute_step_action(
    flow_id: &str,
    step: &TaskFlowStep,
    directive: &str,
    upstream_outputs: &[StepExecutionOutput],
    identity: &SovereignIdentity,
    gemma: &crate::gemma::GemmaService,
    turn_guard: Option<(&PersistenceEngine, &ChatTurnPersistenceContext)>,
) -> Result<StepExecutionResult, TaskFlowError> {
    validate_taskflow_action_turn_guard(turn_guard)?;
    let action_json = serde_json::to_string(&step.action)
        .map_err(|error| TaskFlowError::runtime(error.to_string()))?;
    let session_id = format!("taskflow-agent:{flow_id}");
    let authorization = gemma
        .generate_workflow_decision_sync(&session_id, directive, &action_json, None)
        .map_err(TaskFlowError::from_gemma)?;
    if !matches!(authorization.directive, LocalDecisionDirective::Execute) {
        return Err(TaskFlowError::step_failed(&format!(
            "Local Gemma halted step {}: {}",
            step.step_id, authorization.formal_conclusion
        )));
    }
    validate_taskflow_action_turn_guard(turn_guard)?;
    let output = match &step.action {
        TaskAction::Search { query } => execute_local_search(query)?,
        TaskAction::Extract { source } => {
            execute_local_extract(source, directive, upstream_outputs)?
        }
        TaskAction::Summarize { topic } => execute_local_summary(topic, upstream_outputs, gemma)?,
        TaskAction::VisualNode {
            node_id,
            node_kind,
            label,
            detail,
            connector,
            configuration,
            notes,
        } => execute_visual_node_action(
            node_id,
            node_kind,
            label,
            detail,
            connector.as_deref(),
            configuration,
            notes.as_deref(),
            upstream_outputs,
            gemma,
        )?,
    };
    validate_taskflow_action_turn_guard(turn_guard)?;

    let conclusion = match &step.action {
        TaskAction::Search { .. } => "Search returned verified local source content.",
        TaskAction::Extract { .. } => "Extract loaded and matched real workspace source text.",
        TaskAction::Summarize { .. } => {
            "Local Gemma inference returned a non-empty summary grounded in verified extracts."
        }
        TaskAction::VisualNode { .. } => "Visual node execution is unavailable in TaskFlow.",
    };

    let output_json = serde_json::to_string(&output)
        .map_err(|error| TaskFlowError::runtime(error.to_string()))?;
    let certification = gemma
        .generate_workflow_decision_sync(&session_id, directive, &action_json, Some(&output_json))
        .map_err(TaskFlowError::from_gemma)?;
    let thoughts = vec![
        format!("authorize: {}", authorization.thought_summary.trim()),
        format!("certify: {}", certification.thought_summary.trim()),
    ];
    let certificate = certificate_for(step, conclusion, identity, &output, certification)?;
    Ok(StepExecutionResult {
        output,
        certificate,
        thoughts,
    })
}

fn validate_taskflow_action_turn_guard(
    turn_guard: Option<(&PersistenceEngine, &ChatTurnPersistenceContext)>,
) -> Result<(), TaskFlowError> {
    let Some((persistence, context)) = turn_guard else {
        return Err(TaskFlowError::stale(
            "TaskFlow action execution requires an immutable chat turn guard.".to_string(),
        ));
    };
    persistence
        .validate_chat_turn_generation(context)
        .map_err(|error| {
            TaskFlowError::stale(format!(
                "TaskFlow action cancelled because its originating turn is stale: {error}"
            ))
        })
}

fn certificate_for(
    step: &TaskFlowStep,
    conclusion: &str,
    identity: &SovereignIdentity,
    output: &StepExecutionOutput,
    model_certificate: LocalWorkflowDecision,
) -> Result<LogicalCertificate, TaskFlowError> {
    validate_step_evidence(step, output)?;
    let output_payload =
        serde_json::to_string(output).map_err(|error| TaskFlowError::runtime(error.to_string()))?;
    let proof_hash = sha256_hex(output_payload.as_bytes());
    let mut execution_path = model_certificate.execution_path;
    execution_path.insert(0, format!("Step {} entered ACTIVE state.", step.step_id));
    execution_path.push(format!("Action executed: {}.", json_string(&step.action)?));
    execution_path.push(format!(
        "Verified {} non-empty content block(s).",
        output.content_blocks.len()
    ));
    execution_path.extend(output.content_blocks.iter().map(|block| {
        format!(
            "Verified source={} bytes={} sha256={}.",
            block.source, block.byte_count, block.content_hash
        )
    }));
    if let Some(model_path) = &output.model_path {
        execution_path.push(format!("Local model output returned from {model_path}."));
    }
    execution_path.push(format!("Cryptographic execution proof hash: {proof_hash}."));
    execution_path
        .push("Post-condition certificate generated after evidence validation.".to_string());

    let output_sha256 = model_certificate.output_sha256.ok_or_else(|| {
        TaskFlowError::runtime(
            "Local Gemma certificate omitted the verified output hash.".to_string(),
        )
    })?;
    let mut premises = step.pre_conditions.clone();
    premises.extend(model_certificate.premises);
    premises.push(format!("output_sha256={output_sha256}"));
    let mut certificate = LogicalCertificate {
        premises,
        execution_path,
        formal_conclusion: format!(
            "{} Model conclusion: {} Deterministic payload proof: {}.",
            conclusion, model_certificate.formal_conclusion, proof_hash
        ),
        signature: None,
    };
    certificate.signature = Some(
        identity
            .sign_certificate_parts(
                &certificate.premises,
                &certificate.execution_path,
                &certificate.formal_conclusion,
            )
            .map_err(|error| TaskFlowError {
                code: error.code,
                boundary: error.boundary,
                message: error.message,
            })?,
    );
    Ok(certificate)
}

fn redundancy_certificate_for(
    step: &TaskFlowStep,
    identity: &SovereignIdentity,
) -> Result<LogicalCertificate, TaskFlowError> {
    let mut certificate = LogicalCertificate {
        premises: step.pre_conditions.clone(),
        execution_path: vec![
            format!("Step {} was not re-executed.", step.step_id),
            "Mesh memory ledger reported an existing completed certificate hash for this mission step."
                .to_string(),
        ],
        formal_conclusion:
            "Redundant execution was skipped; this certificate does not attest to newly performed work."
                .to_string(),
        signature: None,
    };
    certificate.signature = Some(
        identity
            .sign_certificate_parts(
                &certificate.premises,
                &certificate.execution_path,
                &certificate.formal_conclusion,
            )
            .map_err(|error| TaskFlowError {
                code: error.code,
                boundary: error.boundary,
                message: error.message,
            })?,
    );
    Ok(certificate)
}

fn execute_local_search(query: &str) -> Result<StepExecutionOutput, TaskFlowError> {
    let matcher = query_matcher(query)?;
    let mut files = Vec::new();
    for root in approved_workspace_roots() {
        collect_search_files(&root, &mut files)?;
        if files.len() >= MAX_SEARCH_FILES {
            break;
        }
    }

    let mut content_blocks = Vec::new();
    for path in files {
        if content_blocks.len() >= MAX_SEARCH_RESULTS {
            break;
        }
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let path_text = path.to_string_lossy();
        let excerpt = matching_context(&content, &matcher, MAX_CONTEXT_CHARS_PER_FILE);
        let content = if excerpt.is_empty() && matcher.is_match(&path_text) {
            truncate_chars(content.trim(), MAX_CONTEXT_CHARS_PER_FILE)
        } else {
            excerpt
        };
        if content.trim().is_empty() {
            continue;
        }
        content_blocks.push(content_block(path_text.to_string(), content));
    }

    if content_blocks.is_empty() {
        return Err(TaskFlowError::step_failed(
            "Search completed against approved workspace directories but returned no verified content blocks.",
        ));
    }
    Ok(StepExecutionOutput {
        action_kind: "search".to_string(),
        summary: format!(
            "Local workspace search returned {} verified content block(s) for '{}'.",
            content_blocks.len(),
            query.trim()
        ),
        content_blocks,
        model_path: None,
        completed_at_ms: unix_time_ms(),
    })
}

fn execute_local_extract(
    source: &str,
    directive: &str,
    upstream_outputs: &[StepExecutionOutput],
) -> Result<StepExecutionOutput, TaskFlowError> {
    let matcher = query_matcher(directive)?;
    let sources = if source == "grounding_cache:latest" {
        upstream_outputs
            .iter()
            .rev()
            .find(|output| output.action_kind == "search")
            .map(|output| {
                output
                    .content_blocks
                    .iter()
                    .map(|block| block.source.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        vec![source.to_string()]
    };
    if sources.is_empty() {
        return Err(TaskFlowError::step_failed(
            "Extract requires a verified search result or an explicit approved workspace path.",
        ));
    }

    let mut content_blocks = Vec::new();
    for source_path in sources {
        if content_blocks.len() >= MAX_SEARCH_RESULTS {
            break;
        }
        let path = guard_workspace_file(Path::new(&source_path))?;
        let content = fs::read_to_string(&path).map_err(|error| {
            TaskFlowError::step_failed(&format!(
                "Extract could not read {}: {error}",
                path.display()
            ))
        })?;
        let context = matching_context(&content, &matcher, MAX_CONTEXT_CHARS_PER_FILE);
        if context.trim().is_empty() {
            continue;
        }
        content_blocks.push(content_block(path.to_string_lossy().to_string(), context));
    }
    if content_blocks.is_empty() {
        return Err(TaskFlowError::step_failed(
            "Extract read the approved source files but found no context matching the task directive.",
        ));
    }
    Ok(StepExecutionOutput {
        action_kind: "extract".to_string(),
        summary: format!(
            "Extract loaded {} workspace source file(s) and produced verified grounding context.",
            content_blocks.len()
        ),
        content_blocks,
        model_path: None,
        completed_at_ms: unix_time_ms(),
    })
}

fn execute_local_summary(
    topic: &str,
    upstream_outputs: &[StepExecutionOutput],
    gemma: &crate::gemma::GemmaService,
) -> Result<StepExecutionOutput, TaskFlowError> {
    let extracted = upstream_outputs
        .iter()
        .rev()
        .find(|output| output.action_kind == "extract")
        .ok_or_else(|| {
            TaskFlowError::step_failed(
                "Summarize requires verified Extract output before local inference.",
            )
        })?;
    let grounded_text = truncate_chars(
        &extracted
            .content_blocks
            .iter()
            .map(|block| format!("SOURCE: {}\n{}", block.source, block.content))
            .collect::<Vec<_>>()
            .join("\n\n"),
        MAX_SUMMARY_SOURCE_CHARS,
    );
    let inference = gemma
        .summarize_grounded_text_sync(topic, &grounded_text)
        .map_err(|error| {
            TaskFlowError::step_failed(&format!(
                "Local Gemma summary execution failed: {}",
                error.message
            ))
        })?;
    let summary = inference.text.trim();
    if summary.is_empty() {
        return Err(TaskFlowError::step_failed(
            "Local Gemma summary returned no content; no certificate was produced.",
        ));
    }
    Ok(StepExecutionOutput {
        action_kind: "summarize".to_string(),
        summary: "Local Gemma generated a grounded summary from verified extracts.".to_string(),
        content_blocks: vec![content_block(
            format!("local-gemma:{}", inference.model_path),
            summary.to_string(),
        )],
        model_path: Some(inference.model_path),
        completed_at_ms: unix_time_ms(),
    })
}

fn execute_visual_node_action(
    _node_id: &str,
    node_kind: &str,
    label: &str,
    _detail: &str,
    _connector: Option<&str>,
    _configuration: &Value,
    _notes: Option<&str>,
    _upstream_outputs: &[StepExecutionOutput],
    _gemma: &crate::gemma::GemmaService,
) -> Result<StepExecutionOutput, TaskFlowError> {
    Err(TaskFlowError::unavailable(
        "taskflow_visual_node_execution_unavailable",
        &format!(
            "Visual node '{}' ({}) cannot be executed by TaskFlow; use the compiled Workflow runtime.",
            label.trim(),
            node_kind.trim()
        ),
    ))
}

fn validate_step_evidence(
    step: &TaskFlowStep,
    output: &StepExecutionOutput,
) -> Result<(), TaskFlowError> {
    let expected_kind = match &step.action {
        TaskAction::Search { .. } => "search",
        TaskAction::Extract { .. } => "extract",
        TaskAction::Summarize { .. } => "summarize",
        TaskAction::VisualNode { .. } => "visual_node",
    };
    if output.action_kind != expected_kind
        || output.summary.trim().is_empty()
        || output.completed_at_ms <= 0
        || output.content_blocks.is_empty()
    {
        return Err(TaskFlowError::step_failed(
            "Certificate rejected: completed action evidence is missing or mismatched.",
        ));
    }
    for block in &output.content_blocks {
        if block.source.trim().is_empty()
            || block.content.trim().is_empty()
            || block.byte_count != block.content.len()
            || block.content_hash != sha256_hex(block.content.as_bytes())
        {
            return Err(TaskFlowError::step_failed(
                "Certificate rejected: a content block failed integrity validation.",
            ));
        }
    }
    match &step.action {
        TaskAction::Search { .. } | TaskAction::Extract { .. } => {
            for block in &output.content_blocks {
                guard_workspace_file(Path::new(&block.source))?;
            }
        }
        TaskAction::Summarize { .. } => {
            if output.model_path.as_deref().is_none_or(str::is_empty)
                || output
                    .content_blocks
                    .iter()
                    .any(|block| !block.source.starts_with("local-gemma:"))
            {
                return Err(TaskFlowError::step_failed(
                    "Certificate rejected: summary lacks a verified local model output.",
                ));
            }
        }
        TaskAction::VisualNode { .. } => {
            return Err(TaskFlowError::unavailable(
                "taskflow_visual_node_execution_unavailable",
                "TaskFlow cannot certify visual-node execution without the compiled Workflow runtime.",
            ));
        }
    }
    Ok(())
}

fn parse_step_output(output: &str) -> Result<StepExecutionOutput, TaskFlowError> {
    serde_json::from_str(output).map_err(|error| {
        TaskFlowError::runtime(format!(
            "Stored TaskFlow execution evidence is invalid: {error}"
        ))
    })
}

fn content_block(source: String, content: String) -> VerifiedContentBlock {
    VerifiedContentBlock {
        source,
        byte_count: content.len(),
        content_hash: sha256_hex(content.as_bytes()),
        content,
    }
}

fn query_matcher(query: &str) -> Result<Regex, TaskFlowError> {
    let mut terms = query
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .map(str::trim)
        .filter(|term| term.len() >= 3)
        .map(str::to_lowercase)
        .filter(|term| {
            !matches!(
                term.as_str(),
                "and" | "are" | "for" | "from" | "into" | "that" | "the" | "this" | "with"
            )
        })
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    if terms.is_empty() {
        return Err(TaskFlowError::step_failed(
            "Search query must contain at least one searchable term.",
        ));
    }
    let pattern = terms
        .iter()
        .map(|term| regex::escape(term))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!("(?i)(?:{pattern})"))
        .map_err(|error| TaskFlowError::step_failed(&format!("Search regex failed: {error}")))
}

fn matching_context(content: &str, matcher: &Regex, max_chars: usize) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    let mut selected = BTreeSet::new();
    for (index, line) in lines.iter().enumerate() {
        if matcher.is_match(line) {
            selected.insert(index.saturating_sub(1));
            selected.insert(index);
            if index + 1 < lines.len() {
                selected.insert(index + 1);
            }
        }
    }
    let context = selected
        .into_iter()
        .filter_map(|index| {
            lines
                .get(index)
                .map(|line| format!("L{}: {}", index + 1, line))
        })
        .collect::<Vec<_>>()
        .join("\n");
    truncate_chars(&context, max_chars)
}

fn truncate_chars(content: &str, max_chars: usize) -> String {
    content.chars().take(max_chars).collect()
}

fn approved_workspace_roots() -> Vec<PathBuf> {
    let root = project_root();
    let roots = ["workspace", "Eldris", "ark"]
        .iter()
        .map(|directory| root.join(directory))
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        vec![root]
    } else {
        roots
    }
}

fn collect_search_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), TaskFlowError> {
    if files.len() >= MAX_SEARCH_FILES {
        return Ok(());
    }
    let entries = fs::read_dir(root).map_err(|error| {
        TaskFlowError::step_failed(&format!(
            "Search could not enumerate approved directory {}: {error}",
            root.display()
        ))
    })?;
    for entry in entries {
        if files.len() >= MAX_SEARCH_FILES {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            if should_skip_directory(&path) {
                continue;
            }
            collect_search_files(&path, files)?;
        } else if is_indexable_text_file(&path)
            && fs::metadata(&path)
                .map(|metadata| metadata.len() <= MAX_SEARCH_FILE_BYTES)
                .unwrap_or(false)
        {
            files.push(path);
        }
    }
    Ok(())
}

fn should_skip_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.starts_with('.')
                || matches!(
                    name,
                    "models" | "node_modules" | "target" | "logs" | "release"
                )
        })
}

fn is_indexable_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| {
            matches!(
                extension.as_str(),
                "txt"
                    | "md"
                    | "json"
                    | "jsonl"
                    | "yaml"
                    | "yml"
                    | "toml"
                    | "csv"
                    | "xml"
                    | "html"
                    | "css"
                    | "sql"
                    | "rs"
                    | "ts"
                    | "tsx"
                    | "js"
                    | "jsx"
            )
        })
}

fn guard_workspace_file(requested: &Path) -> Result<PathBuf, TaskFlowError> {
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(TaskFlowError::step_failed(
            "Workspace source rejected because it traverses outside approved directories.",
        ));
    }
    let root = project_root();
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let canonical = candidate.canonicalize().map_err(|error| {
        TaskFlowError::step_failed(&format!(
            "Workspace source {} is unavailable: {error}",
            candidate.display()
        ))
    })?;
    if !canonical.is_file()
        || !approved_workspace_roots().iter().any(|approved| {
            approved
                .canonicalize()
                .map(|root| canonical.starts_with(root))
                .unwrap_or(false)
        })
    {
        return Err(TaskFlowError::step_failed(
            "Workspace source is outside the approved TaskFlow directories.",
        ));
    }
    Ok(canonical)
}

fn verify_preconditions(
    connection: &Connection,
    flow_id: &str,
    step: &TaskFlowStep,
) -> Result<(), TaskFlowError> {
    if step.sequence <= 1 {
        return Ok(());
    }
    let prior_verified: i64 = connection
        .query_row(
            "
            SELECT COUNT(*)
            FROM taskflow_steps
            WHERE flow_id = ?1
              AND sequence < ?2
              AND status = 'verified'
              AND logical_certificate IS NOT NULL
            ",
            params![flow_id, step.sequence],
            |row| row.get(0),
        )
        .map_err(TaskFlowError::database)?;
    if prior_verified == step.sequence - 1 {
        Ok(())
    } else {
        Err(TaskFlowError::step_failed(
            "Pre-condition failed: previous Logical Certificate is missing.",
        ))
    }
}

fn ensure_parent_session(
    connection: &Connection,
    parent_session_id: &str,
    directive: &str,
) -> Result<(), TaskFlowError> {
    connection
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                parent_session_id TEXT,
                agent_kind TEXT NOT NULL,
                task TEXT NOT NULL,
                status TEXT NOT NULL,
                restricted_context TEXT NOT NULL,
                message_history TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            ",
        )
        .map_err(TaskFlowError::database)?;
    connection
        .execute(
            "
            INSERT INTO sessions (
                session_id, parent_session_id, agent_kind, task, status,
                restricted_context, message_history, created_at_ms, updated_at_ms
            )
            VALUES (?1, NULL, 'taskflow_parent', ?2, 'active', ?3, ?4, ?5, ?6)
            ON CONFLICT(session_id) DO UPDATE SET
                task = excluded.task,
                status = 'active',
                updated_at_ms = excluded.updated_at_ms
            ",
            params![
                parent_session_id,
                directive,
                "{\"filesystem_sandbox\":\"workspace/taskflows\",\"tool_permissions\":[\"taskflow\"],\"can_access_parent_db\":false}",
                "[]",
                unix_time_ms(),
                unix_time_ms()
            ],
        )
        .map_err(TaskFlowError::database)?;
    Ok(())
}

fn update_step_skipped(
    connection: &Connection,
    flow_id: &str,
    step_id: &str,
    output: &str,
    certificate: &LogicalCertificate,
) -> Result<(), TaskFlowError> {
    let certificate_json = json_string(certificate)?;
    connection
        .execute(
            "
            UPDATE taskflow_steps
            SET status = 'skipped', output = ?1, logical_certificate = ?2
            WHERE flow_id = ?3 AND step_id = ?4
            ",
            params![output, certificate_json, flow_id, step_id],
        )
        .map_err(TaskFlowError::database)?;
    touch_flow(connection, flow_id)?;
    Ok(())
}

fn insert_step(
    connection: &Connection,
    flow_id: &str,
    step: &TaskFlowStep,
) -> Result<(), TaskFlowError> {
    let pre_conditions_json = json_string(&step.pre_conditions)?;
    let action_json = json_string(&step.action)?;
    let post_conditions_json = json_string(&step.post_conditions)?;
    connection
        .execute(
            "
            INSERT INTO taskflow_steps (
                flow_id, step_id, sequence, status, pre_conditions, action_json,
                post_conditions, logical_certificate, output, decision_node
            )
            VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?6, NULL, NULL, NULL)
            ",
            params![
                flow_id,
                &step.step_id,
                step.sequence,
                pre_conditions_json,
                action_json,
                post_conditions_json
            ],
        )
        .map_err(TaskFlowError::database)?;
    Ok(())
}

fn update_flow_status(
    connection: &Connection,
    flow_id: &str,
    status: &str,
) -> Result<(), TaskFlowError> {
    connection
        .execute(
            "UPDATE taskflows SET status = ?1, updated_at_ms = ?2 WHERE flow_id = ?3",
            params![status, unix_time_ms(), flow_id],
        )
        .map_err(TaskFlowError::database)?;
    Ok(())
}

fn update_step_status(
    connection: &Connection,
    flow_id: &str,
    step_id: &str,
    status: &str,
) -> Result<(), TaskFlowError> {
    connection
        .execute(
            "UPDATE taskflow_steps SET status = ?1 WHERE flow_id = ?2 AND step_id = ?3",
            params![status, flow_id, step_id],
        )
        .map_err(TaskFlowError::database)?;
    touch_flow(connection, flow_id)?;
    Ok(())
}

fn update_step_verified(
    connection: &Connection,
    flow_id: &str,
    step_id: &str,
    output: &str,
    certificate: &LogicalCertificate,
) -> Result<(), TaskFlowError> {
    let certificate_json = json_string(certificate)?;
    connection
        .execute(
            "
            UPDATE taskflow_steps
            SET status = 'verified', output = ?1, logical_certificate = ?2
            WHERE flow_id = ?3 AND step_id = ?4
            ",
            params![output, certificate_json, flow_id, step_id],
        )
        .map_err(TaskFlowError::database)?;
    touch_flow(connection, flow_id)?;
    Ok(())
}

fn update_step_failed(
    connection: &Connection,
    flow_id: &str,
    step_id: &str,
    reason: &str,
) -> Result<(), TaskFlowError> {
    connection
        .execute(
            "
            UPDATE taskflow_steps
            SET status = 'failed', output = ?1, decision_node = ?2
            WHERE flow_id = ?3 AND step_id = ?4
            ",
            params![reason, reason, flow_id, step_id],
        )
        .map_err(TaskFlowError::database)?;
    touch_flow(connection, flow_id)?;
    Ok(())
}

fn touch_flow(connection: &Connection, flow_id: &str) -> Result<(), TaskFlowError> {
    connection
        .execute(
            "UPDATE taskflows SET updated_at_ms = ?1 WHERE flow_id = ?2",
            params![unix_time_ms(), flow_id],
        )
        .map_err(TaskFlowError::database)?;
    Ok(())
}

fn create_decision(
    connection: &Connection,
    flow_id: &str,
    failed_step_id: &str,
    reason: &str,
    suggested_fix: &str,
) -> Result<DecisionNode, TaskFlowError> {
    connection
        .execute(
            "
            INSERT INTO taskflow_decisions (
                flow_id, failed_step_id, reason, suggested_fix, status, created_at_ms
            )
            VALUES (?1, ?2, ?3, ?4, 'open', ?5)
            ",
            params![
                flow_id,
                failed_step_id,
                reason,
                suggested_fix,
                unix_time_ms()
            ],
        )
        .map_err(TaskFlowError::database)?;
    Ok(DecisionNode {
        id: connection.last_insert_rowid(),
        flow_id: flow_id.to_string(),
        failed_step_id: failed_step_id.to_string(),
        reason: reason.to_string(),
        suggested_fix: suggested_fix.to_string(),
        status: "open".to_string(),
        created_at_ms: unix_time_ms(),
    })
}

fn insert_heartbeat(
    connection: &Connection,
    flow_id: &str,
    step_id: Option<&str>,
    parent_session_id: &str,
    status: &str,
    drift_score: f32,
    message: &str,
) -> Result<i64, TaskFlowError> {
    connection
        .execute(
            "
            INSERT INTO taskflow_heartbeats (
                flow_id, step_id, parent_session_id, status, drift_score, message, created_at_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
            params![
                flow_id,
                step_id,
                parent_session_id,
                status,
                drift_score,
                message,
                unix_time_ms()
            ],
        )
        .map_err(TaskFlowError::database)?;
    Ok(connection.last_insert_rowid())
}

fn insert_intel_heartbeat(
    connection: &Connection,
    parent_session_id: &str,
    insight: &str,
    certificate: &LogicalCertificate,
) -> Result<(), TaskFlowError> {
    let certificate_json = json_string(certificate)?;
    connection
        .execute(
            "
            INSERT INTO intel_ledger (session_id, insight, logical_certificate, committed_at_ms)
            VALUES (?1, ?2, ?3, ?4)
            ",
            params![parent_session_id, insight, certificate_json, unix_time_ms()],
        )
        .map_err(TaskFlowError::database)?;
    Ok(())
}

fn select_flow_tx(connection: &Connection, flow_id: &str) -> Result<TaskFlow, TaskFlowError> {
    select_flow_connection(connection, flow_id).map_err(TaskFlowError::database)
}

fn select_taskflow_turn_context(
    connection: &Connection,
    flow_id: &str,
) -> Result<ChatTurnPersistenceContext, TaskFlowError> {
    let values = connection
        .query_row(
            "
            SELECT chat_turn_id, chat_generation_token, chat_session_id, chat_agent_id,
                   chat_provider_id, chat_model_id, chat_parent_turn_id, chat_root_turn_id,
                   chat_turn_kind
            FROM taskflows
            WHERE flow_id = ?1
            ",
            params![flow_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .map_err(TaskFlowError::database)?;
    let required = |field: &str, value: Option<String>| -> Result<String, TaskFlowError> {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                TaskFlowError::stale(format!(
                    "TaskFlow is missing its immutable originating {field}."
                ))
            })
    };
    Ok(ChatTurnPersistenceContext {
        turn_id: required("turn_id", values.0)?,
        generation_token: required("generation_token", values.1)?,
        session_id: required("session_id", values.2)?,
        agent_id: required("agent_id", values.3)?,
        provider_id: required("provider_id", values.4)?,
        model_id: required("model_id", values.5)?,
        parent_turn_id: values.6,
        root_turn_id: required("root_turn_id", values.7)?,
        turn_kind: required("turn_kind", values.8)?,
    })
}

fn mark_taskflow_cancelled(
    connection: &Connection,
    flow: &TaskFlow,
    _reason: &str,
) -> Result<(), TaskFlowError> {
    update_flow_status(connection, &flow.flow_id, "cancelled")?;
    connection
        .execute(
            "
            UPDATE taskflow_steps
            SET status = 'cancelled'
            WHERE flow_id = ?1 AND status IN ('queued', 'active')
            ",
            params![flow.flow_id],
        )
        .map_err(TaskFlowError::database)?;
    Ok(())
}

fn validate_taskflow_turn_generation(
    connection: &Connection,
    flow: &TaskFlow,
    context: &ChatTurnPersistenceContext,
) -> Result<(), TaskFlowError> {
    if let Err(error) = validate_chat_turn_generation_on_connection(connection, context) {
        let message = format!(
            "TaskFlow cancelled because its originating chat turn is stale or deleted: {}",
            error.message
        );
        mark_taskflow_cancelled(connection, flow, &message)?;
        return Err(TaskFlowError::stale(message));
    }
    Ok(())
}

fn validate_chat_turn_generation_on_connection(
    connection: &Connection,
    context: &ChatTurnPersistenceContext,
) -> Result<(), TaskFlowError> {
    let matches: i64 = connection
        .query_row(
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
            ],
            |row| row.get(0),
        )
        .map_err(TaskFlowError::database)?;
    if matches != 1 {
        return Err(TaskFlowError::stale(
            "TaskFlow originating chat session or generation is stale or deleted.".to_string(),
        ));
    }
    Ok(())
}

fn select_flow_connection(connection: &Connection, flow_id: &str) -> rusqlite::Result<TaskFlow> {
    let (mission_id, parent_session_id, directive, status, created_at_ms, updated_at_ms): (
        String,
        String,
        String,
        String,
        i64,
        i64,
    ) = connection.query_row(
        "
        SELECT mission_id, parent_session_id, directive, status, created_at_ms, updated_at_ms
        FROM taskflows
        WHERE flow_id = ?1
        ",
        params![flow_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )?;
    Ok(TaskFlow {
        flow_id: flow_id.to_string(),
        mission_id,
        parent_session_id,
        directive,
        status: flow_status_from_str(&status),
        steps: select_steps(connection, flow_id)?,
        decision_nodes: select_decisions(connection, flow_id)?,
        heartbeats: select_heartbeats(connection, flow_id)?,
        created_at_ms,
        updated_at_ms,
    })
}

fn select_steps(connection: &Connection, flow_id: &str) -> rusqlite::Result<Vec<TaskFlowStep>> {
    let mut statement = connection.prepare(
        "
        SELECT step_id, sequence, status, pre_conditions, action_json,
               post_conditions, logical_certificate, output, decision_node
        FROM taskflow_steps
        WHERE flow_id = ?1
        ORDER BY sequence ASC
        ",
    )?;
    let rows = statement.query_map(params![flow_id], |row| {
        let status: String = row.get(2)?;
        let pre_conditions: String = row.get(3)?;
        let action_json: String = row.get(4)?;
        let post_conditions: String = row.get(5)?;
        let certificate: Option<String> = row.get(6)?;
        let pre_conditions = serde_json::from_str(&pre_conditions)
            .map_err(|error| taskflow_json_column_error(3, error))?;
        let action = serde_json::from_str(&action_json)
            .map_err(|error| taskflow_json_column_error(4, error))?;
        let post_conditions = serde_json::from_str(&post_conditions)
            .map_err(|error| taskflow_json_column_error(5, error))?;
        let logical_certificate = certificate
            .as_deref()
            .map(|value| {
                serde_json::from_str(value).map_err(|error| taskflow_json_column_error(6, error))
            })
            .transpose()?;
        Ok(TaskFlowStep {
            step_id: row.get(0)?,
            sequence: row.get(1)?,
            status: step_status_from_str(&status),
            pre_conditions,
            action,
            post_conditions,
            logical_certificate,
            output: row.get(7)?,
            decision_node: row.get(8)?,
        })
    })?;
    rows.collect()
}

fn select_decisions(connection: &Connection, flow_id: &str) -> rusqlite::Result<Vec<DecisionNode>> {
    let mut statement = connection.prepare(
        "
        SELECT id, failed_step_id, reason, suggested_fix, status, created_at_ms
        FROM taskflow_decisions
        WHERE flow_id = ?1
        ORDER BY id DESC
        ",
    )?;
    let rows = statement.query_map(params![flow_id], |row| {
        Ok(DecisionNode {
            id: row.get(0)?,
            flow_id: flow_id.to_string(),
            failed_step_id: row.get(1)?,
            reason: row.get(2)?,
            suggested_fix: row.get(3)?,
            status: row.get(4)?,
            created_at_ms: row.get(5)?,
        })
    })?;
    rows.collect()
}

fn select_heartbeats(
    connection: &Connection,
    flow_id: &str,
) -> rusqlite::Result<Vec<TaskHeartbeat>> {
    let mut statement = connection.prepare(
        "
        SELECT id, step_id, parent_session_id, status, drift_score, message, created_at_ms
        FROM taskflow_heartbeats
        WHERE flow_id = ?1
        ORDER BY id DESC
        LIMIT 30
        ",
    )?;
    let rows = statement.query_map(params![flow_id], |row| {
        Ok(TaskHeartbeat {
            id: row.get(0)?,
            flow_id: flow_id.to_string(),
            step_id: row.get(1)?,
            parent_session_id: row.get(2)?,
            status: row.get(3)?,
            drift_score: row.get(4)?,
            message: row.get(5)?,
            created_at_ms: row.get(6)?,
        })
    })?;
    rows.collect()
}

fn hash_certificate(certificate: &LogicalCertificate) -> Result<String, TaskFlowError> {
    Ok(sha256_hex(json_string(certificate)?.as_bytes()))
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|existing| existing == column) {
        connection.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

fn flow_status_from_str(status: &str) -> TaskFlowStatus {
    match status {
        "queued" => TaskFlowStatus::Queued,
        "active" => TaskFlowStatus::Active,
        "verified" => TaskFlowStatus::Verified,
        "failed" => TaskFlowStatus::Failed,
        "diagnostic" => TaskFlowStatus::Diagnostic,
        "paused" => TaskFlowStatus::Paused,
        "secure_pause" => TaskFlowStatus::SecurePause,
        "cancelled" => TaskFlowStatus::Cancelled,
        _ => TaskFlowStatus::Failed,
    }
}

fn step_status_from_str(status: &str) -> TaskStepStatus {
    match status {
        "queued" => TaskStepStatus::Queued,
        "active" => TaskStepStatus::Active,
        "verified" => TaskStepStatus::Verified,
        "failed" => TaskStepStatus::Failed,
        "skipped" => TaskStepStatus::Skipped,
        "cancelled" => TaskStepStatus::Cancelled,
        _ => TaskStepStatus::Failed,
    }
}

impl PartialEq for TaskFlowStatus {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

fn json_string<T: Serialize>(value: &T) -> Result<String, TaskFlowError> {
    serde_json::to_string(value).map_err(|error| {
        TaskFlowError::runtime(format!("TaskFlow JSON serialization failed: {error}"))
    })
}

fn taskflow_json_column_error(index: usize, error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))
}

fn project_root() -> PathBuf {
    crate::settings::app_data_root()
}

impl TaskFlowError {
    fn database(error: rusqlite::Error) -> Self {
        Self {
            code: "taskflow_database_error",
            boundary: "TaskFlowEngine",
            message: error.to_string(),
        }
    }

    fn runtime(message: String) -> Self {
        Self {
            code: "taskflow_runtime_error",
            boundary: "TaskFlowEngine",
            message,
        }
    }

    fn invalid(message: &str) -> Self {
        Self {
            code: "taskflow_invalid_request",
            boundary: "TaskFlowEngine",
            message: message.to_string(),
        }
    }

    fn unavailable(code: &'static str, message: &str) -> Self {
        Self {
            code,
            boundary: "TaskFlowRuntime",
            message: message.to_string(),
        }
    }

    fn step_failed(message: &str) -> Self {
        Self {
            code: "taskflow_step_failed",
            boundary: "DiagnosticMode",
            message: message.to_string(),
        }
    }

    fn stale(message: String) -> Self {
        Self {
            code: "taskflow_chat_turn_stale",
            boundary: "TaskFlowTurnGuard",
            message,
        }
    }

    fn from_gemma(error: crate::gemma::GemmaError) -> Self {
        Self {
            code: error.code,
            boundary: "GemmaSchema",
            message: error.message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc, Mutex};

    #[test]
    fn matching_context_keeps_neighboring_source_lines() {
        let matcher = query_matcher("certificate output").expect("compile matcher");
        let context = matching_context(
            "unrelated\ncertificate generated after validation\nfollowing detail\nignored",
            &matcher,
            4_000,
        );
        assert!(context.contains("L1: unrelated"));
        assert!(context.contains("L2: certificate generated after validation"));
        assert!(context.contains("L3: following detail"));
        assert!(!context.contains("L4: ignored"));
    }

    #[test]
    fn default_taskflow_rejects_actuation_instead_of_simulating_completion() {
        let error = build_steps(
            "Write a markdown document after reviewing the supplied evidence.",
            &[],
        )
        .expect_err("file actuation must not become a research-only TaskFlow");
        assert_eq!(error.code, "taskflow_directive_unsupported");
        assert!(error.message.contains("authorized execution tool"));

        let steps = build_steps("Summarize the explicitly supplied evidence.", &[])
            .expect("read-only evidence synthesis remains supported");
        assert_eq!(steps.len(), 3);
    }

    #[test]
    fn visual_taskflow_nodes_fail_closed_without_workflow_runtime() {
        let nodes = vec![TaskFlowVisualNode {
            node_id: "write-report".to_string(),
            node_kind: "file_write".to_string(),
            label: "Write report".to_string(),
            detail: "Write a report to disk".to_string(),
            connector: None,
            configuration: Value::Null,
            notes: None,
        }];
        let error = build_steps("Review evidence and write a report.", &nodes)
            .expect_err("TaskFlow must not compile a node schema as execution evidence");
        assert_eq!(error.code, "taskflow_visual_node_execution_unavailable");
    }

    #[test]
    fn taskflow_monitor_fails_closed_without_synthetic_heartbeat() {
        let engine = TaskFlowEngine {
            db_path: Arc::new(PathBuf::from("unused-monitor-test.sqlite")),
            write_lock: Arc::new(Mutex::new(())),
        };
        let error = engine
            .start_monitor_sync(
                StartMonitorRequest {
                    flow_id: "flow-test".to_string(),
                    parent_session_id: "session-test".to_string(),
                    monitor_label: "drift".to_string(),
                },
                None,
            )
            .expect_err("an unimplemented monitor must report unavailable");
        assert_eq!(error.code, "taskflow_monitor_unavailable");
        assert!(error.message.contains("no synthetic heartbeat"));
    }

    #[test]
    fn workspace_guard_rejects_parent_traversal() {
        let error = guard_workspace_file(Path::new("../outside.txt"))
            .expect_err("parent traversal must be rejected");
        assert!(error.message.contains("traverses outside"));
    }

    #[test]
    fn summary_evidence_rejects_tampered_content() {
        let step = TaskFlowStep {
            step_id: "summarize".to_string(),
            sequence: 3,
            status: TaskStepStatus::Active,
            pre_conditions: Vec::new(),
            action: TaskAction::Summarize {
                topic: "verified topic".to_string(),
            },
            post_conditions: Vec::new(),
            logical_certificate: None,
            output: None,
            decision_node: None,
        };
        let mut output = StepExecutionOutput {
            action_kind: "summarize".to_string(),
            summary: "Generated a local summary.".to_string(),
            content_blocks: vec![content_block(
                "local-gemma:/models/gemma-4".to_string(),
                "Verified summary text.".to_string(),
            )],
            model_path: Some("/models/gemma-4".to_string()),
            completed_at_ms: 1,
        };
        validate_step_evidence(&step, &output).expect("valid evidence");
        output.content_blocks[0].content.push_str(" tampered");
        assert!(validate_step_evidence(&step, &output).is_err());
    }

    #[test]
    fn stored_taskflow_output_rejects_invalid_evidence_json() {
        let error = parse_step_output("{not-valid-json")
            .expect_err("corrupt stored output must not be silently ignored");
        assert_eq!(error.code, "taskflow_runtime_error");
        assert!(error.message.contains("execution evidence is invalid"));
    }

    #[test]
    fn taskflow_migrations_create_intel_ledger_dependency() {
        let root = std::env::temp_dir().join(format!(
            "oomu-taskflow-migration-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp taskflow root is created");
        let engine = TaskFlowEngine {
            db_path: Arc::new(root.join("oomu_state.sqlite")),
            write_lock: Arc::new(Mutex::new(())),
        };

        engine.run_migrations().expect("taskflow migrations run");
        let connection = engine.open_connection().expect("taskflow db opens");
        ensure_parent_session(&connection, "parent-session", "Run premade workflow")
            .expect("parent session upserts");
        let certificate = LogicalCertificate {
            premises: vec!["TaskFlow migration completed.".to_string()],
            execution_path: vec!["Inserted verified intel after migration.".to_string()],
            formal_conclusion: "intel_ledger is available to TaskFlow.".to_string(),
            signature: None,
        };

        insert_intel_heartbeat(
            &connection,
            "parent-session",
            "Premade workflow completed a verified step.",
            &certificate,
        )
        .expect("intel ledger insert succeeds");

        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM intel_ledger", [], |row| row.get(0))
            .expect("intel ledger is queryable");
        assert_eq!(count, 1);

        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn taskflow_origin_guard_rejects_mismatched_create_and_deleted_execute() {
        let root = std::env::temp_dir().join(format!(
            "oomu-taskflow-turn-guard-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp taskflow root is created");
        let db_path = root.join("state.sqlite");
        let persistence = PersistenceEngine::initialize_at(db_path.clone())
            .expect("chat persistence initializes");
        let engine = TaskFlowEngine::initialize_at(db_path).expect("taskflow engine initializes");
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-taskflow".to_string(),
                provider_id: "provider-taskflow".to_string(),
                model_id: "model-taskflow".to_string(),
                title: Some("TaskFlow guard".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .expect("chat session is created");
        let context = ChatTurnPersistenceContext {
            turn_id: "turn-taskflow-root".to_string(),
            generation_token: "generation-taskflow-root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: session.provider_id.clone(),
            model_id: session.model_id.clone(),
            parent_turn_id: None,
            root_turn_id: "turn-taskflow-root".to_string(),
            turn_kind: "root".to_string(),
        };
        persistence
            .begin_or_validate_running_chat_turn(&context)
            .expect("originating turn begins");

        let mut mismatched_context = context.clone();
        mismatched_context.generation_token = "wrong-generation".to_string();
        let mismatched_request = CreateTaskFlowRequest {
            directive: "Summarize the explicitly supplied workspace evidence.".to_string(),
            parent_session_id: session.id.clone(),
            turn_context: TaskFlowTurnContextRequest::from_persistence_context(&mismatched_context),
            workflow_id: None,
            workflow_version: None,
            workflow_name: None,
            workflow_nodes: None,
        };
        let error = tauri::async_runtime::block_on(engine.create_flow(
            mismatched_request,
            persistence.clone(),
            mismatched_context,
        ))
        .expect_err("mismatched generation is rejected before flow creation");
        assert_eq!(error.code, "taskflow_chat_turn_stale");
        let connection = engine.open_connection().expect("taskflow db opens");
        let flow_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM taskflows", [], |row| row.get(0))
            .unwrap();
        assert_eq!(flow_count, 0);
        drop(connection);

        let valid_request = CreateTaskFlowRequest {
            directive: "Summarize the explicitly supplied workspace evidence.".to_string(),
            parent_session_id: session.id.clone(),
            turn_context: TaskFlowTurnContextRequest::from_persistence_context(&context),
            workflow_id: None,
            workflow_version: None,
            workflow_name: None,
            workflow_nodes: None,
        };
        let flow = tauri::async_runtime::block_on(engine.create_flow(
            valid_request,
            persistence.clone(),
            context.clone(),
        ))
        .expect("valid guarded flow is created");
        assert!(persistence
            .delete_chat_session_by_id(&session.id)
            .expect("session deletion succeeds"));

        let execute_error = engine
            .execute_flow_sync(
                ExecuteTaskFlowRequest {
                    flow_id: flow.flow_id.clone(),
                    turn_context: TaskFlowTurnContextRequest::from_persistence_context(&context),
                },
                SovereignIdentity::initialize_ephemeral(),
                None,
                &crate::gemma::GemmaService::new_loading(),
                Some(&persistence),
                None,
            )
            .expect_err("deleted origin fails before Gemma or TaskFlow effects");
        assert_eq!(execute_error.code, "taskflow_chat_turn_stale");

        let connection = engine.open_connection().expect("taskflow db reopens");
        let status: String = connection
            .query_row(
                "SELECT status FROM taskflows WHERE flow_id = ?1",
                params![flow.flow_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "cancelled");
        let live_steps: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM taskflow_steps WHERE flow_id = ?1 AND status IN ('active', 'verified')",
                params![flow.flow_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(live_steps, 0);
        let intel_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM intel_ledger", [], |row| row.get(0))
            .unwrap();
        assert_eq!(intel_count, 0);
        let recoverable_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM taskflows WHERE flow_id = ?1 AND status IN ('queued', 'active', 'failed')",
                params![flow.flow_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recoverable_count, 0);

        drop(connection);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn deletion_after_action_before_persistence_rejects_verified_taskflow_writes() {
        let root = std::env::temp_dir().join(format!(
            "oomu-taskflow-mid-execution-delete-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&root).expect("temp taskflow root is created");
        let db_path = root.join("state.sqlite");
        let persistence = PersistenceEngine::initialize_at(db_path.clone())
            .expect("chat persistence initializes");
        let engine = TaskFlowEngine::initialize_at(db_path).expect("taskflow engine initializes");
        let session = persistence
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-taskflow-race".to_string(),
                provider_id: "provider-taskflow-race".to_string(),
                model_id: "model-taskflow-race".to_string(),
                title: Some("TaskFlow deletion race".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .expect("chat session is created");
        let context = ChatTurnPersistenceContext {
            turn_id: "turn-taskflow-race".to_string(),
            generation_token: "generation-taskflow-race".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: session.provider_id.clone(),
            model_id: session.model_id.clone(),
            parent_turn_id: None,
            root_turn_id: "turn-taskflow-race".to_string(),
            turn_kind: "root".to_string(),
        };
        persistence
            .begin_or_validate_running_chat_turn(&context)
            .expect("originating turn begins");
        let flow = tauri::async_runtime::block_on(engine.create_flow(
            CreateTaskFlowRequest {
                directive: "Summarize explicitly supplied evidence.".to_string(),
                parent_session_id: session.id.clone(),
                turn_context: TaskFlowTurnContextRequest::from_persistence_context(&context),
                workflow_id: None,
                workflow_version: None,
                workflow_name: None,
                workflow_nodes: None,
            },
            persistence.clone(),
            context.clone(),
        ))
        .expect("guarded flow is created");
        let step = flow.steps.first().expect("flow has a step").clone();
        let connection = engine.open_connection().expect("taskflow db opens");
        let heartbeat_count_before_delete: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM taskflow_heartbeats WHERE flow_id = ?1",
                params![flow.flow_id],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);

        let worker_engine = engine.clone();
        let worker_context = context.clone();
        let worker_flow_id = flow.flow_id.clone();
        let worker_session_id = session.id.clone();
        let (action_complete_tx, action_complete_rx) = mpsc::channel();
        let (persist_tx, persist_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            action_complete_tx
                .send(())
                .expect("signals that the action completed");
            persist_rx
                .recv()
                .expect("waits until deletion commits before persistence");
            let certificate = LogicalCertificate {
                premises: vec!["The simulated action returned output.".to_string()],
                execution_path: vec!["Attempted guarded persistence.".to_string()],
                formal_conclusion: "The output is valid only for a current turn.".to_string(),
                signature: None,
            };
            worker_engine.with_guarded_turn_transition(&worker_context, |transaction| {
                update_step_verified(
                    transaction,
                    &worker_flow_id,
                    &step.step_id,
                    "{\"summary\":\"late result\"}",
                    &certificate,
                )?;
                insert_intel_heartbeat(
                    transaction,
                    &worker_session_id,
                    "late verified intel",
                    &certificate,
                )?;
                insert_heartbeat(
                    transaction,
                    &worker_flow_id,
                    Some(&step.step_id),
                    &worker_session_id,
                    "verified",
                    0.0,
                    "late verified heartbeat",
                )?;
                Ok(())
            })
        });

        action_complete_rx
            .recv()
            .expect("action reaches the pre-persistence boundary");
        assert!(persistence
            .delete_chat_session_by_id(&session.id)
            .expect("session deletion succeeds"));
        persist_tx
            .send(())
            .expect("releases the late persistence attempt");
        let error = worker
            .join()
            .expect("persistence worker joins")
            .expect_err("deleted origin rejects late verified transition");
        assert_eq!(error.code, "taskflow_chat_turn_stale");

        let connection = engine.open_connection().expect("taskflow db reopens");
        let status: String = connection
            .query_row(
                "SELECT status FROM taskflows WHERE flow_id = ?1",
                params![flow.flow_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "cancelled");
        let verified_steps: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM taskflow_steps WHERE flow_id = ?1 AND status = 'verified'",
                params![flow.flow_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(verified_steps, 0);
        let intel_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM intel_ledger", [], |row| row.get(0))
            .unwrap();
        assert_eq!(intel_count, 0);
        let heartbeat_count_after_attempt: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM taskflow_heartbeats WHERE flow_id = ?1",
                params![flow.flow_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(heartbeat_count_after_attempt, heartbeat_count_before_delete);

        drop(connection);
        let _ = fs::remove_dir_all(root);
    }
}
