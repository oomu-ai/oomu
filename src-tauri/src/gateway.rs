use crate::agent_manager::{AgentConfig, AgentManager};
use crate::db::{
    ChannelConfigRecord, CreateChatSessionRequest, PersistenceEngine, COMMUNITY_CHANNEL_PLATFORMS,
};
use crate::foundation::{clock::unix_time_ms_i64 as unix_time_ms, digest::sha256_hex};
use crate::gemma::GemmaService;
use crate::inference::{self, ChatTurnRequest};
use crate::knowledge::KnowledgeStore;
use crate::memory_ledger::MemoryLedger;
use crate::shield_gate::{self, RequestedAction};
use crate::sovereign_identity::SovereignIdentity;
use futures_util::{SinkExt, StreamExt};
use rand_core::{OsRng, RngCore};
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};
use tauri::{async_runtime::JoinHandle, Emitter, Manager};
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use tokio_tungstenite::{connect_async, tungstenite::Message as WebSocketMessage};
mod activation;
pub(crate) mod auto_turn;
pub(crate) use activation::{validate_channel_activation, validate_slack_channel_authority};
mod channel_status;
mod provider_receipts;
pub(crate) mod runtime_activation;
mod slack;
use activation::probe_telegram_bot;
use channel_status::{
    active_connection_state, channel_label, inactive_connection_state,
    is_supported_gateway_platform, worker_fingerprint,
};
use provider_receipts::{
    ordered_provider_receipt, send_discord_reply, send_discord_reply_with_receipt,
    send_telegram_reply, send_telegram_reply_with_receipt,
};
use slack::{send_slack_message, slack_config_from_channel, spawn_slack_worker};
const GATEWAY_WORKER_POLL_INTERVAL: Duration = Duration::from_secs(5);
const TELEGRAM_LONG_POLL_TIMEOUT_SECONDS: u64 = 30;
const TELEGRAM_INITIAL_BACKOFF: Duration = Duration::from_secs(2);
const TELEGRAM_MAX_BACKOFF: Duration = Duration::from_secs(30);
const TELEGRAM_MESSAGE_CHUNK_BYTES: usize = 3800;
const DISCORD_MESSAGE_CHUNK_BYTES: usize = 1900;
const DISCORD_GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
const DISCORD_RECONNECT_INITIAL_BACKOFF: Duration = Duration::from_secs(3);
const DISCORD_RECONNECT_MAX_BACKOFF: Duration = Duration::from_secs(15);
const DISCORD_RECONNECT_BACKOFF_STEP: Duration = Duration::from_secs(3);
const DISCORD_GATEWAY_INTENTS: i64 = (1 << 9) | (1 << 15);
const GATEWAY_WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const GATEWAY_WORKER_RESTART_OBSERVATION_MS: i64 = 1_000;
static GATEWAY_LOG_CORRELATION_KEY: OnceLock<[u8; 32]> = OnceLock::new();

pub(crate) type RoutineApprovalResolver = fn(
    &PersistenceEngine,
    GemmaService,
    tauri::AppHandle,
    String,
    String,
    bool,
) -> Result<(), String>;

static ROUTINE_APPROVAL_RESOLVER: OnceLock<RoutineApprovalResolver> = OnceLock::new();

pub(crate) fn register_routine_approval_resolver(
    resolver: RoutineApprovalResolver,
) -> Result<(), String> {
    ROUTINE_APPROVAL_RESOLVER
        .set(resolver)
        .map_err(|_| "The routine approval resolver is already registered.".to_string())
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(
        crate::tools::task_tool_runtime::TaskToolRegistration {
            operation: "configure_channel",
            validate: validate_configure_channel_tool,
            validate_resolved: validate_configure_channel_tool,
            resolve: crate::tools::task_tool_runtime::identity_resolver,
            execute: execute_configure_channel_tool,
            planner_context: Some(configure_channel_planner_context),
            schema: configure_channel_tool_schema,
            metadata: crate::tools::task_tool_runtime::TaskToolMetadata {
                description: "Verify, securely save, activate, or disable a Telegram, Discord, or Slack messaging channel after explicit user approval.",
                risk_tier: crate::tools::task_tool_runtime::TaskToolRiskTier::FileWrite,
                approval_tier: crate::tools::task_tool_runtime::TaskToolApprovalTier::Explicit,
                agent_error_code: "channel_configuration_failed",
                agent_error_boundary: "GatewayChannelConfiguration",
                execution_path: "The approved channel settings were verified, stored in the protected credential store, and reconciled with the live messaging worker.",
            },
        },
    )
}

fn configure_channel_tool_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "platform": {
                "type": "string",
                "enum": COMMUNITY_CHANNEL_PLATFORMS
            },
            "credentials_json": {
                "type": "string",
                "minLength": 2,
                "maxLength": 16384,
                "description": "A JSON object encoded as a string. Use {} to reuse settings already stored by OOMU. Telegram uses botToken; Discord uses apiKey and optional allowlistChannels; Slack uses connectorId and allowlistChannels."
            },
            "owner_id": {
                "type": "string",
                "minLength": 0,
                "maxLength": 256
            },
            "is_active": {
                "type": "boolean"
            }
        },
        "required": ["platform", "credentials_json", "owner_id", "is_active"],
        "additionalProperties": false
    })
}

fn validate_configure_channel_tool(
    arguments: Value,
) -> Result<crate::tools::task_tool_runtime::TaskToolValidation, String> {
    let mut request = serde_json::from_value::<shield_gate::ConfigureChannelRequest>(arguments)
        .map_err(|_| {
            "configure_channel arguments do not match the registered schema.".to_string()
        })?;
    request.platform = request.platform.trim().to_ascii_lowercase();
    request.owner_id = request.owner_id.trim().to_string();
    request.credentials_json = request.credentials_json.trim().to_string();
    if !COMMUNITY_CHANNEL_PLATFORMS.contains(&request.platform.as_str()) {
        return Err("configure_channel platform is not supported.".to_string());
    }
    if request.owner_id.len() > 256 || request.owner_id.contains('\0') {
        return Err("configure_channel owner_id is outside the bounded contract.".to_string());
    }
    if request.credentials_json.len() > 16_384
        || request.credentials_json.contains('\0')
        || !serde_json::from_str::<Value>(&request.credentials_json)
            .is_ok_and(|value| value.is_object())
    {
        return Err(
            "configure_channel credentials_json must contain one bounded JSON object.".to_string(),
        );
    }
    Ok(crate::tools::task_tool_runtime::TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn execute_configure_channel_tool<'a>(
    context: crate::tools::task_tool_runtime::TaskToolExecutionContext<'a>,
    arguments: Value,
) -> crate::tools::task_tool_runtime::TaskToolFuture<'a> {
    Box::pin(async move {
        let request = serde_json::from_value::<shield_gate::ConfigureChannelRequest>(arguments)
            .map_err(|_| {
                "configure_channel arguments do not match the registered schema.".to_string()
            })?;
        let existing = context
            .persistence
            .select_channel_config(&request.platform)
            .map_err(|error| error.to_string())?;
        let effective_credentials = if request.credentials_json == "{}" {
            existing
                .as_ref()
                .map(|config| config.credentials_json.clone())
                .unwrap_or_else(|| "{}".to_string())
        } else {
            request.credentials_json.clone()
        };
        let effective_owner = if request.owner_id.is_empty() {
            existing
                .as_ref()
                .and_then(|config| config.owner_id.clone())
                .unwrap_or_default()
        } else {
            request.owner_id.clone()
        };
        let app = context
            .app
            .ok_or_else(|| "configure_channel requires the OOMU desktop runtime.".to_string())?;
        if request.is_active {
            validate_channel_activation(
                &request.platform,
                &effective_credentials,
                Some(&effective_owner),
            )
            .await?;
            validate_slack_channel_authority(
                &request.platform,
                &effective_credentials,
                Some(&effective_owner),
                context.persistence.clone(),
                app.state::<SovereignIdentity>().inner().clone(),
            )
            .await?;
        }
        let gateway = app.state::<SovereignGatewayService>();
        let credentials_json = if request.credentials_json == "{}" {
            None
        } else {
            Some(request.credentials_json.clone())
        };
        let owner_id = (!request.owner_id.is_empty()).then(|| request.owner_id.clone());
        let saved = context
            .persistence
            .upsert_channel_config(crate::db::SaveChannelConfigRequest {
                platform: request.platform.clone(),
                is_active: request.is_active,
                credentials_json,
                owner_id,
            })
            .map_err(|error| error.to_string())?;
        gateway.enable_workers_for_explicit_connection_action();
        gateway.refresh_workers(context.persistence).await?;
        let statuses = gateway.snapshot_statuses(context.persistence).await?;
        let status = statuses
            .into_iter()
            .find(|status| status.platform == request.platform)
            .ok_or_else(|| {
                "configure_channel could not verify the saved worker state.".to_string()
            })?;
        let verified = saved.is_active == request.is_active
            && status.is_active == request.is_active
            && status.connection_state != "error"
            && status.connection_state != "unsupported";
        if !verified {
            return Err("configure_channel could not verify the live channel state.".to_string());
        }
        let detail = if !request.is_active {
            "Channel disabled"
        } else {
            "Bot linked successfully"
        };
        let receipt = json!({
            "status": "completed",
            "verified": true,
            "platform": request.platform,
            "isActive": request.is_active,
            "connectionState": status.connection_state,
            "workerState": status.worker_state,
            "detail": detail,
        });
        Ok(shield_gate::ExecuteCommandResponse {
            operation: "configure_channel".to_string(),
            status: shield_gate::CommandStatus::Completed,
            message: receipt.to_string(),
            metrics: None,
            claims: vec![format!(
                "CLAIM channel_configuration_verified=true platform={} active={} owner_lock_sha256={}",
                status.platform,
                request.is_active,
                sha256_hex(effective_owner.as_bytes())
            )],
            verified: true,
            model_used: None,
        })
    })
}

fn configure_channel_planner_context(
    persistence: &PersistenceEngine,
    _session_id: &str,
) -> Result<Option<String>, String> {
    let states = persistence
        .select_channel_config_summaries()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|config| {
            json!({
                "platform": config.platform,
                "configured": config.credential_configured,
                "active": config.is_active,
            })
        })
        .collect::<Vec<_>>();
    Ok(Some(format!(
        "Messaging channel changes use configure_channel and always require approval. Current non-secret channel state: {}. To reuse a saved account, set credentials_json to \"{{}}\" and owner_id to an empty string; native code resolves both privately. Never invent credentials or account identifiers.",
        Value::Array(states)
    )))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayIncomingMessage {
    pub platform: String,
    pub sender_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    pub body: String,
    pub message_id: Option<String>,
    pub received_at_ms: i64,
    #[serde(default)]
    pub requested_actions: Vec<RequestedAction>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayChannelStatus {
    pub platform: String,
    pub label: String,
    pub is_active: bool,
    pub connection_state: String,
    pub owner_id: Option<String>,
    pub allowed_channel_ids: Vec<String>,
    pub worker_state: String,
    pub last_checked_at_ms: Option<i64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GatewayChannelStatusLogEvent {
    status: GatewayChannelStatus,
    timestamp_ms: i64,
    trace: String,
}

struct GatewayServiceInner {
    statuses: HashMap<String, GatewayChannelStatus>,
    workers: HashMap<String, GatewayWorker>,
    worker_fingerprints: HashMap<String, String>,
    worker_restart_after_ms: HashMap<String, i64>,
    worker_restart_suppressed: HashMap<String, String>,
    app_handle: Option<tauri::AppHandle>,
}

struct GatewayWorker {
    handle: JoinHandle<()>,
    finished: Arc<AtomicBool>,
}

struct GatewayWorkerCompletionGuard(Arc<AtomicBool>);

impl Drop for GatewayWorkerCompletionGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[derive(Clone)]
pub struct SovereignGatewayService {
    incoming_sender: mpsc::Sender<GatewayIncomingMessage>,
    inner: Arc<Mutex<GatewayServiceInner>>,
    refresh_lock: Arc<AsyncMutex<()>>,
    shutting_down: Arc<AtomicBool>,
    workers_enabled: Arc<AtomicBool>,
    app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
    http_client: HttpClient,
    agent_manager: AgentManager,
    knowledge_store: KnowledgeStore,
    memory_ledger: MemoryLedger,
    identity: SovereignIdentity,
    gemma: GemmaService,
    safe_mode: bool,
}

impl SovereignGatewayService {
    pub fn initialize(
        persistence: PersistenceEngine,
        agent_manager: AgentManager,
        knowledge_store: KnowledgeStore,
        memory_ledger: MemoryLedger,
        identity: SovereignIdentity,
        gemma: GemmaService,
        safe_mode: bool,
    ) -> Self {
        let (incoming_sender, mut incoming_receiver) = mpsc::channel(128);
        let service = Self {
            incoming_sender,
            inner: Arc::new(Mutex::new(GatewayServiceInner {
                statuses: default_status_map(),
                workers: HashMap::new(),
                worker_fingerprints: HashMap::new(),
                worker_restart_after_ms: HashMap::new(),
                worker_restart_suppressed: HashMap::new(),
                app_handle: None,
            })),
            refresh_lock: Arc::new(AsyncMutex::new(())),
            shutting_down: Arc::new(AtomicBool::new(false)),
            workers_enabled: Arc::new(AtomicBool::new(false)),
            app_handle: Arc::new(Mutex::new(None)),
            http_client: HttpClient::new(),
            agent_manager,
            knowledge_store,
            memory_ledger,
            identity,
            gemma,
            safe_mode,
        };

        let dispatcher_service = service.clone();
        let dispatcher_persistence = persistence.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(message) = incoming_receiver.recv().await {
                dispatcher_service
                    .dispatch_incoming_message(dispatcher_persistence.clone(), message)
                    .await;
            }
        });

        let supervisor_service = service.clone();
        let supervisor_persistence = persistence.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(GATEWAY_WORKER_POLL_INTERVAL).await;
                if supervisor_service.shutting_down.load(Ordering::Acquire) {
                    break;
                }
                if !supervisor_service.workers_are_enabled() {
                    continue;
                }
                if let Err(error) = supervisor_service
                    .refresh_workers(&supervisor_persistence)
                    .await
                {
                    eprintln!(
                        "SOVEREIGN_GATEWAY_SUPERVISOR_REFRESH_FAILED error={}",
                        compact_log_text(&error, 160)
                    );
                }
            }
        });

        eprintln!("SOVEREIGN_GATEWAY_SERVICE_READY");
        service
    }

    pub async fn refresh_workers(&self, persistence: &PersistenceEngine) -> Result<(), String> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Ok(());
        }
        let _refresh_guard = self.refresh_lock.lock().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return Ok(());
        }
        let active_configs = persistence
            .select_active_channel_configs()
            .map_err(|error| error.to_string())?;
        let active_platforms = active_configs
            .iter()
            .map(|config| config.platform.clone())
            .collect::<HashSet<_>>();

        let mut workers_to_stop = Vec::new();
        {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| "Gateway service lock poisoned.".to_string())?;
            let finished_workers = inner
                .workers
                .iter()
                .filter_map(|(platform, worker)| {
                    worker
                        .finished
                        .load(Ordering::Acquire)
                        .then(|| platform.clone())
                })
                .collect::<Vec<_>>();
            for platform in finished_workers {
                if let Some(worker) = inner.workers.remove(&platform) {
                    workers_to_stop.push(worker);
                }
                inner.worker_fingerprints.remove(&platform);
                if !inner.worker_restart_suppressed.contains_key(&platform) {
                    inner.worker_restart_after_ms.insert(
                        platform.clone(),
                        unix_time_ms() + GATEWAY_WORKER_RESTART_OBSERVATION_MS,
                    );
                }
            }
            for config in &active_configs {
                let platform = config.platform.clone();
                let mut status = status_from_config(config);
                let current_fingerprint = inner.worker_fingerprints.get(&platform).cloned();
                let next_fingerprint = worker_fingerprint(config);
                let restart_deferred = inner
                    .worker_restart_after_ms
                    .get(&platform)
                    .is_some_and(|restart_after| *restart_after > unix_time_ms());
                let restart_suppression = inner.worker_restart_suppressed.get(&platform).cloned();
                if let Some(reason) = restart_suppression.as_deref() {
                    apply_restart_suppressed_status(&mut status, reason);
                } else if config.is_active
                    && !inner.workers.contains_key(&platform)
                    && restart_deferred
                {
                    if let Some(existing) = inner.statuses.get(&platform) {
                        status = existing.clone();
                    }
                } else if config.is_active
                    && inner.workers.contains_key(&platform)
                    && current_fingerprint.as_deref() != Some(next_fingerprint.as_str())
                {
                    if let Some(worker) = inner.workers.remove(&platform) {
                        workers_to_stop.push(worker);
                    }
                    inner.worker_fingerprints.remove(&platform);
                    inner.worker_restart_after_ms.remove(&platform);
                    status.worker_state = "restarting".to_string();
                } else if inner.workers.contains_key(&platform) {
                    if let Some(existing) = inner.statuses.get(&platform) {
                        status.worker_state = existing.worker_state.clone();
                        status.connection_state = existing.connection_state.clone();
                        status.detail = existing.detail.clone();
                    }
                }
                inner.statuses.insert(platform, status);
            }

            let inactive_workers = inner
                .workers
                .keys()
                .filter(|platform| !active_platforms.contains(*platform))
                .cloned()
                .collect::<Vec<_>>();
            for platform in inactive_workers {
                if let Some(worker) = inner.workers.remove(&platform) {
                    workers_to_stop.push(worker);
                }
                inner.worker_fingerprints.remove(&platform);
                inner.worker_restart_after_ms.remove(&platform);
                let restart_suppression = inner.worker_restart_suppressed.get(&platform).cloned();
                if let Some(status) = inner.statuses.get_mut(&platform) {
                    status.worker_state = "idle".to_string();
                    status.is_active = false;
                    status.connection_state = inactive_connection_state(&platform).to_string();
                    status.last_checked_at_ms = Some(unix_time_ms());
                    status.detail = None;
                    if let Some(reason) = restart_suppression.as_deref() {
                        apply_restart_suppressed_status(status, reason);
                    }
                }
            }
        }

        for worker in workers_to_stop {
            await_gateway_worker_shutdown(worker).await;
        }

        for config in active_configs {
            let restart_blocked = {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|_| "Gateway service lock poisoned.".to_string())?;
                inner
                    .worker_restart_after_ms
                    .get(&config.platform)
                    .is_some_and(|restart_after| *restart_after > unix_time_ms())
                    || inner
                        .worker_restart_suppressed
                        .contains_key(&config.platform)
            };
            if restart_blocked {
                continue;
            }
            self.ensure_worker(persistence.clone(), config)?;
        }
        Ok(())
    }

    pub fn shutdown_workers(&self) {
        self.shutting_down.store(true, Ordering::Release);
        tauri::async_runtime::block_on(async {
            let _refresh_guard = self.refresh_lock.lock().await;
            let handles = self
                .inner
                .lock()
                .map(|mut inner| {
                    inner.worker_fingerprints.clear();
                    inner.worker_restart_after_ms.clear();
                    inner.worker_restart_suppressed.clear();
                    inner
                        .workers
                        .drain()
                        .map(|(_, worker)| worker)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for worker in handles {
                await_gateway_worker_shutdown(worker).await;
            }
        });
    }

    pub async fn snapshot_statuses(
        &self,
        persistence: &PersistenceEngine,
    ) -> Result<Vec<GatewayChannelStatus>, String> {
        self.refresh_workers(persistence).await?;
        let inner = self
            .inner
            .lock()
            .map_err(|_| "Gateway service lock poisoned.".to_string())?;
        Ok(COMMUNITY_CHANNEL_PLATFORMS
            .iter()
            .filter_map(|platform| inner.statuses.get(*platform).cloned())
            .collect())
    }

    fn ensure_worker(
        &self,
        persistence: PersistenceEngine,
        config: ChannelConfigRecord,
    ) -> Result<(), String> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Ok(());
        }
        let platform = config.platform.clone();
        if !is_supported_gateway_platform(&platform) {
            self.update_channel_runtime_status(
                &platform,
                "unsupported",
                "unsupported",
                Some("gateway_platform_unsupported"),
            );
            return Err(format!("gateway_platform_unsupported:{platform}"));
        }
        let fingerprint = worker_fingerprint(&config);
        {
            let inner = self
                .inner
                .lock()
                .map_err(|_| "Gateway service lock poisoned.".to_string())?;
            if inner.workers.contains_key(&platform) {
                return Ok(());
            }
        }

        let finished = Arc::new(AtomicBool::new(false));
        let completion_flag = Arc::clone(&finished);
        let handle = if platform == "telegram" {
            let credentials = match telegram_credentials_from_config(&config) {
                Ok(credentials) => credentials,
                Err(error) => {
                    self.update_channel_runtime_status(
                        &platform,
                        "idle",
                        "error",
                        Some(error.as_str()),
                    );
                    eprintln!(
                        "SOVEREIGN_GATEWAY_WORKER_CONFIG_INVALID platform=telegram error={}",
                        compact_log_text(&error, 160)
                    );
                    return Ok(());
                }
            };
            spawn_telegram_worker(
                Arc::clone(&self.inner),
                self.incoming_sender.clone(),
                self.http_client.clone(),
                credentials,
                completion_flag,
            )
        } else if platform == "discord" {
            let credentials = match discord_credentials_from_config(&config) {
                Ok(credentials) => credentials,
                Err(error) => {
                    self.update_channel_runtime_status(
                        &platform,
                        "idle",
                        "error",
                        Some(error.as_str()),
                    );
                    eprintln!(
                        "SOVEREIGN_GATEWAY_WORKER_CONFIG_INVALID platform=discord error={}",
                        compact_log_text(&error, 160)
                    );
                    return Ok(());
                }
            };
            spawn_discord_worker(
                Arc::clone(&self.inner),
                self.incoming_sender.clone(),
                credentials,
                completion_flag,
            )
        } else if platform == "slack" {
            let settings = match slack_config_from_channel(&config) {
                Ok(settings) => settings,
                Err(error) => {
                    self.update_channel_runtime_status(
                        &platform,
                        "idle",
                        "error",
                        Some(error.as_str()),
                    );
                    return Ok(());
                }
            };
            spawn_slack_worker(
                Arc::clone(&self.inner),
                self.incoming_sender.clone(),
                persistence,
                self.identity.clone(),
                settings,
                completion_flag,
            )
        } else {
            return Err(format!("gateway_platform_unsupported:{platform}"));
        };

        let worker = GatewayWorker { handle, finished };
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Gateway service lock poisoned.".to_string())?;
        if self.shutting_down.load(Ordering::Acquire) {
            let shutdown_worker_key = if inner.workers.contains_key(&platform) {
                format!("{platform}#shutdown-{}", random_secret_hex())
            } else {
                platform.clone()
            };
            inner.workers.insert(shutdown_worker_key, worker);
            return Ok(());
        }
        if inner.workers.contains_key(&platform) {
            stop_gateway_worker(worker);
            return Ok(());
        }
        inner
            .worker_fingerprints
            .insert(platform.clone(), fingerprint);
        inner.worker_restart_after_ms.remove(&platform);
        inner.workers.insert(platform.clone(), worker);
        if let Some(status) = inner.statuses.get_mut(&platform) {
            status.worker_state = "connecting".to_string();
            status.connection_state = inactive_connection_state(&platform).to_string();
            status.last_checked_at_ms = Some(unix_time_ms());
            status.detail = Some("checking_connection".to_string());
        }
        Ok(())
    }

    async fn dispatch_incoming_message(
        &self,
        persistence: PersistenceEngine,
        message: GatewayIncomingMessage,
    ) {
        match shield_gate::verify_gateway_message_allowlist(&persistence, &message) {
            Ok(decision) if decision.allowed => {
                let remote_filter =
                    shield_gate::filter_gateway_remote_actions(&message.requested_actions);
                if !remote_filter.blocked_actions.is_empty() {
                    eprintln!(
                        "SOVEREIGN_GATEWAY_ACTION_BLOCKED platform={} blocked_actions={}",
                        message.platform,
                        remote_filter.blocked_actions.len()
                    );
                    return;
                }
                if !remote_filter.confirmation_required_actions.is_empty() {
                    eprintln!(
                        "SOVEREIGN_GATEWAY_ACTION_CONFIRMATION_REQUIRED platform={} pending_actions={}",
                        message.platform,
                        remote_filter.confirmation_required_actions.len()
                    );
                    return;
                }
                let receipt_message_id = message
                    .message_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|message_id| !message_id.is_empty())
                    .map(str::to_string);
                if let Some(message_id) = receipt_message_id.as_deref() {
                    match persistence.claim_gateway_message(
                        &message.platform,
                        message_id,
                        message.received_at_ms,
                    ) {
                        Ok(true) => {}
                        Ok(false) => {
                            eprintln!(
                                "SOVEREIGN_GATEWAY_MESSAGE_DROPPED platform={} reason=duplicate message_id_hash={}",
                                message.platform,
                                keyed_gateway_correlation_hash("message-id", message_id)
                            );
                            return;
                        }
                        Err(error) => {
                            eprintln!(
                                "SOVEREIGN_GATEWAY_MESSAGE_REJECTED platform={} reason=dedupe_unavailable error={}",
                                message.platform,
                                compact_log_text(&error.to_string(), 160)
                            );
                            return;
                        }
                    }
                }
                eprintln!(
                    "SOVEREIGN_GATEWAY_MESSAGE_ACCEPTED platform={} message_id_hash={}",
                    message.platform,
                    keyed_gateway_correlation_hash(
                        "message-id",
                        message.message_id.as_deref().unwrap_or("none")
                    )
                );
                let platform = message.platform.clone();
                let message_id = message
                    .message_id
                    .clone()
                    .unwrap_or_else(|| "none".to_string());
                let receipt_persistence = persistence.clone();
                let remote_approval = persistence.resolve_remote_routine_approval(
                    &message.platform,
                    &message.sender_id,
                    &message.body,
                );
                let result = match remote_approval {
                    Ok(Some(resolution)) => {
                        let app = self.app_handle();
                        match app {
                            Ok(app) => {
                                let engine = persistence.clone();
                                let gemma = self.gemma.clone();
                                let instance_id = resolution.instance_id.clone();
                                let approval_token = resolution.approval_token.clone();
                                let approve = resolution.approve;
                                let resolved = tauri::async_runtime::spawn_blocking(move || {
                                    ROUTINE_APPROVAL_RESOLVER.get().ok_or_else(|| {
                                        "The routine approval resolver is unavailable.".to_string()
                                    })?(
                                        &engine, gemma, app, instance_id, approval_token, approve
                                    )
                                })
                                .await
                                .map_err(|error| error.to_string())
                                .and_then(|value| value);
                                match resolved {
                                    Ok(_) => persistence
                                        .reconcile_remote_workflow_task(&resolution.instance_id)
                                        .and_then(|_| {
                                            persistence
                                                .complete_remote_routine_approval(&resolution)
                                        })
                                        .and_then(|_| {
                                            Ok(if resolution.approve {
                                                "Approved action completed.".to_string()
                                            } else {
                                                "Action denied. The task remains stopped."
                                                    .to_string()
                                            })
                                        })
                                        .and_then(|response| {
                                            tauri::async_runtime::block_on(
                                                self.send_direct_response(
                                                    persistence,
                                                    &message,
                                                    &response,
                                                ),
                                            )
                                        }),
                                    Err(error) => Err(error),
                                }
                            }
                            Err(error) => Err(error),
                        }
                    }
                    Err(error) => {
                        self.send_direct_response(persistence, &message, &error)
                            .await
                    }
                    Ok(None) => {
                        let remote_control =
                            persistence.handle_remote_routine_control(&message.body);
                        match remote_control {
                            Ok(Some(response)) => {
                                self.send_direct_response(persistence, &message, &response)
                                    .await
                            }
                            Err(error) => {
                                self.send_direct_response(persistence, &message, &error)
                                    .await
                            }
                            Ok(None) => match platform.as_str() {
                                "telegram" => {
                                    self.dispatch_telegram_message(persistence, message).await
                                }
                                "discord" => {
                                    self.dispatch_discord_message(persistence, message).await
                                }
                                "slack" => self.dispatch_slack_message(persistence, message).await,
                                _ => Err("outbound_delivery_unsupported".to_string()),
                            },
                        }
                    }
                };
                if let Some(receipt_message_id) = receipt_message_id.as_deref() {
                    if let Err(error) = receipt_persistence.finish_gateway_message(
                        &platform,
                        receipt_message_id,
                        result.is_ok(),
                    ) {
                        eprintln!(
                            "SOVEREIGN_GATEWAY_MESSAGE_RECEIPT_FINALIZE_FAILED platform={} message_id_hash={} error={}",
                            platform,
                            keyed_gateway_correlation_hash("message-id", receipt_message_id),
                            compact_log_text(&error.to_string(), 160)
                        );
                    }
                }
                if let Err(error) = result {
                    eprintln!(
                        "SOVEREIGN_GATEWAY_MESSAGE_DELIVERY_FAILED platform={} message_id_hash={} error={}",
                        platform,
                        keyed_gateway_correlation_hash("message-id", &message_id),
                        compact_log_text(&error, 160)
                    );
                }
            }
            Ok(decision) => {
                eprintln!(
                    "SOVEREIGN_GATEWAY_MESSAGE_DROPPED platform={} reason={}",
                    message.platform,
                    compact_log_text(&decision.reason, 160)
                );
            }
            Err(error) => {
                eprintln!(
                    "SOVEREIGN_GATEWAY_MESSAGE_REJECTED platform={} code={} message={}",
                    message.platform,
                    error.code,
                    compact_log_text(&error.message, 160)
                );
            }
        }
    }

    async fn send_direct_response(
        &self,
        persistence: PersistenceEngine,
        message: &GatewayIncomingMessage,
        response: &str,
    ) -> Result<(), String> {
        match message.platform.as_str() {
            "telegram" => {
                let credentials = load_telegram_credentials(persistence).await?;
                send_telegram_reply(
                    &self.http_client,
                    &credentials.bot_token,
                    &message.sender_id,
                    response,
                )
                .await
            }
            "discord" => {
                let credentials = load_discord_credentials(persistence).await?;
                let channel = message
                    .channel_id
                    .as_deref()
                    .ok_or_else(|| "discord_channel_id_missing".to_string())?;
                send_discord_reply(&self.http_client, &credentials.bot_token, channel, response)
                    .await
            }
            "slack" => {
                let channel = message
                    .channel_id
                    .as_deref()
                    .ok_or_else(|| "slack_channel_id_missing".to_string())?;
                self.send_approved_slack_message(persistence, channel, response)
                    .await
                    .map(|_| ())
            }
            _ => Err("outbound_delivery_unsupported".to_string()),
        }
    }

    async fn dispatch_telegram_message(
        &self,
        persistence: PersistenceEngine,
        message: GatewayIncomingMessage,
    ) -> Result<(), String> {
        let credentials = load_telegram_credentials(persistence.clone()).await?;
        let response_text = self.run_remote_chat_turn(persistence, &message).await?;
        send_telegram_reply(
            &self.http_client,
            &credentials.bot_token,
            &message.sender_id,
            &response_text,
        )
        .await?;
        eprintln!(
            "SOVEREIGN_GATEWAY_MESSAGE_DELIVERED platform=telegram chat_id_hash={}",
            keyed_gateway_correlation_hash("telegram-chat-id", &message.sender_id)
        );
        Ok(())
    }

    async fn dispatch_discord_message(
        &self,
        persistence: PersistenceEngine,
        message: GatewayIncomingMessage,
    ) -> Result<(), String> {
        let credentials = load_discord_credentials(persistence.clone()).await?;
        let response_text = self.run_remote_chat_turn(persistence, &message).await?;
        let channel_id = message
            .channel_id
            .as_deref()
            .ok_or_else(|| "discord_channel_id_missing".to_string())?;
        send_discord_reply(
            &self.http_client,
            &credentials.bot_token,
            channel_id,
            &response_text,
        )
        .await?;
        eprintln!(
            "SOVEREIGN_GATEWAY_MESSAGE_DELIVERED platform=discord channel_id_hash={}",
            keyed_gateway_correlation_hash("discord-channel-id", channel_id)
        );
        Ok(())
    }

    async fn dispatch_slack_message(
        &self,
        persistence: PersistenceEngine,
        message: GatewayIncomingMessage,
    ) -> Result<(), String> {
        let response_text = self
            .run_remote_chat_turn(persistence.clone(), &message)
            .await?;
        let channel_id = message
            .channel_id
            .as_deref()
            .ok_or_else(|| "slack_channel_id_missing".to_string())?;
        let provider_message_id = self
            .send_approved_slack_message(persistence, channel_id, &response_text)
            .await?;
        eprintln!(
            "SOVEREIGN_GATEWAY_MESSAGE_DELIVERED platform=slack channel_id_hash={} message_id_hash={}",
            keyed_gateway_correlation_hash("slack-channel-id", channel_id),
            keyed_gateway_correlation_hash("slack-message-id", &provider_message_id)
        );
        Ok(())
    }

    async fn send_approved_slack_message(
        &self,
        persistence: PersistenceEngine,
        channel_id: &str,
        body: &str,
    ) -> Result<String, String> {
        let app = self.app_handle()?;
        let approvals = app.state::<shield_gate::ShieldApprovalManager>();
        let preview = json!({"channel": channel_id, "text": body}).to_string();
        shield_gate::request_user_approval(
            &app,
            approvals.inner(),
            shield_gate::ShieldApprovalRequest {
                approval_token: format!("slack_{}", random_secret_hex()),
                session_id: None,
                turn_id: None,
                generation_token: None,
                action_type: "connector_write".to_string(),
                action_label: "slack.post".to_string(),
                target_path: None,
                principal: Some("https://slack.com".to_string()),
                risk_tier: "consequential".to_string(),
                reason: "This posts the exact message shown to the approved Slack conversation."
                    .to_string(),
                estimated_token_costs: None,
                requested_at_ms: unix_time_ms().max(0) as u64,
                preview: preview.clone(),
                semantic_summary: "Approve this Slack message".to_string(),
                semantic_detail: "OOMU will send only the message and destination shown."
                    .to_string(),
                approval_tier: "effectful".to_string(),
                approval_mode: "one_time".to_string(),
                diff_preview: None,
                scope_trust_available: false,
                scope_trust_prefix: None,
                scope_trust_duration_ms: 0,
                project_id: None,
                task_run_id: None,
                action_class: "connector_write".to_string(),
                argument_class: crate::approval_scopes::argument_class("connector_write", &preview),
                canonical_resource: Some(channel_id.to_string()),
                mandatory_reconfirm: true,
                approval_scope_kinds: vec!["once".to_string()],
            },
        )
        .await
        .map_err(|error| error.message)?;
        let access = self.slack_access(persistence).await?;
        send_slack_message(
            &self.http_client,
            &access.bot_access_token,
            channel_id,
            body,
        )
        .await
    }

    async fn run_remote_chat_turn(
        &self,
        persistence: PersistenceEngine,
        message: &GatewayIncomingMessage,
    ) -> Result<String, String> {
        let app = self.app_handle()?;
        let agent_config = self
            .agent_manager
            .get_most_recent_active_agent_config()
            .await?
            .ok_or_else(|| "Active gateway agent not found".to_string())?;
        let session_id = self
            .ensure_remote_chat_session(persistence.clone(), message, &agent_config)
            .await?;
        let response = inference::run_backend_chat_turn(
            ChatTurnRequest {
                turn_id: None,
                generation_token: None,
                parent_turn_id: None,
                root_turn_id: None,
                turn_kind: None,
                agent_id: agent_config.id,
                message: message.body.clone(),
                display_message: None,
                attachments: Vec::new(),
                session_id: Some(session_id),
                provider_id: None,
                model_id: None,
                locale: None,
                requested_mod_id: None,
                stream_id: None,
                reasoning: Some("medium".to_string()),
                context: None,
                context_budget: None,
                steering: None,
                steering_only: Some(false),
                persist_steering_message: None,
                verified_native_execution_receipt: None,
                native_execution_receipt_id: None,
                automated_web_grounding_enabled: Some(false),
                dynamic_routing_override: Some(false),
                queued_execution: false,
                queued_auto_route_identity: None,
                auto_route_choice: None,
                auto_route_cloud_confirmed: None,
                project_cloud_confirmed: None,
                project_document_composition: None,
            },
            app,
            self.agent_manager.clone(),
            persistence,
            self.knowledge_store.clone(),
            self.memory_ledger.clone(),
            self.identity.clone(),
            self.gemma.clone(),
            self.safe_mode,
        )
        .await
        .map_err(|error| {
            format!(
                "remote inference failed code={} boundary={} message={}",
                error.code, error.boundary, error.message
            )
        })?;
        Ok(response.text)
    }

    async fn ensure_remote_chat_session(
        &self,
        persistence: PersistenceEngine,
        message: &GatewayIncomingMessage,
        agent_config: &AgentConfig,
    ) -> Result<String, String> {
        let session_id = remote_chat_session_id(&message.platform, &message.sender_id);
        let title_sender = message
            .sender_display_name
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(message.sender_id.as_str());
        let request = CreateChatSessionRequest {
            agent_id: agent_config.id.clone(),
            provider_id: agent_config.provider_id.clone(),
            model_id: agent_config.model_id.clone(),
            title: Some(format!(
                "Remote {}: {title_sender}",
                channel_label(&message.platform)
            )),
            dynamic_routing_override: Some(false),
            workspace_id: None,
        };
        let persistence_for_session = persistence.clone();
        let session_id_for_insert = session_id.clone();
        tauri::async_runtime::spawn_blocking(move || {
            persistence_for_session.ensure_chat_session_with_id(&session_id_for_insert, request)
        })
        .await
        .map_err(|error| error.to_string())?
        .map(|session| session.id)
        .map_err(|error| error.to_string())
    }

    pub async fn deliver_routine_notice(
        &self,
        persistence: PersistenceEngine,
        platform: &str,
        destination: &str,
        body: &str,
    ) -> Result<String, String> {
        let body = body.trim();
        if body.is_empty() || body.len() > 2_000 {
            return Err("routine_notice_size_invalid".to_string());
        }
        let config = persistence
            .select_active_channel_configs()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|config| config.platform == platform && config.is_active)
            .ok_or_else(|| "routine_delivery_channel_inactive".to_string())?;
        let owner = config
            .owner_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "routine_delivery_owner_missing".to_string())?;
        if platform == "telegram" && destination.trim() != owner {
            return Err("routine_delivery_destination_not_authorized_owner".to_string());
        }
        match platform {
            "telegram" => {
                let credentials = load_telegram_credentials(persistence).await?;
                if credentials.owner_chat_id.as_deref() != Some(owner) {
                    return Err("routine_delivery_owner_mismatch".to_string());
                }
                send_telegram_reply_with_receipt(
                    &self.http_client,
                    &credentials.bot_token,
                    owner,
                    body,
                )
                .await
            }
            "discord" => {
                let credentials = load_discord_credentials(persistence).await?;
                if !credentials.allowlist_channels.contains(destination.trim()) {
                    return Err("routine_delivery_channel_not_allowlisted".to_string());
                }
                send_discord_reply_with_receipt(
                    &self.http_client,
                    &credentials.bot_token,
                    destination.trim(),
                    body,
                )
                .await
            }
            "slack" => {
                let settings = slack_config_from_channel(&config)?;
                if !settings.allowlist_channels.contains(destination.trim()) {
                    return Err("routine_delivery_channel_not_allowlisted".to_string());
                }
                let provider_message_id = self
                    .send_approved_slack_message(persistence.clone(), destination.trim(), body)
                    .await?;
                ordered_provider_receipt("slack", &[provider_message_id])
            }
            _ => Err("routine_delivery_platform_unsupported".to_string()),
        }
    }

    fn app_handle(&self) -> Result<tauri::AppHandle, String> {
        self.app_handle
            .lock()
            .map_err(|_| "Gateway app handle lock poisoned.".to_string())?
            .clone()
            .ok_or_else(|| "Gateway runtime is not attached to the Tauri app handle.".to_string())
    }

    fn update_channel_runtime_status(
        &self,
        platform: &str,
        worker_state: &str,
        connection_state: &str,
        detail: Option<&str>,
    ) {
        if let Ok(mut inner) = self.inner.lock() {
            let snapshot = inner.statuses.get_mut(platform).map(|status| {
                status.worker_state = worker_state.to_string();
                status.connection_state = connection_state.to_string();
                status.detail = detail.map(crate::redaction::redacted_log_text);
                status.last_checked_at_ms = Some(unix_time_ms());
                status.clone()
            });
            if let Some(status) = snapshot {
                emit_channel_status_event(&inner, status);
            }
        }
    }
}

fn spawn_telegram_worker(
    inner_ref: Arc<Mutex<GatewayServiceInner>>,
    incoming_sender: mpsc::Sender<GatewayIncomingMessage>,
    http_client: HttpClient,
    credentials: TelegramChannelCredentials,
    completion_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let _completion_guard = GatewayWorkerCompletionGuard(completion_flag);
        eprintln!(
            "SOVEREIGN_GATEWAY_WORKER_STARTING platform=telegram owner_configured={}",
            credentials.owner_chat_id.is_some()
        );
        let mut offset = 0_i64;
        let mut backoff = TELEGRAM_INITIAL_BACKOFF;
        if let Err(error) = probe_telegram_bot(&http_client, &credentials.bot_token).await {
            set_worker_status(
                &inner_ref,
                "telegram",
                "stopped",
                "error",
                Some("connection_check_failed"),
            );
            eprintln!(
                "SOVEREIGN_GATEWAY_TELEGRAM_PROBE_FAILED error={}",
                compact_log_text(&error, 160)
            );
            return;
        }
        set_worker_status(
            &inner_ref,
            "telegram",
            "running",
            active_connection_state("telegram"),
            Some("ready"),
        );
        loop {
            match poll_telegram_updates(&http_client, &credentials.bot_token, offset).await {
                Ok(updates) => {
                    set_worker_status(
                        &inner_ref,
                        "telegram",
                        "running",
                        active_connection_state("telegram"),
                        Some("ready"),
                    );
                    backoff = TELEGRAM_INITIAL_BACKOFF;
                    for update in updates {
                        let next_offset = update.update_id.saturating_add(1);
                        if next_offset > offset {
                            offset = next_offset;
                        }
                        if let Some(message) = update.message {
                            if let Some(incoming) = telegram_message_to_gateway_message(message) {
                                eprintln!(
                                    "{} update_id={} offset={}",
                                    gateway_message_log_fields("telegram", &incoming),
                                    update.update_id,
                                    offset
                                );
                                if incoming_sender.send(incoming).await.is_err() {
                                    set_worker_status(
                                        &inner_ref,
                                        "telegram",
                                        "stopped",
                                        "error",
                                        Some("ingress_queue_closed"),
                                    );
                                    eprintln!(
                                        "SOVEREIGN_GATEWAY_WORKER_STOPPING platform=telegram reason=ingress_queue_closed"
                                    );
                                    return;
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    let detail = format!("poll_retry_{}s", backoff.as_secs());
                    set_worker_status(&inner_ref, "telegram", "running", "error", Some(&detail));
                    eprintln!(
                        "SOVEREIGN_GATEWAY_TELEGRAM_POLL_FAILED retry_seconds={} error={}",
                        backoff.as_secs(),
                        compact_log_text(&error, 160)
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(TELEGRAM_MAX_BACKOFF);
                }
            }
        }
    })
}

fn spawn_discord_worker(
    inner_ref: Arc<Mutex<GatewayServiceInner>>,
    incoming_sender: mpsc::Sender<GatewayIncomingMessage>,
    credentials: DiscordChannelCredentials,
    completion_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let _completion_guard = GatewayWorkerCompletionGuard(completion_flag);
        eprintln!(
            "SOVEREIGN_GATEWAY_WORKER_STARTING platform=discord owner_configured={} allowlist_channels={}",
            credentials.owner_id.is_some(),
            credentials.allowlist_channels.len()
        );
        let mut session_id = None;
        let mut last_sequence = None;
        let mut backoff = DISCORD_RECONNECT_INITIAL_BACKOFF;
        loop {
            set_worker_status(
                &inner_ref,
                "discord",
                "connecting",
                inactive_connection_state("discord"),
                Some("connecting"),
            );
            let result = run_discord_gateway_session(
                Arc::clone(&inner_ref),
                incoming_sender.clone(),
                credentials.clone(),
                &mut session_id,
                &mut last_sequence,
            )
            .await;
            if let Err(error) = result {
                set_worker_status(
                    &inner_ref,
                    "discord",
                    "running",
                    "error",
                    Some("reconnecting"),
                );
                eprintln!(
                    "SOVEREIGN_GATEWAY_DISCORD_CONNECTION_DROPPED retry_seconds={} error={}",
                    backoff.as_secs(),
                    compact_log_text(&error, 180)
                );
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff + DISCORD_RECONNECT_BACKOFF_STEP).min(DISCORD_RECONNECT_MAX_BACKOFF);
        }
    })
}

async fn run_discord_gateway_session(
    inner_ref: Arc<Mutex<GatewayServiceInner>>,
    incoming_sender: mpsc::Sender<GatewayIncomingMessage>,
    credentials: DiscordChannelCredentials,
    session_id: &mut Option<String>,
    last_sequence: &mut Option<i64>,
) -> Result<(), String> {
    let (stream, _) = connect_async(DISCORD_GATEWAY_URL)
        .await
        .map_err(|error| format!("Discord Gateway WebSocket connect failed: {error}"))?;
    set_worker_status(
        &inner_ref,
        "discord",
        "connecting",
        inactive_connection_state("discord"),
        Some("websocket_connected"),
    );
    let (mut writer, mut reader) = stream.split();
    let (gateway_sender, mut gateway_receiver) = mpsc::channel::<Value>(32);
    let writer_handle = tauri::async_runtime::spawn(async move {
        while let Some(payload) = gateway_receiver.recv().await {
            if let Err(error) = writer
                .send(WebSocketMessage::Text(payload.to_string().into()))
                .await
            {
                eprintln!(
                    "SOVEREIGN_GATEWAY_DISCORD_WRITE_FAILED error={}",
                    compact_log_text(&error.to_string(), 160)
                );
                return;
            }
        }
    });

    let sequence_register = Arc::new(tokio::sync::Mutex::new(*last_sequence));
    let mut heartbeat_handle = None;
    let outcome = loop {
        let Some(next_message) = reader.next().await else {
            break Err("discord_gateway_stream_closed".to_string());
        };
        let message =
            next_message.map_err(|error| format!("Discord Gateway read failed: {error}"))?;
        match message {
            WebSocketMessage::Text(text) => {
                let payload = serde_json::from_str::<Value>(text.as_ref())
                    .map_err(|error| format!("Discord Gateway payload was not JSON: {error}"))?;
                if let Some(sequence) = payload.get("s").and_then(Value::as_i64) {
                    *last_sequence = Some(sequence);
                    *sequence_register.lock().await = Some(sequence);
                }
                let opcode = payload.get("op").and_then(Value::as_i64).unwrap_or(-1);
                match opcode {
                    0 => {
                        let event_type = payload.get("t").and_then(Value::as_str).unwrap_or("");
                        match event_type {
                            "READY" => {
                                if let Some(id) =
                                    payload.pointer("/d/session_id").and_then(Value::as_str)
                                {
                                    *session_id = Some(id.to_string());
                                }
                                set_worker_status(
                                    &inner_ref,
                                    "discord",
                                    "running",
                                    active_connection_state("discord"),
                                    Some("ready"),
                                );
                                eprintln!("DISCORD_GATEWAY_HANDSHAKE_COMPLETED");
                            }
                            "RESUMED" => {
                                set_worker_status(
                                    &inner_ref,
                                    "discord",
                                    "running",
                                    active_connection_state("discord"),
                                    Some("resumed"),
                                );
                                eprintln!("DISCORD_GATEWAY_HANDSHAKE_COMPLETED");
                            }
                            "MESSAGE_CREATE" => {
                                if let Some(incoming) =
                                    discord_message_to_gateway_message(&payload, &credentials)
                                {
                                    eprintln!(
                                        "{} channel_id_hash={}",
                                        gateway_message_log_fields("discord", &incoming),
                                        keyed_gateway_correlation_hash(
                                            "discord-channel-id",
                                            incoming.channel_id.as_deref().unwrap_or("none")
                                        )
                                    );
                                    if incoming_sender.send(incoming).await.is_err() {
                                        break Err("discord_ingress_queue_closed".to_string());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    1 => {
                        send_discord_heartbeat(&gateway_sender, *last_sequence).await?;
                    }
                    7 => {
                        break Err("discord_gateway_requested_reconnect".to_string());
                    }
                    9 => {
                        if payload.get("d").and_then(Value::as_bool) != Some(true) {
                            *session_id = None;
                            *last_sequence = None;
                            *sequence_register.lock().await = None;
                        }
                        break Err("discord_gateway_invalid_session".to_string());
                    }
                    10 => {
                        let heartbeat_interval_ms = payload
                            .pointer("/d/heartbeat_interval")
                            .and_then(Value::as_u64)
                            .ok_or_else(|| {
                                "discord_hello_missing_heartbeat_interval".to_string()
                            })?;
                        let heartbeat_sender = gateway_sender.clone();
                        let heartbeat_sequence = Arc::clone(&sequence_register);
                        heartbeat_handle = Some(tauri::async_runtime::spawn(async move {
                            let interval = Duration::from_millis(heartbeat_interval_ms.max(1_000));
                            loop {
                                tokio::time::sleep(interval).await;
                                let sequence = *heartbeat_sequence.lock().await;
                                if send_discord_heartbeat(&heartbeat_sender, sequence)
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                                eprintln!("DISCORD_GATEWAY_HEARTBEAT_SENT");
                            }
                        }));
                        if let (Some(existing_session_id), Some(sequence)) =
                            (session_id.as_deref(), *last_sequence)
                        {
                            gateway_sender
                                .send(json!({
                                    "op": 6,
                                    "d": {
                                        "token": credentials.bot_token.as_str(),
                                        "session_id": existing_session_id,
                                        "seq": sequence,
                                    },
                                }))
                                .await
                                .map_err(|error| {
                                    format!("discord_resume_queue_unavailable: {error}")
                                })?;
                        } else {
                            gateway_sender
                                .send(discord_identify_payload(&credentials.bot_token))
                                .await
                                .map_err(|error| {
                                    format!("discord_identify_queue_unavailable: {error}")
                                })?;
                        }
                    }
                    11 => {}
                    _ => {}
                }
            }
            WebSocketMessage::Close(frame) => {
                break Err(format!(
                    "discord_gateway_closed close_frame_present={}",
                    frame.is_some()
                ));
            }
            WebSocketMessage::Ping(_) => {}
            WebSocketMessage::Pong(_)
            | WebSocketMessage::Binary(_)
            | WebSocketMessage::Frame(_) => {}
        }
    };
    if let Some(handle) = heartbeat_handle {
        handle.abort();
    }
    writer_handle.abort();
    outcome
}

async fn send_discord_heartbeat(
    gateway_sender: &mpsc::Sender<Value>,
    sequence: Option<i64>,
) -> Result<(), String> {
    gateway_sender
        .send(json!({ "op": 1, "d": sequence }))
        .await
        .map_err(|error| format!("discord_heartbeat_queue_unavailable: {error}"))
}

fn discord_identify_payload(bot_token: &str) -> Value {
    json!({
        "op": 2,
        "d": {
            "token": bot_token,
            "intents": DISCORD_GATEWAY_INTENTS,
            "properties": {
                "os": env::consts::OS,
                "browser": "oomu",
                "device": "oomu",
            },
        },
    })
}

fn discord_message_to_gateway_message(
    payload: &Value,
    credentials: &DiscordChannelCredentials,
) -> Option<GatewayIncomingMessage> {
    let data = payload.get("d")?;
    if data.pointer("/author/bot").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let author_id = data.pointer("/author/id")?.as_str()?.trim();
    let channel_id = data.get("channel_id")?.as_str()?.trim();
    let body = data.get("content")?.as_str()?.trim().to_string();
    if author_id.is_empty() || channel_id.is_empty() || body.is_empty() {
        return None;
    }
    let Some(owner_id) = credentials.owner_id.as_deref() else {
        eprintln!("SOVEREIGN_GATEWAY_MESSAGE_DROPPED platform=discord reason=owner_unset");
        return None;
    };
    if owner_id.trim() != author_id {
        eprintln!("SOVEREIGN_GATEWAY_MESSAGE_DROPPED platform=discord reason=unauthorized_sender");
        return None;
    }
    if credentials.allowlist_channels.is_empty()
        || !credentials.allowlist_channels.contains(channel_id)
    {
        eprintln!(
            "SOVEREIGN_GATEWAY_MESSAGE_DROPPED platform=discord reason=channel_not_allowlisted"
        );
        return None;
    }
    let sender_display_name = data
        .pointer("/author/global_name")
        .and_then(Value::as_str)
        .or_else(|| data.pointer("/author/username").and_then(Value::as_str))
        .and_then(|value| clean_optional_text(Some(value)));
    Some(GatewayIncomingMessage {
        platform: "discord".to_string(),
        sender_id: author_id.to_string(),
        sender_display_name,
        channel_id: Some(channel_id.to_string()),
        body,
        message_id: data.get("id").and_then(Value::as_str).map(str::to_string),
        received_at_ms: unix_time_ms(),
        requested_actions: Vec::new(),
    })
}

async fn load_discord_credentials(
    persistence: PersistenceEngine,
) -> Result<DiscordChannelCredentials, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = persistence
            .select_channel_config("discord")
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Discord channel config is missing.".to_string())?;
        if !config.is_active {
            return Err("Discord channel is inactive.".to_string());
        }
        discord_credentials_from_config(&config)
    })
    .await
    .map_err(|error| error.to_string())?
}

async fn load_slack_settings(
    persistence: PersistenceEngine,
) -> Result<slack::SlackChannelConfig, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = persistence
            .select_channel_config("slack")
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Slack channel config is missing.".to_string())?;
        if !config.is_active {
            return Err("Slack channel is inactive.".to_string());
        }
        slack_config_from_channel(&config)
    })
    .await
    .map_err(|error| error.to_string())?
}

async fn poll_telegram_updates(
    client: &HttpClient,
    bot_token: &str,
    offset: i64,
) -> Result<Vec<TelegramUpdate>, String> {
    let endpoint = telegram_api_url(bot_token, "getUpdates");
    let timeout = TELEGRAM_LONG_POLL_TIMEOUT_SECONDS.to_string();
    let offset = offset.to_string();
    let params = [
        ("offset", offset.as_str()),
        ("timeout", timeout.as_str()),
        ("allowed_updates", "[\"message\"]"),
    ];
    let response = client
        .get(endpoint)
        .query(&params)
        .send()
        .await
        .map_err(|error| {
            format!(
                "Telegram getUpdates request failed: {}",
                crate::redaction::redact_network_error(&error.to_string())
            )
        })?;
    let status = response.status();
    let payload = response
        .json::<TelegramUpdatesResponse>()
        .await
        .map_err(|error| {
            format!(
                "Telegram getUpdates response was not valid JSON: {}",
                crate::redaction::redact_text(&error.to_string())
            )
        })?;
    if status.is_success() && payload.ok {
        Ok(payload.result)
    } else {
        Err(telegram_api_failure(
            "getUpdates",
            status.as_u16(),
            payload.error_code,
            payload.description.as_deref(),
        ))
    }
}

async fn load_telegram_credentials(
    persistence: PersistenceEngine,
) -> Result<TelegramChannelCredentials, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = persistence
            .select_channel_config("telegram")
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Telegram channel config is missing.".to_string())?;
        if !config.is_active {
            return Err("Telegram channel is inactive.".to_string());
        }
        telegram_credentials_from_config(&config)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn telegram_message_to_gateway_message(message: TelegramMessage) -> Option<GatewayIncomingMessage> {
    let body = message.text?.trim().to_string();
    if body.is_empty() {
        return None;
    }
    let sender_display_name = telegram_display_name(message.from.as_ref(), &message.chat);
    Some(GatewayIncomingMessage {
        platform: "telegram".to_string(),
        sender_id: message.chat.id.to_string(),
        sender_display_name,
        channel_id: None,
        body,
        message_id: Some(message.message_id.to_string()),
        received_at_ms: unix_time_ms(),
        requested_actions: Vec::new(),
    })
}

fn telegram_display_name(user: Option<&TelegramUser>, chat: &TelegramChat) -> Option<String> {
    let from_user = user.and_then(|user| {
        let name = [user.first_name.as_deref(), user.last_name.as_deref()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !name.is_empty() {
            Some(name)
        } else {
            user.username
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("@{value}"))
        }
    });
    from_user.or_else(|| {
        chat.title
            .as_deref()
            .or(chat.username.as_deref())
            .or(chat.first_name.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn telegram_credentials_from_config(
    config: &ChannelConfigRecord,
) -> Result<TelegramChannelCredentials, String> {
    let parsed = serde_json::from_str::<TelegramCredentialJson>(&config.credentials_json)
        .map_err(|error| format!("telegram_credentials_json_invalid: {error}"))?;
    let bot_token = parsed.bot_token.trim().to_string();
    if bot_token.is_empty() {
        return Err("telegram_bot_token_missing".to_string());
    }
    let owner_chat_id = clean_optional_text(config.owner_id.as_deref())
        .or_else(|| clean_optional_text(parsed.owner_chat_id.as_deref()));
    Ok(TelegramChannelCredentials {
        bot_token,
        owner_chat_id,
    })
}

fn telegram_config_owner_id(config: &ChannelConfigRecord) -> Option<String> {
    clean_optional_text(config.owner_id.as_deref()).or_else(|| {
        serde_json::from_str::<TelegramCredentialJson>(&config.credentials_json)
            .ok()
            .and_then(|credentials| clean_optional_text(credentials.owner_chat_id.as_deref()))
    })
}

fn chunk_gateway_reply(text: &str, max_bytes: usize) -> Vec<String> {
    let trimmed = text.trim();
    let text = if trimmed.is_empty() {
        "The remote agent completed the turn without returning text."
    } else {
        trimmed
    };
    let mut chunks = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if !current.is_empty() && current.len() + character.len_utf8() > max_bytes {
            chunks.push(current);
            current = String::new();
        }
        current.push(character);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn discord_credentials_from_config(
    config: &ChannelConfigRecord,
) -> Result<DiscordChannelCredentials, String> {
    let parsed = serde_json::from_str::<Value>(&config.credentials_json)
        .map_err(|error| format!("discord_credentials_json_invalid: {error}"))?;
    let bot_token = credential_string(
        &parsed,
        &["botToken", "bot_token", "apiKey", "api_key", "token"],
    )
    .ok_or_else(|| "discord_bot_token_missing".to_string())?;
    let owner_id = clean_optional_text(config.owner_id.as_deref()).or_else(|| {
        credential_string(
            &parsed,
            &["ownerId", "owner_id", "ownerUserId", "owner_user_id"],
        )
    });
    let allowlist_channels = credential_string_list(
        &parsed,
        &[
            "allowlistChannels",
            "allowlist_channels",
            "channelIds",
            "channel_ids",
        ],
    );
    Ok(DiscordChannelCredentials {
        bot_token,
        owner_id,
        allowlist_channels,
    })
}

fn discord_config_owner_id(config: &ChannelConfigRecord) -> Option<String> {
    clean_optional_text(config.owner_id.as_deref()).or_else(|| {
        serde_json::from_str::<Value>(&config.credentials_json)
            .ok()
            .and_then(|value| {
                credential_string(
                    &value,
                    &["ownerId", "owner_id", "ownerUserId", "owner_user_id"],
                )
            })
    })
}

fn credential_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .and_then(|value| clean_optional_text(Some(value)))
}

fn credential_string_list(value: &Value, keys: &[&str]) -> HashSet<String> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .map(|entry| match entry {
            Value::Array(values) => values
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|value| clean_optional_text(Some(value)))
                .collect(),
            Value::String(value) => value
                .split([',', '\n'])
                .filter_map(|entry| clean_optional_text(Some(entry)))
                .collect(),
            _ => HashSet::new(),
        })
        .unwrap_or_default()
}

fn remote_chat_session_id(platform: &str, sender_id: &str) -> String {
    let sender = sender_id
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let sender = sender.trim_matches('-');
    if sender.is_empty() {
        format!("remote-{}-unknown", normalize_platform_fragment(platform))
    } else {
        format!("remote-{}-{sender}", normalize_platform_fragment(platform))
    }
}

fn normalize_platform_fragment(platform: &str) -> String {
    platform
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn stop_gateway_worker(worker: GatewayWorker) {
    tauri::async_runtime::spawn(await_gateway_worker_shutdown(worker));
}

async fn await_gateway_worker_shutdown(worker: GatewayWorker) {
    worker.handle.abort();
    let mut handle = worker.handle;
    if tokio::time::timeout(GATEWAY_WORKER_SHUTDOWN_TIMEOUT, &mut handle)
        .await
        .is_err()
    {
        handle.abort();
        let _ = handle.await;
    }
}

fn random_secret_hex() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn apply_restart_suppressed_status(status: &mut GatewayChannelStatus, reason: &str) {
    status.worker_state = "stopped".to_string();
    status.connection_state = if status.is_active {
        "error"
    } else {
        inactive_connection_state(&status.platform)
    }
    .to_string();
    status.detail = Some(reason.to_string());
    status.last_checked_at_ms = Some(unix_time_ms());
}

fn set_worker_status(
    inner_ref: &Arc<Mutex<GatewayServiceInner>>,
    platform: &str,
    worker_state: &str,
    connection_state: &str,
    detail: Option<&str>,
) {
    if let Ok(mut inner) = inner_ref.lock() {
        let snapshot = inner.statuses.get_mut(platform).map(|status| {
            status.worker_state = worker_state.to_string();
            status.connection_state = connection_state.to_string();
            status.detail = detail.map(crate::redaction::redacted_log_text);
            status.last_checked_at_ms = Some(unix_time_ms());
            status.clone()
        });
        if let Some(status) = snapshot {
            emit_channel_status_event(&inner, status);
        }
    }
}

fn emit_channel_status_event(inner: &GatewayServiceInner, status: GatewayChannelStatus) {
    let Some(app) = inner.app_handle.as_ref() else {
        return;
    };
    let detail = status.detail.as_deref().unwrap_or("none");
    let trace = format!(
        "platform={} worker={} connection={} detail={detail}",
        status.platform, status.worker_state, status.connection_state
    );
    let _ = app.emit(
        "oomu://channel-status-log",
        GatewayChannelStatusLogEvent {
            timestamp_ms: unix_time_ms(),
            status,
            trace,
        },
    );
}

fn telegram_api_url(bot_token: &str, method: &str) -> String {
    format!(
        "https://api.telegram.org/bot{}/{}",
        bot_token.trim(),
        method
    )
}

fn telegram_api_failure(
    method: &str,
    status: u16,
    error_code: Option<i64>,
    description: Option<&str>,
) -> String {
    format!(
        "Telegram {method} failed status={} error_code={} description={}",
        status,
        error_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "none".to_string()),
        crate::redaction::redacted_log_text(description.unwrap_or("none"))
    )
}

fn compact_log_text(value: &str, max_chars: usize) -> String {
    let redacted = crate::redaction::redacted_log_text(value);
    let compact = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut truncated = compact.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn gateway_message_log_fields(platform: &str, message: &GatewayIncomingMessage) -> String {
    let correlation_hash = keyed_gateway_correlation_hash(platform, &message.body);
    format!(
        "SOVEREIGN_GATEWAY_MESSAGE_RECEIVED platform={} event_id_hash={} body_bytes={} correlation_hash={}",
        platform,
        keyed_gateway_correlation_hash(
            "message-id",
            message.message_id.as_deref().unwrap_or("none")
        ),
        message.body.len(),
        correlation_hash
    )
}

fn keyed_gateway_correlation_hash(domain: &str, value: &str) -> String {
    let key = GATEWAY_LOG_CORRELATION_KEY.get_or_init(|| {
        let mut key = [0_u8; 32];
        OsRng.fill_bytes(&mut key);
        key
    });
    let mut input = Vec::with_capacity(key.len() + domain.len() + value.len() + 2);
    input.extend_from_slice(key);
    input.push(0);
    input.extend_from_slice(domain.as_bytes());
    input.push(0);
    input.extend_from_slice(value.as_bytes());
    let correlation_hash = sha256_hex(&input);
    correlation_hash[..24].to_string()
}

fn clean_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn default_status_map() -> HashMap<String, GatewayChannelStatus> {
    COMMUNITY_CHANNEL_PLATFORMS
        .iter()
        .map(|platform| {
            let status = GatewayChannelStatus {
                platform: (*platform).to_string(),
                label: channel_label(platform).to_string(),
                is_active: false,
                connection_state: inactive_connection_state(platform).to_string(),
                owner_id: None,
                allowed_channel_ids: Vec::new(),
                worker_state: "idle".to_string(),
                last_checked_at_ms: None,
                detail: None,
            };
            ((*platform).to_string(), status)
        })
        .collect()
}

fn status_from_config(config: &ChannelConfigRecord) -> GatewayChannelStatus {
    let slack_settings = (config.platform == "slack")
        .then(|| slack_config_from_channel(config).ok())
        .flatten();
    let effective_owner_id = match config.platform.as_str() {
        "telegram" => telegram_config_owner_id(config),
        "discord" => discord_config_owner_id(config),
        "slack" => slack_settings.as_ref().map(|value| value.owner_id.clone()),
        _ => clean_optional_text(config.owner_id.as_deref()),
    };
    let mut allowed_channel_ids = slack_settings
        .as_ref()
        .map(|value| value.allowlist_channels.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    allowed_channel_ids.sort();
    GatewayChannelStatus {
        platform: config.platform.clone(),
        label: config.label.clone(),
        is_active: config.is_active,
        connection_state: if config.is_active && effective_owner_id.is_some() {
            active_connection_state(&config.platform).to_string()
        } else {
            inactive_connection_state(&config.platform).to_string()
        },
        owner_id: effective_owner_id.clone(),
        allowed_channel_ids,
        worker_state: if config.is_active { "starting" } else { "idle" }.to_string(),
        last_checked_at_ms: Some(unix_time_ms()),
        detail: if config.is_active && effective_owner_id.is_none() {
            Some("owner_missing".to_string())
        } else {
            None
        },
    }
}

#[derive(Debug, Clone)]
struct TelegramChannelCredentials {
    bot_token: String,
    owner_chat_id: Option<String>,
}

#[derive(Debug, Clone)]
struct DiscordChannelCredentials {
    bot_token: String,
    owner_id: Option<String>,
    allowlist_channels: HashSet<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TelegramCredentialJson {
    #[serde(default, alias = "botToken", alias = "bot_token", alias = "token")]
    bot_token: String,
    #[serde(
        default,
        alias = "owner_id",
        alias = "ownerChatId",
        alias = "chat_id",
        alias = "chatId"
    )]
    owner_chat_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdatesResponse {
    ok: bool,
    #[serde(default)]
    result: Vec<TelegramUpdate>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    error_code: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    message_id: i64,
    #[serde(default)]
    from: Option<TelegramUser>,
    chat: TelegramChat,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    last_name: Option<String>,
    #[serde(default)]
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

#[cfg(test)]
mod tests;
