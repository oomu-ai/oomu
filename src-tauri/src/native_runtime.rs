mod prefill;

use crate::{
    metal_backend,
    shield_gate::{
        CodebaseCompileRequest, CodebaseCompileTarget, CommandStatus, ExecuteCommandResponse,
    },
};
use llama_cpp_2::{
    context::{params::LlamaContextParams, LlamaContext},
    gguf::GgufContext,
    list_llama_ggml_backend_devices,
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{params::LlamaModelParams, AddBos, LlamaModel},
    mtmd::{mtmd_default_marker, MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputText},
    sampling::LlamaSampler,
    token::LlamaToken,
    LlamaBackendDeviceType,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    ffi::CString,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    num::NonZeroU32,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    process::{Command as StdCommand, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use sysinfo::{Pid, System};
use tauri::Emitter;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
    time,
};

const MIN_GGUF_BYTES: u64 = 32;
const DEFAULT_CONTEXT_SIZE: u32 = 12288;
const DEFAULT_BATCH_SIZE: u32 = 512;
const DEFAULT_UBATCH_SIZE: u32 = 256;
const DEFAULT_MAX_SESSIONS: u32 = 8;
const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 120;
const DEFAULT_PINNED_PREFIX_TOKENS: usize = 256;
const COMPLETE_WRITE_PROBE_MS: u64 = 25;
const CODEBASE_COMPILE_TIMEOUT_SECS: u64 = 300;
const CODEBASE_COMPILE_LOG_EVENT: &str = "codebase-compile-log";
const CODEBASE_COMPILE_REFRESH_EVENT: &str = "codebase-compile-refresh";
const AUTONOMIC_RECYCLE_MEMORY_THRESHOLD_BYTES: u64 = 500 * 1024 * 1024;
const AUTONOMIC_TERMINATE_WAIT_MS: u64 = 2_000;
const DISPLAYLINK_MANAGER_RESTART_LABEL: &str = "open -g -j -a DisplayLink Manager";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutonomicRestartStrategy {
    OpenMacApp { app_name: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AutonomicRecyclePolicy {
    canonical_name: &'static str,
    match_terms: &'static [&'static str],
    category: &'static str,
    restart: Option<AutonomicRestartStrategy>,
}

const AUTONOMIC_RECYCLE_POLICIES: &[AutonomicRecyclePolicy] = &[
    AutonomicRecyclePolicy {
        canonical_name: "CrashRestartHelper",
        match_terms: &["crashrestarthelper"],
        category: "display_utility",
        restart: Some(AutonomicRestartStrategy::OpenMacApp {
            app_name: "DisplayLink Manager",
        }),
    },
    AutonomicRecyclePolicy {
        canonical_name: "DisplayLink Manager",
        match_terms: &["displaylink manager"],
        category: "display_utility",
        restart: Some(AutonomicRestartStrategy::OpenMacApp {
            app_name: "DisplayLink Manager",
        }),
    },
    AutonomicRecyclePolicy {
        canonical_name: "DisplayLinkUserAgent",
        match_terms: &["displaylinkuseragent"],
        category: "display_utility",
        restart: Some(AutonomicRestartStrategy::OpenMacApp {
            app_name: "DisplayLink Manager",
        }),
    },
    AutonomicRecyclePolicy {
        canonical_name: "DisplayLinkLoginScreenExtension",
        match_terms: &["displaylinkloginscreenextension"],
        category: "display_utility",
        restart: Some(AutonomicRestartStrategy::OpenMacApp {
            app_name: "DisplayLink Manager",
        }),
    },
    AutonomicRecyclePolicy {
        canonical_name: "DisplayLinkUIAgent",
        match_terms: &["displaylinkuiagent"],
        category: "display_utility",
        restart: Some(AutonomicRestartStrategy::OpenMacApp {
            app_name: "DisplayLink Manager",
        }),
    },
    AutonomicRecyclePolicy {
        canonical_name: "Turbopack",
        match_terms: &["turbopack", "next-server"],
        category: "development_helper",
        restart: None,
    },
    AutonomicRecyclePolicy {
        canonical_name: "Vite",
        match_terms: &["vite"],
        category: "development_helper",
        restart: None,
    },
    AutonomicRecyclePolicy {
        canonical_name: "Webpack",
        match_terms: &["webpack"],
        category: "development_helper",
        restart: None,
    },
];

static NATIVE_RUNTIME: OnceLock<Result<Arc<NativeRuntime>, NativeRuntimeError>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HardwareProfile {
    pub operating_system: String,
    pub architecture: String,
    pub apple_silicon: bool,
    pub metal_available: bool,
    pub gpu_offload_available: bool,
    pub mmap_available: bool,
    pub mlock_available: bool,
    pub accelerator_name: Option<String>,
    pub accelerator_memory_bytes: u64,
    pub logical_threads: usize,
    pub total_memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuntimeConfig {
    pub context_size: u32,
    pub batch_size: u32,
    pub ubatch_size: u32,
    pub decode_threads: i32,
    pub batch_threads: i32,
    pub requested_gpu_layers: u32,
    pub use_mmap: bool,
    pub use_mlock: bool,
    pub max_sessions: u32,
    pub idle_timeout_secs: u64,
    pub pinned_prefix_tokens: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NativeModelProfile {
    pub path: PathBuf,
    pub architecture: String,
    pub name: String,
    pub tensor_count: usize,
    pub layer_count: u32,
    pub embedding_length: u32,
    pub per_layer_embedding_length: Option<u32>,
    pub multi_layer_embeddings: bool,
    pub parameter_count: u64,
    pub model_bytes: u64,
    pub chat_template_present: bool,
    pub filesystem: String,
    pub device_label: String,
    pub gpu_layers: u32,
    pub gpu_offload_ratio: f32,
    pub runtime_config: RuntimeConfig,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NativeRuntimeError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AutonomicRecyclePolicyInfo {
    pub canonical_name: &'static str,
    pub category: &'static str,
    pub restart_available: bool,
    pub restart_strategy: Option<&'static str>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutonomicRecycleRequest {
    pub pid: u32,
    #[serde(default)]
    pub process_name: String,
    pub expected_resident_memory_bytes: Option<u64>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutonomicRecycleResponse {
    pub status: String,
    pub pid: u32,
    pub process_name: String,
    pub category: String,
    pub resident_memory_bytes: u64,
    pub threshold_bytes: u64,
    pub terminated: bool,
    pub restart_attempted: bool,
    pub restart_status: String,
    pub detail: String,
}

pub struct NativeRuntime {
    backend: LlamaBackend,
    hardware: HardwareProfile,
    config: RuntimeConfig,
}

pub struct NativeModelHandle {
    command_tx: mpsc::Sender<ModelCommand>,
    join: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
pub struct NativeSessionRequest {
    pub session_id: String,
    pub system_prompt: Option<String>,
    pub prompt: String,
    pub prompt_is_full_context: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NativeSessionStats {
    pub session_id: String,
    pub cached_tokens: usize,
    pub evaluated_tokens: usize,
    pub context_tokens: usize,
    pub pinned_tokens: usize,
    pub shifted_tokens: usize,
    pub evicted_sessions: usize,
    pub cold_start: bool,
}

#[derive(Debug, Clone)]
pub struct NativeGenerationRequest {
    pub session: NativeSessionRequest,
    pub media: Vec<NativeMediaInput>,
    pub max_new_tokens: usize,
    pub temperature: f32,
    pub top_k: i32,
    pub top_p: f32,
    pub repeat_penalty: f32,
    pub grammar: Option<String>,
    pub cancellation: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct NativeMediaInput {
    pub name: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NativeTokenEvent {
    pub sequence: usize,
    pub token_id: i32,
    pub text: String,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NativeGenerationResult {
    pub text: String,
    /// Unsanitized concatenation of every generated token piece (special tokens visible).
    /// Used for diagnostics and as a salvage source when the sanitizer suppresses everything.
    pub raw_text: String,
    pub token_ids: Vec<i32>,
    pub time_to_first_token_ms: u128,
    pub cancelled: bool,
    pub session_stats: NativeSessionStats,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodebaseCompileLogEvent {
    pub target: String,
    pub phase: String,
    pub stream: String,
    pub line: String,
    pub elapsed_ms: u128,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodebaseCompileRefreshEvent {
    pub target: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CodebaseCompileCommand {
    phase: &'static str,
    program: &'static str,
    args: &'static [&'static str],
    display: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodebaseCompileCommandResult {
    phase: &'static str,
    display: &'static str,
    exit_code: Option<i32>,
    timed_out: bool,
    stdout: String,
    stderr: String,
}

enum ModelCommand {
    #[cfg(test)]
    AppendContext {
        request: NativeSessionRequest,
        response_tx: mpsc::SyncSender<Result<NativeSessionStats, NativeRuntimeError>>,
    },
    Generate {
        request: NativeGenerationRequest,
        event_tx: mpsc::Sender<NativeTokenEvent>,
        response_tx: mpsc::SyncSender<Result<NativeGenerationResult, NativeRuntimeError>>,
    },
    EmbedText {
        text: String,
        response_tx: mpsc::SyncSender<Result<Vec<f32>, NativeRuntimeError>>,
    },
    FlushMemory {
        response_tx: mpsc::SyncSender<()>,
    },
    Shutdown,
}

struct SessionCache {
    sequence_id: i32,
    tokens: Vec<LlamaToken>,
    source_tokens: Vec<LlamaToken>,
    pinned_tokens: usize,
    system_prompt: Option<String>,
    resident: bool,
    last_used: Instant,
}

struct CacheManager {
    sessions: HashMap<String, SessionCache>,
    max_sessions: usize,
}

impl NativeRuntime {
    pub fn initialize() -> Result<Arc<Self>, NativeRuntimeError> {
        NATIVE_RUNTIME
            .get_or_init(|| {
                let mut backend = catch_unwind(AssertUnwindSafe(LlamaBackend::init))
                    .map_err(|_| NativeRuntimeError {
                        code: "llama_runtime_init_panicked",
                        message:
                            "llama.cpp backend initialization panicked and was safely contained."
                                .to_string(),
                    })?
                    .map_err(|error| NativeRuntimeError {
                        code: "llama_runtime_init_failed",
                        message: format!("llama.cpp backend initialization failed: {error}"),
                    })?;
                if env::var("OOMU_LLAMA_VERBOSE").ok().as_deref() != Some("1") {
                    backend.void_logs();
                }
                let hardware = detect_hardware(&backend)?;
                let config = RuntimeConfig::for_hardware(&hardware);
                Ok(Arc::new(Self {
                    backend,
                    hardware,
                    config,
                }))
            })
            .clone()
    }

    pub fn hardware(&self) -> &HardwareProfile {
        &self.hardware
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn inspect_model(&self, path: &Path) -> Result<NativeModelProfile, NativeRuntimeError> {
        let ready = validate_gguf_readiness(path)?;
        let gguf = GgufContext::from_file(&ready.path).ok_or_else(|| NativeRuntimeError {
            code: "llama_gguf_parse_failed",
            message: format!(
                "llama.cpp rejected {} while parsing GGUF metadata.",
                ready.path.display()
            ),
        })?;
        let params = LlamaModelParams::default()
            .with_vocab_only(true)
            .with_n_gpu_layers(0)
            .with_use_mmap(false);
        let model = catch_unwind(AssertUnwindSafe(|| {
            LlamaModel::load_from_file(&self.backend, &ready.path, &params)
        }))
        .map_err(|_| NativeRuntimeError {
            code: "llama_model_validation_panicked",
            message: format!(
                "llama.cpp panicked while validating {} and the failure was contained.",
                ready.path.display()
            ),
        })?
        .map_err(|error| NativeRuntimeError {
            code: "llama_model_validation_failed",
            message: format!(
                "llama.cpp could not validate {}: {error}",
                ready.path.display()
            ),
        })?;
        validate_loaded_model(
            &model,
            ready,
            usize::try_from(gguf.n_tensors()).unwrap_or_default(),
            &self.hardware,
            &self.config,
        )
    }

    pub fn load_model(
        self: &Arc<Self>,
        path: &Path,
    ) -> Result<(NativeModelHandle, NativeModelProfile), NativeRuntimeError> {
        self.load_model_with_min_context_size(path, None)
    }

    pub fn load_model_with_min_context_size(
        self: &Arc<Self>,
        path: &Path,
        min_context_size: Option<u32>,
    ) -> Result<(NativeModelHandle, NativeModelProfile), NativeRuntimeError> {
        let ready = validate_gguf_readiness(path)?;
        let (command_tx, command_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let runtime = Arc::clone(self);
        let worker_config = self.config.with_min_context_size(min_context_size);
        let model_path = ready.path.clone();
        let join = thread::Builder::new()
            .name("oomu-llama-model".to_string())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    runtime.run_model_worker(ready, command_rx, &ready_tx, worker_config)
                }))
                .map_err(|_| NativeRuntimeError {
                    code: "llama_model_worker_panicked",
                    message: format!(
                        "llama.cpp panicked while loading {} and the failure was contained.",
                        model_path.display()
                    ),
                })
                .and_then(|result| result);

                if let Err(error) = result {
                    let _ = ready_tx.send(Err(error));
                }
            })
            .map_err(|error| NativeRuntimeError {
                code: "llama_model_worker_spawn_failed",
                message: format!("Unable to start the llama.cpp model worker: {error}"),
            })?;

        let handle = NativeModelHandle {
            command_tx,
            join: Some(join),
        };
        let profile = ready_rx.recv().map_err(|error| NativeRuntimeError {
            code: "llama_model_worker_disconnected",
            message: format!("llama.cpp model worker disconnected during startup: {error}"),
        })??;
        Ok((handle, profile))
    }

    fn run_model_worker(
        &self,
        ready: ReadyGguf,
        command_rx: mpsc::Receiver<ModelCommand>,
        ready_tx: &mpsc::SyncSender<Result<NativeModelProfile, NativeRuntimeError>>,
        config: RuntimeConfig,
    ) -> Result<(), NativeRuntimeError> {
        let mut cache = CacheManager::new(config.max_sessions);
        let multimodal_projector = discover_multimodal_projector(&ready.path);
        let mut initial_load = true;
        loop {
            let command = match initial_load {
                true => {
                    let (model, profile) = self.load_resident_model(&ready, &config)?;
                    let mut context = self.create_context(&model, &profile, &config)?;
                    ready_tx
                        .send(Ok(profile))
                        .map_err(|error| NativeRuntimeError {
                            code: "llama_model_worker_disconnected",
                            message: format!(
                                "Unable to publish llama.cpp model readiness: {error}"
                            ),
                        })?;
                    initial_load = false;
                    match self.run_resident_cycle(
                        &model,
                        &mut context,
                        &command_rx,
                        &mut cache,
                        None,
                        multimodal_projector.as_deref(),
                        &config,
                    )? {
                        ResidentCycleExit::Dormant => continue,
                        ResidentCycleExit::Shutdown => return Ok(()),
                    }
                }
                false => match command_rx.recv() {
                    Ok(ModelCommand::Shutdown) | Err(_) => return Ok(()),
                    Ok(ModelCommand::FlushMemory { response_tx }) => {
                        let _ = response_tx.send(());
                        continue;
                    }
                    Ok(command) => command,
                },
            };

            let (model, profile) = match self.load_resident_model(&ready, &config) {
                Ok(loaded) => loaded,
                Err(error) => {
                    reply_with_error(command, error);
                    continue;
                }
            };
            let mut context = match self.create_context(&model, &profile, &config) {
                Ok(context) => context,
                Err(error) => {
                    reply_with_error(command, error);
                    continue;
                }
            };
            cache.mark_all_dormant();
            match self.run_resident_cycle(
                &model,
                &mut context,
                &command_rx,
                &mut cache,
                Some(command),
                multimodal_projector.as_deref(),
                &config,
            )? {
                ResidentCycleExit::Dormant => {}
                ResidentCycleExit::Shutdown => return Ok(()),
            }
        }
    }

    fn load_resident_model(
        &self,
        ready: &ReadyGguf,
        config: &RuntimeConfig,
    ) -> Result<(LlamaModel, NativeModelProfile), NativeRuntimeError> {
        let params = LlamaModelParams::default()
            .with_n_gpu_layers(config.requested_gpu_layers)
            .with_use_mmap(config.use_mmap)
            .with_use_mlock(config.use_mlock);
        let model =
            LlamaModel::load_from_file(&self.backend, &ready.path, &params).map_err(|error| {
                NativeRuntimeError {
                    code: "llama_model_load_failed",
                    message: format!("llama.cpp could not load {}: {error}", ready.path.display()),
                }
            })?;
        let gguf = GgufContext::from_file(&ready.path).ok_or_else(|| NativeRuntimeError {
            code: "llama_gguf_parse_failed",
            message: format!(
                "llama.cpp rejected {} while reading final GGUF metadata.",
                ready.path.display()
            ),
        })?;
        let profile = validate_loaded_model(
            &model,
            ReadyGguf {
                path: ready.path.clone(),
                byte_count: ready.byte_count,
                filesystem: ready.filesystem.clone(),
            },
            usize::try_from(gguf.n_tensors()).unwrap_or_default(),
            &self.hardware,
            config,
        )?;
        Ok((model, profile))
    }

    fn create_context<'model>(
        &self,
        model: &'model LlamaModel,
        profile: &NativeModelProfile,
        config: &RuntimeConfig,
    ) -> Result<LlamaContext<'model>, NativeRuntimeError> {
        let context_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(config.context_size))
            .with_n_batch(config.batch_size)
            .with_n_ubatch(config.ubatch_size)
            .with_n_seq_max(1)
            .with_n_threads(config.decode_threads)
            .with_n_threads_batch(config.batch_threads);
        model
            .new_context(&self.backend, context_params)
            .map_err(|error| NativeRuntimeError {
                code: "llama_context_init_failed",
                message: format!(
                    "llama.cpp loaded {} but could not allocate its stateful context: {error}",
                    profile.path.display()
                ),
            })
    }

    fn embed_text_with_model(
        &self,
        model: &LlamaModel,
        text: &str,
        config: &RuntimeConfig,
    ) -> Result<Vec<f32>, NativeRuntimeError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(NativeRuntimeError {
                code: "llama_embedding_input_empty",
                message: "llama.cpp embedding requires non-empty text.".to_string(),
            });
        }
        let tokens = tokenize(model, text, AddBos::Always)?;
        if tokens.is_empty() {
            return Err(NativeRuntimeError {
                code: "llama_embedding_tokenization_empty",
                message: "llama.cpp produced no tokens for the embedding input.".to_string(),
            });
        }
        if tokens.len() > config.context_size as usize {
            return Err(NativeRuntimeError {
                code: "llama_embedding_context_exceeded",
                message: format!(
                    "Embedding input requires {} tokens but the resident context limit is {}.",
                    tokens.len(),
                    config.context_size
                ),
            });
        }

        let context_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(config.context_size))
            .with_n_batch(config.batch_size)
            .with_n_ubatch(config.ubatch_size)
            .with_n_seq_max(1)
            .with_n_threads(config.decode_threads)
            .with_n_threads_batch(config.batch_threads)
            .with_embeddings(true);
        let mut context = model
            .new_context(&self.backend, context_params)
            .map_err(|error| NativeRuntimeError {
                code: "llama_embedding_context_init_failed",
                message: format!("llama.cpp could not allocate an embedding context: {error}"),
            })?;

        let mut aggregate: Option<Vec<f32>> = None;
        let mut embedded_tokens = 0usize;
        let batch_size = config.batch_size.max(1) as usize;
        for (chunk_index, chunk) in tokens.chunks(batch_size).enumerate() {
            let mut batch = LlamaBatch::new(chunk.len(), 1);
            let start_position = chunk_index * batch_size;
            for (offset, token) in chunk.iter().enumerate() {
                let position =
                    i32::try_from(start_position + offset).map_err(|_| NativeRuntimeError {
                        code: "llama_embedding_position_overflow",
                        message: "Embedding token position exceeded llama.cpp limits.".to_string(),
                    })?;
                batch
                    .add(*token, position, &[0], true)
                    .map_err(|error| NativeRuntimeError {
                        code: "llama_embedding_batch_failed",
                        message: format!("Unable to build the llama.cpp embedding batch: {error}"),
                    })?;
            }
            context
                .decode(&mut batch)
                .map_err(|error| NativeRuntimeError {
                    code: "llama_embedding_decode_failed",
                    message: format!("llama.cpp embedding decode failed: {error}"),
                })?;
            for index in 0..chunk.len() {
                let embedding =
                    context
                        .embeddings_ith(index as i32)
                        .map_err(|error| NativeRuntimeError {
                            code: "llama_embedding_unavailable",
                            message: format!("llama.cpp did not expose token embeddings: {error}"),
                        })?;
                let vector = aggregate.get_or_insert_with(|| vec![0.0; embedding.len()]);
                if vector.len() != embedding.len() {
                    return Err(NativeRuntimeError {
                        code: "llama_embedding_dimension_changed",
                        message: "llama.cpp returned inconsistent embedding dimensions."
                            .to_string(),
                    });
                }
                for (target, value) in vector.iter_mut().zip(embedding.iter()) {
                    *target += *value;
                }
                embedded_tokens += 1;
            }
        }

        let mut vector = aggregate.ok_or_else(|| NativeRuntimeError {
            code: "llama_embedding_output_empty",
            message: "llama.cpp returned no embedding tensor.".to_string(),
        })?;
        let divisor = embedded_tokens as f32;
        for value in &mut vector {
            *value /= divisor;
        }
        let magnitude = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        if !magnitude.is_finite() || magnitude <= f32::EPSILON {
            return Err(NativeRuntimeError {
                code: "llama_embedding_output_invalid",
                message: "llama.cpp returned a zero or non-finite embedding tensor.".to_string(),
            });
        }
        for value in &mut vector {
            *value /= magnitude;
        }
        Ok(vector)
    }

    fn run_resident_cycle(
        &self,
        model: &LlamaModel,
        context: &mut LlamaContext<'_>,
        command_rx: &mpsc::Receiver<ModelCommand>,
        cache: &mut CacheManager,
        first_command: Option<ModelCommand>,
        multimodal_projector: Option<&Path>,
        config: &RuntimeConfig,
    ) -> Result<ResidentCycleExit, NativeRuntimeError> {
        let idle_timeout = Duration::from_secs(config.idle_timeout_secs.max(1));
        let mut first_command = first_command;
        let mut multimodal_context: Option<MtmdContext> = None;

        loop {
            let command = match first_command.take() {
                Some(command) => command,
                None => match command_rx.recv_timeout(idle_timeout) {
                    Ok(command) => command,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        context.clear_kv_cache();
                        cache.mark_all_dormant();
                        return Ok(ResidentCycleExit::Dormant);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Ok(ResidentCycleExit::Shutdown);
                    }
                },
            };

            match command {
                #[cfg(test)]
                ModelCommand::AppendContext {
                    request,
                    response_tx,
                } => {
                    let result = cache.append(
                        model,
                        context,
                        request,
                        config.context_size as usize,
                        config.batch_size as usize,
                        config.pinned_prefix_tokens,
                        None,
                        |_, _| {},
                    );
                    let _ = response_tx.send(result);
                }
                ModelCommand::Generate {
                    request,
                    event_tx,
                    response_tx,
                } => {
                    let result = if request.media.is_empty() {
                        cache.generate(
                            model,
                            context,
                            request,
                            event_tx,
                            config.context_size as usize,
                            config.batch_size as usize,
                            config.pinned_prefix_tokens,
                        )
                    } else {
                        let multimodal_context_result = if let Some(existing) =
                            multimodal_context.as_ref()
                        {
                            Ok(existing)
                        } else {
                            let projector = multimodal_projector.ok_or_else(|| {
                                NativeRuntimeError {
                                    code: "local_model_multimodal_projector_missing",
                                    message: "This local vision model is missing its matching image projector. Reinstall the selected model to use images.".to_string(),
                                }
                            });
                            match projector.and_then(|projector| {
                                create_multimodal_context(model, projector, config)
                            }) {
                                Ok(created) => {
                                    multimodal_context = Some(created);
                                    Ok(multimodal_context
                                        .as_ref()
                                        .expect("multimodal context was just initialized"))
                                }
                                Err(error) => Err(error),
                            }
                        };
                        match multimodal_context_result {
                            Ok(multimodal) => cache.generate_multimodal(
                                model,
                                context,
                                multimodal,
                                request,
                                event_tx,
                                config.context_size as usize,
                                config.batch_size as usize,
                            ),
                            Err(error) => Err(error),
                        }
                    };
                    let _ = response_tx.send(result);
                }
                ModelCommand::EmbedText { text, response_tx } => {
                    let result = self.embed_text_with_model(model, &text, config);
                    let _ = response_tx.send(result);
                }
                ModelCommand::FlushMemory { response_tx } => {
                    context.clear_kv_cache();
                    cache.mark_all_dormant();
                    let _ = response_tx.send(());
                    return Ok(ResidentCycleExit::Dormant);
                }
                ModelCommand::Shutdown => return Ok(ResidentCycleExit::Shutdown),
            }
        }
    }
}

impl RuntimeConfig {
    fn for_hardware(hardware: &HardwareProfile) -> Self {
        let logical_threads = hardware.logical_threads;
        let decode_threads = env_i32("OOMU_LLAMA_THREADS")
            .unwrap_or_else(|| i32::try_from((logical_threads / 2).max(1)).unwrap_or(i32::MAX));
        let batch_threads = env_i32("OOMU_LLAMA_BATCH_THREADS")
            .unwrap_or_else(|| i32::try_from(logical_threads).unwrap_or(i32::MAX));
        let metal_enabled = hardware.apple_silicon
            && hardware.metal_available
            && env::var("OOMU_DISABLE_METAL").ok().as_deref() != Some("1");
        let requested_gpu_layers =
            env_u32("OOMU_LLAMA_GPU_LAYERS").unwrap_or(if metal_enabled { u32::MAX } else { 0 });

        Self {
            context_size: env_u32("OOMU_LLAMA_CONTEXT_SIZE")
                .unwrap_or(DEFAULT_CONTEXT_SIZE)
                .clamp(512, 131_072),
            batch_size: env_u32("OOMU_LLAMA_BATCH_SIZE")
                .unwrap_or(DEFAULT_BATCH_SIZE)
                .clamp(32, 8_192),
            ubatch_size: env_u32("OOMU_LLAMA_UBATCH_SIZE")
                .unwrap_or(DEFAULT_UBATCH_SIZE)
                .clamp(32, 2_048),
            decode_threads: decode_threads.max(1),
            batch_threads: batch_threads.max(1),
            requested_gpu_layers,
            use_mmap: hardware.mmap_available,
            use_mlock: hardware.mlock_available
                && env::var("OOMU_LLAMA_MLOCK").ok().as_deref() == Some("1"),
            max_sessions: env_u32("OOMU_LLAMA_MAX_SESSIONS")
                .unwrap_or(DEFAULT_MAX_SESSIONS)
                .clamp(1, 64),
            idle_timeout_secs: env_u64("OOMU_LLAMA_IDLE_TIMEOUT_SECS")
                .unwrap_or(DEFAULT_IDLE_TIMEOUT_SECS)
                .clamp(1, 86_400),
            pinned_prefix_tokens: env_usize("OOMU_LLAMA_PINNED_PREFIX_TOKENS")
                .unwrap_or(DEFAULT_PINNED_PREFIX_TOKENS)
                .clamp(1, 8_192),
        }
    }

    fn with_min_context_size(&self, min_context_size: Option<u32>) -> Self {
        let mut config = self.clone();
        if let Some(min_context_size) = min_context_size {
            config.context_size = config
                .context_size
                .max(min_context_size.clamp(512, 131_072));
        }
        config
    }
}

impl NativeModelHandle {
    #[cfg(test)]
    pub fn append_context(
        &self,
        request: NativeSessionRequest,
    ) -> Result<NativeSessionStats, NativeRuntimeError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(ModelCommand::AppendContext {
                request,
                response_tx,
            })
            .map_err(|error| NativeRuntimeError {
                code: "llama_model_worker_disconnected",
                message: format!("Unable to submit a stateful llama.cpp request: {error}"),
            })?;
        response_rx.recv().map_err(|error| NativeRuntimeError {
            code: "llama_model_worker_disconnected",
            message: format!("Stateful llama.cpp request did not complete: {error}"),
        })?
    }

    pub fn flush_memory(&self) -> Result<(), NativeRuntimeError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(ModelCommand::FlushMemory { response_tx })
            .map_err(|error| NativeRuntimeError {
                code: "llama_model_worker_disconnected",
                message: format!("Unable to request llama.cpp memory release: {error}"),
            })?;
        response_rx.recv().map_err(|error| NativeRuntimeError {
            code: "llama_model_worker_disconnected",
            message: format!("llama.cpp memory release did not complete: {error}"),
        })
    }

    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>, NativeRuntimeError> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(ModelCommand::EmbedText {
                text: text.to_string(),
                response_tx,
            })
            .map_err(|error| NativeRuntimeError {
                code: "llama_model_worker_disconnected",
                message: format!("Unable to submit llama.cpp embedding request: {error}"),
            })?;
        response_rx.recv().map_err(|error| NativeRuntimeError {
            code: "llama_model_worker_disconnected",
            message: format!("llama.cpp embedding request did not complete: {error}"),
        })?
    }

    pub fn generate(
        &self,
        request: NativeGenerationRequest,
        mut on_token: impl FnMut(NativeTokenEvent),
    ) -> Result<NativeGenerationResult, NativeRuntimeError> {
        let (event_tx, event_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.command_tx
            .send(ModelCommand::Generate {
                request,
                event_tx,
                response_tx,
            })
            .map_err(|error| NativeRuntimeError {
                code: "llama_model_worker_disconnected",
                message: format!("Unable to submit llama.cpp generation: {error}"),
            })?;

        loop {
            match event_rx.recv_timeout(Duration::from_millis(10)) {
                Ok(event) => on_token(event),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return response_rx.recv().map_err(|error| NativeRuntimeError {
                        code: "llama_model_worker_disconnected",
                        message: format!("llama.cpp generation did not complete: {error}"),
                    })?;
                }
            }
            if let Ok(result) = response_rx.try_recv() {
                while let Ok(event) = event_rx.try_recv() {
                    on_token(event);
                }
                return result;
            }
        }
    }
}

impl Drop for NativeModelHandle {
    fn drop(&mut self) {
        let _ = self.command_tx.send(ModelCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[tauri::command]
pub async fn recycle_autonomic_helper(
    request: AutonomicRecycleRequest,
) -> Result<AutonomicRecycleResponse, NativeRuntimeError> {
    tauri::async_runtime::spawn_blocking(move || perform_autonomic_recycle_sync(request))
        .await
        .map_err(|error| NativeRuntimeError {
            code: "autonomic_recycle_worker_failed",
            message: format!("Autonomic recycle worker failed: {error}"),
        })?
}

pub(crate) fn autonomic_recycle_memory_threshold_bytes() -> u64 {
    AUTONOMIC_RECYCLE_MEMORY_THRESHOLD_BYTES
}

pub(crate) fn autonomic_recycle_allowlist_labels() -> Vec<String> {
    AUTONOMIC_RECYCLE_POLICIES
        .iter()
        .map(|policy| policy.canonical_name.to_string())
        .collect()
}

pub(crate) fn autonomic_recycle_policy_for_process(
    process_name: &str,
    command: &str,
) -> Option<AutonomicRecyclePolicyInfo> {
    matching_autonomic_recycle_policy(process_name, command).map(|policy| {
        AutonomicRecyclePolicyInfo {
            canonical_name: policy.canonical_name,
            category: policy.category,
            restart_available: policy.restart.is_some(),
            restart_strategy: policy.restart.map(autonomic_restart_strategy_label),
        }
    })
}

fn perform_autonomic_recycle_sync(
    request: AutonomicRecycleRequest,
) -> Result<AutonomicRecycleResponse, NativeRuntimeError> {
    let observation = observe_process_for_autonomic_recycle(request.pid)?;
    let policy = validate_autonomic_recycle_candidate(&request, &observation)?;

    if request.dry_run {
        return Ok(AutonomicRecycleResponse {
            status: "validated".to_string(),
            pid: observation.pid,
            process_name: observation.process_name,
            category: policy.category.to_string(),
            resident_memory_bytes: observation.resident_memory_bytes,
            threshold_bytes: AUTONOMIC_RECYCLE_MEMORY_THRESHOLD_BYTES,
            terminated: false,
            restart_attempted: false,
            restart_status: "dry_run".to_string(),
            detail: "Autonomic recycle dry run validated the helper without terminating it."
                .to_string(),
        });
    }

    terminate_process_gracefully(observation.pid)?;
    let restart_status = restart_autonomic_helper(policy)?;
    Ok(AutonomicRecycleResponse {
        status: "recycled".to_string(),
        pid: observation.pid,
        process_name: observation.process_name,
        category: policy.category.to_string(),
        resident_memory_bytes: observation.resident_memory_bytes,
        threshold_bytes: AUTONOMIC_RECYCLE_MEMORY_THRESHOLD_BYTES,
        terminated: true,
        restart_attempted: true,
        restart_status,
        detail: format!(
            "Recycled {} pid {} after RSS exceeded {}.",
            policy.canonical_name,
            observation.pid,
            format_bytes(AUTONOMIC_RECYCLE_MEMORY_THRESHOLD_BYTES)
        ),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AutonomicProcessObservation {
    pid: u32,
    process_name: String,
    command: String,
    resident_memory_bytes: u64,
}

fn observe_process_for_autonomic_recycle(
    pid: u32,
) -> Result<AutonomicProcessObservation, NativeRuntimeError> {
    let mut system = System::new_all();
    system.refresh_all();
    let process = system
        .process(Pid::from_u32(pid))
        .ok_or_else(|| NativeRuntimeError {
            code: "autonomic_recycle_process_not_found",
            message: format!("No live process was found for pid {pid}."),
        })?;
    Ok(AutonomicProcessObservation {
        pid,
        process_name: process.name().to_string_lossy().to_string(),
        command: process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" "),
        resident_memory_bytes: process.memory(),
    })
}

fn validate_autonomic_recycle_candidate(
    request: &AutonomicRecycleRequest,
    observation: &AutonomicProcessObservation,
) -> Result<AutonomicRecyclePolicy, NativeRuntimeError> {
    let policy =
        matching_autonomic_recycle_policy(&observation.process_name, &observation.command).ok_or_else(
            || NativeRuntimeError {
                code: "autonomic_recycle_not_allowlisted",
                message: format!(
                    "Refusing to recycle pid {} ({}): process is not in the OOMU autonomic helper allowlist.",
                    observation.pid, observation.process_name
                ),
            },
        )?;

    if !request.process_name.trim().is_empty()
        && !requested_process_name_matches_policy(
            &request.process_name,
            &observation.process_name,
            policy,
        )
    {
        return Err(NativeRuntimeError {
            code: "autonomic_recycle_process_mismatch",
            message: format!(
                "Refusing to recycle pid {}: requested process '{}' but live process is '{}'.",
                observation.pid, request.process_name, observation.process_name
            ),
        });
    }

    if observation.resident_memory_bytes < AUTONOMIC_RECYCLE_MEMORY_THRESHOLD_BYTES {
        return Err(NativeRuntimeError {
            code: "autonomic_recycle_threshold_not_breached",
            message: format!(
                "Refusing to recycle {} pid {}: RSS {} is below the {} threshold.",
                observation.process_name,
                observation.pid,
                format_bytes(observation.resident_memory_bytes),
                format_bytes(AUTONOMIC_RECYCLE_MEMORY_THRESHOLD_BYTES)
            ),
        });
    }

    if policy.restart.is_none() {
        return Err(NativeRuntimeError {
            code: "autonomic_recycle_restart_unavailable",
            message: format!(
                "Refusing to recycle {} pid {}: this allowlisted helper has no fixed restart strategy.",
                observation.process_name, observation.pid
            ),
        });
    }

    Ok(policy)
}

fn terminate_process_gracefully(pid: u32) -> Result<(), NativeRuntimeError> {
    let output = StdCommand::new("/bin/kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .output()
        .map_err(|error| NativeRuntimeError {
            code: "autonomic_recycle_terminate_failed",
            message: format!("Failed to send TERM to pid {pid}: {error}"),
        })?;
    if !output.status.success() {
        return Err(NativeRuntimeError {
            code: "autonomic_recycle_terminate_failed",
            message: format!(
                "Failed to send TERM to pid {pid}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }

    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(AUTONOMIC_TERMINATE_WAIT_MS) {
        if !process_exists(pid)? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }

    Err(NativeRuntimeError {
        code: "autonomic_recycle_terminate_timeout",
        message: format!(
            "Process pid {pid} did not exit within {} ms after TERM.",
            AUTONOMIC_TERMINATE_WAIT_MS
        ),
    })
}

fn process_exists(pid: u32) -> Result<bool, NativeRuntimeError> {
    StdCommand::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .map_err(|error| NativeRuntimeError {
            code: "autonomic_recycle_process_probe_failed",
            message: format!("Unable to verify whether pid {pid} exited: {error}"),
        })
}

fn restart_autonomic_helper(policy: AutonomicRecyclePolicy) -> Result<String, NativeRuntimeError> {
    match policy.restart {
        Some(AutonomicRestartStrategy::OpenMacApp { app_name }) => restart_mac_app(app_name),
        None => Err(NativeRuntimeError {
            code: "autonomic_recycle_restart_unavailable",
            message: format!("{} has no restart strategy.", policy.canonical_name),
        }),
    }
}

#[cfg(target_os = "macos")]
fn restart_mac_app(app_name: &str) -> Result<String, NativeRuntimeError> {
    let output = StdCommand::new("open")
        .args(["-g", "-j", "-a", app_name])
        .output()
        .map_err(|error| NativeRuntimeError {
            code: "autonomic_recycle_restart_failed",
            message: format!("Failed to restart {app_name}: {error}"),
        })?;
    if output.status.success() {
        Ok(format!("restarted {app_name}"))
    } else {
        Err(NativeRuntimeError {
            code: "autonomic_recycle_restart_failed",
            message: format!(
                "Failed to restart {app_name}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        })
    }
}

#[cfg(not(target_os = "macos"))]
fn restart_mac_app(app_name: &str) -> Result<String, NativeRuntimeError> {
    Err(NativeRuntimeError {
        code: "autonomic_recycle_restart_unsupported",
        message: format!("Restarting {app_name} is only supported on macOS."),
    })
}

fn matching_autonomic_recycle_policy(
    process_name: &str,
    command: &str,
) -> Option<AutonomicRecyclePolicy> {
    let haystack = format!("{process_name} {command}").to_ascii_lowercase();
    AUTONOMIC_RECYCLE_POLICIES.iter().copied().find(|policy| {
        policy
            .match_terms
            .iter()
            .any(|term| process_haystack_contains(&haystack, term))
    })
}

fn process_haystack_contains(haystack: &str, term: &str) -> bool {
    if term.contains(' ') {
        return haystack.contains(term);
    }
    haystack
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
        .any(|part| part.eq_ignore_ascii_case(term))
}

fn requested_process_name_matches_policy(
    requested: &str,
    observed: &str,
    policy: AutonomicRecyclePolicy,
) -> bool {
    requested.eq_ignore_ascii_case(observed)
        || requested.eq_ignore_ascii_case(policy.canonical_name)
        || matching_autonomic_recycle_policy(requested, "").is_some_and(|matched| {
            matched
                .canonical_name
                .eq_ignore_ascii_case(policy.canonical_name)
        })
}

fn autonomic_restart_strategy_label(strategy: AutonomicRestartStrategy) -> &'static str {
    match strategy {
        AutonomicRestartStrategy::OpenMacApp { .. } => DISPLAYLINK_MANAGER_RESTART_LABEL,
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.2} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes / KB)
    } else {
        format!("{bytes:.0} B")
    }
}

pub(crate) async fn execute_codebase_compile(
    app: &tauri::AppHandle,
    request: CodebaseCompileRequest,
) -> ExecuteCommandResponse {
    let target = request.target;
    let target_label = target.as_str();
    let repo_root = match fs::canonicalize(crate::shield_gate::development_repo_root()) {
        Ok(root) => root,
        Err(error) => {
            return codebase_compile_response(
                target,
                false,
                format!(
                    "codebase_compile could not resolve the development repository root: {error}"
                ),
                Vec::new(),
            );
        }
    };

    emit_compile_event(
        app,
        target,
        "queued",
        "system",
        format!(
            "Starting {target_label} compile in {}.",
            repo_root.display()
        ),
        None,
        Instant::now(),
    );

    let mut results = Vec::new();
    for command in codebase_compile_plan(target) {
        let result = match run_codebase_compile_command(app, target, &repo_root, command).await {
            Ok(result) => result,
            Err(message) => {
                return codebase_compile_response(target, false, message, results);
            }
        };
        let failed = result.timed_out || result.exit_code != Some(0);
        let phase = result.phase;
        let display = result.display;
        let exit_code = result.exit_code;
        let timed_out = result.timed_out;
        results.push(result);
        if failed {
            let reason = if timed_out {
                format!(
                    "{phase} timed out after {CODEBASE_COMPILE_TIMEOUT_SECS} seconds while running {display}."
                )
            } else {
                format!("{phase} failed while running {display}. Exit code: {exit_code:?}.")
            };
            return codebase_compile_response(target, false, reason, results);
        }
    }

    if target == CodebaseCompileTarget::Frontend {
        let _ = app.emit(
            CODEBASE_COMPILE_REFRESH_EVENT,
            CodebaseCompileRefreshEvent {
                target: target_label.to_string(),
                reason: "frontend_compile_succeeded".to_string(),
            },
        );
    }

    codebase_compile_response(
        target,
        true,
        format!("{target_label} compile completed successfully."),
        results,
    )
}

async fn run_codebase_compile_command(
    app: &tauri::AppHandle,
    target: CodebaseCompileTarget,
    repo_root: &Path,
    spec: CodebaseCompileCommand,
) -> Result<CodebaseCompileCommandResult, String> {
    let started = Instant::now();
    emit_compile_event(
        app,
        target,
        spec.phase,
        "system",
        format!("Running {}.", spec.display),
        None,
        started,
    );

    let mut command = Command::new(spec.program);
    command
        .args(spec.args)
        .current_dir(repo_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|error| {
        format!(
            "codebase_compile failed to start {} in {}: {error}",
            spec.display,
            repo_root.display()
        )
    })?;

    let stdout = child.stdout.take().ok_or_else(|| {
        format!(
            "codebase_compile could not capture stdout for {}.",
            spec.display
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        format!(
            "codebase_compile could not capture stderr for {}.",
            spec.display
        )
    })?;
    let stdout_task = spawn_compile_log_reader(
        app.clone(),
        target,
        spec.phase.to_string(),
        "stdout".to_string(),
        stdout,
        started,
    );
    let stderr_task = spawn_compile_log_reader(
        app.clone(),
        target,
        spec.phase.to_string(),
        "stderr".to_string(),
        stderr,
        started,
    );

    let mut timed_out = false;
    let status = match time::timeout(
        Duration::from_secs(CODEBASE_COMPILE_TIMEOUT_SECS),
        child.wait(),
    )
    .await
    {
        Ok(Ok(status)) => Some(status),
        Ok(Err(error)) => {
            return Err(format!(
                "codebase_compile failed while waiting for {}: {error}",
                spec.display
            ));
        }
        Err(_) => {
            timed_out = true;
            let _ = child.kill().await;
            child.wait().await.ok()
        }
    };

    let stdout = stdout_task.await.unwrap_or_default();
    let stderr = stderr_task.await.unwrap_or_default();
    let exit_code = status.and_then(|status| status.code());
    let final_line = if timed_out {
        format!(
            "{} timed out after {CODEBASE_COMPILE_TIMEOUT_SECS} seconds.",
            spec.display
        )
    } else {
        format!("{} exited with code {:?}.", spec.display, exit_code)
    };
    emit_compile_event(
        app, target, spec.phase, "system", final_line, exit_code, started,
    );

    Ok(CodebaseCompileCommandResult {
        phase: spec.phase,
        display: spec.display,
        exit_code,
        timed_out,
        stdout,
        stderr,
    })
}

fn spawn_compile_log_reader<R>(
    app: tauri::AppHandle,
    target: CodebaseCompileTarget,
    phase: String,
    stream: String,
    reader: R,
    started: Instant,
) -> tokio::task::JoinHandle<String>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut collected = String::new();
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    collected.push_str(&line);
                    collected.push('\n');
                    emit_compile_event(&app, target, &phase, &stream, line, None, started);
                }
                Ok(None) => break,
                Err(error) => {
                    let line = format!("Failed to read {stream}: {error}");
                    collected.push_str(&line);
                    collected.push('\n');
                    emit_compile_event(&app, target, &phase, &stream, line, None, started);
                    break;
                }
            }
        }
        collected
    })
}

fn emit_compile_event(
    app: &tauri::AppHandle,
    target: CodebaseCompileTarget,
    phase: &str,
    stream: &str,
    line: impl Into<String>,
    exit_code: Option<i32>,
    started: Instant,
) {
    let _ = app.emit(
        CODEBASE_COMPILE_LOG_EVENT,
        CodebaseCompileLogEvent {
            target: target.as_str().to_string(),
            phase: phase.to_string(),
            stream: stream.to_string(),
            line: line.into(),
            elapsed_ms: started.elapsed().as_millis(),
            exit_code,
        },
    );
}

fn codebase_compile_response(
    target: CodebaseCompileTarget,
    success: bool,
    summary: String,
    results: Vec<CodebaseCompileCommandResult>,
) -> ExecuteCommandResponse {
    let mut message = summary;
    for result in &results {
        message.push_str(&format!(
            "\n\n[{}] {} exit={:?} timed_out={}",
            result.phase, result.display, result.exit_code, result.timed_out
        ));
        let stdout = truncate_compile_output(&result.stdout, 1800);
        let stderr = truncate_compile_output(&result.stderr, 2200);
        if !stdout.trim().is_empty() {
            message.push_str(&format!("\nstdout:\n{stdout}"));
        }
        if !stderr.trim().is_empty() {
            message.push_str(&format!("\nstderr:\n{stderr}"));
        }
    }

    ExecuteCommandResponse {
        operation: "codebase_compile".to_string(),
        status: if success {
            CommandStatus::Completed
        } else {
            CommandStatus::Failed
        },
        message,
        metrics: None,
        claims: vec![format!(
            "CLAIM codebase_compile target={} success={} phases={}",
            target.as_str(),
            success,
            results.len()
        )],
        verified: success,
        model_used: None,
    }
}

fn codebase_compile_plan(target: CodebaseCompileTarget) -> Vec<CodebaseCompileCommand> {
    match target {
        CodebaseCompileTarget::Backend => vec![
            CodebaseCompileCommand {
                phase: "preflight",
                program: "cargo",
                args: &["check", "--manifest-path", "src-tauri/Cargo.toml"],
                display: "cargo check --manifest-path src-tauri/Cargo.toml",
            },
            CodebaseCompileCommand {
                phase: "build",
                program: "cargo",
                args: &["build", "--manifest-path", "src-tauri/Cargo.toml"],
                display: "cargo build --manifest-path src-tauri/Cargo.toml",
            },
        ],
        CodebaseCompileTarget::Frontend => vec![
            CodebaseCompileCommand {
                phase: "preflight",
                program: "npm",
                args: &["run", "typecheck"],
                display: "npm run typecheck",
            },
            CodebaseCompileCommand {
                phase: "build",
                program: "npm",
                args: &["run", "build"],
                display: "npm run build",
            },
        ],
    }
}

fn truncate_compile_output(value: &str, max_chars: usize) -> String {
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

enum ResidentCycleExit {
    Dormant,
    Shutdown,
}

fn discover_multimodal_projector(model_path: &Path) -> Option<PathBuf> {
    let directory = model_path.parent()?;
    let mut candidates = fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.to_ascii_lowercase().contains("mmproj"))
        })
        .filter(|path| fs::metadata(path).is_ok_and(|metadata| metadata.len() > 1024 * 1024))
        .collect::<Vec<_>>();
    candidates.sort();
    (candidates.len() == 1).then(|| candidates.remove(0))
}

fn create_multimodal_context(
    model: &LlamaModel,
    projector_path: &Path,
    config: &RuntimeConfig,
) -> Result<MtmdContext, NativeRuntimeError> {
    let projector = projector_path.to_str().ok_or_else(|| NativeRuntimeError {
        code: "local_model_multimodal_projector_invalid",
        message: "The local image projector path is invalid.".to_string(),
    })?;
    let marker = CString::new(mtmd_default_marker()).map_err(|_| NativeRuntimeError {
        code: "local_model_multimodal_marker_invalid",
        message: "The local image marker could not be initialized.".to_string(),
    })?;
    let params = MtmdContextParams {
        use_gpu: config.requested_gpu_layers > 0,
        print_timings: false,
        n_threads: config.batch_threads.max(1),
        media_marker: marker,
        image_min_tokens: -1,
        image_max_tokens: -1,
    };
    let context = MtmdContext::init_from_file(projector, model, &params).map_err(|error| {
        NativeRuntimeError {
            code: "local_model_multimodal_projector_load_failed",
            message: format!("The local image projector could not be loaded: {error}"),
        }
    })?;
    if !context.support_vision() {
        return Err(NativeRuntimeError {
            code: "local_model_vision_unsupported",
            message: "The selected local model does not support image input.".to_string(),
        });
    }
    Ok(context)
}

impl CacheManager {
    fn new(max_sessions: u32) -> Self {
        Self {
            sessions: HashMap::new(),
            max_sessions: max_sessions as usize,
        }
    }

    fn mark_all_dormant(&mut self) {
        for session in self.sessions.values_mut() {
            session.resident = false;
        }
    }

    fn append(
        &mut self,
        model: &LlamaModel,
        context: &mut LlamaContext<'_>,
        request: NativeSessionRequest,
        context_size: usize,
        batch_size: usize,
        default_pinned_tokens: usize,
        cancellation: Option<&AtomicBool>,
        mut on_prefill_progress: impl FnMut(usize, usize),
    ) -> Result<NativeSessionStats, NativeRuntimeError> {
        let session_id = normalize_session_id(&request.session_id);
        self.ensure_session_slot(context, &session_id);
        let system_changed = !request.prompt_is_full_context
            && self
                .sessions
                .get(&session_id)
                .is_some_and(|session| session.system_prompt != request.system_prompt);

        if system_changed {
            if let Some(session) = self.sessions.get_mut(&session_id) {
                if session.resident {
                    clear_sequence(context, session.sequence_id)?;
                }
                session.tokens.clear();
                session.source_tokens.clear();
                session.pinned_tokens = 0;
                session.resident = false;
                session.system_prompt = request.system_prompt.clone();
            }
        }

        let is_new_context = self
            .sessions
            .get(&session_id)
            .is_none_or(|session| session.tokens.is_empty());

        // Under n_seq_max = 1, all sessions share sequence_id = 0.
        // If the active session is not currently resident in the KV cache,
        // we must clear sequence 0 and mark all other sessions as non-resident
        // to prevent cache collision or corrupting sequence 0.
        let active_is_resident = self
            .sessions
            .get(&session_id)
            .is_some_and(|session| session.resident);

        if !active_is_resident {
            clear_sequence(context, 0)?;
            for session in self.sessions.values_mut() {
                session.resident = false;
            }
        }

        let mut incoming = Vec::new();
        let mut pinned_tokens = None;
        if request.prompt_is_full_context {
            incoming.extend(tokenize(model, request.prompt.trim(), AddBos::Always)?);
            pinned_tokens = request
                .system_prompt
                .as_deref()
                .map(str::trim)
                .filter(|prompt| !prompt.is_empty())
                .map(|prompt| tokenize(model, prompt, AddBos::Always))
                .transpose()?
                .map(|tokens| tokens.len().min(incoming.len()));
        } else if is_new_context {
            if let Some(system_prompt) = request
                .system_prompt
                .as_deref()
                .map(str::trim)
                .filter(|prompt| !prompt.is_empty())
            {
                incoming.extend(tokenize(model, system_prompt, AddBos::Always)?);
                incoming.extend(tokenize(model, "\n\n", AddBos::Never)?);
                pinned_tokens = Some(incoming.len());
            }
            incoming.extend(tokenize(
                model,
                request.prompt.trim(),
                if incoming.is_empty() {
                    AddBos::Always
                } else {
                    AddBos::Never
                },
            )?);
        } else {
            incoming.extend(tokenize(
                model,
                &format!("\n\n{}", request.prompt.trim()),
                AddBos::Never,
            )?);
        }
        if incoming.is_empty() {
            return Err(NativeRuntimeError {
                code: "llama_empty_token_sequence",
                message: "llama.cpp tokenization produced no input tokens.".to_string(),
            });
        }

        let requested_pinned = pinned_tokens.unwrap_or_else(|| {
            default_pinned_tokens.min(
                self.sessions
                    .get(&session_id)
                    .map_or(incoming.len(), |session| {
                        session.tokens.len().saturating_add(incoming.len())
                    }),
            )
        });
        let mut shifted_tokens = 0;
        let mut evicted_sessions = 0;
        let maximum_session_tokens = context_size.saturating_sub(1).max(1);

        if request.prompt_is_full_context {
            let target_tokens = incoming;
            let session = self
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| missing_session_error(&session_id))?;
            if target_tokens.starts_with(&session.source_tokens) {
                incoming = target_tokens[session.source_tokens.len()..].to_vec();
            } else if session.source_tokens == session.tokens {
                let common_prefix = common_prefix_len(&session.tokens, &target_tokens);
                if common_prefix < session.tokens.len() {
                    if session.resident {
                        context
                            .clear_kv_cache_seq(
                                Some(session.sequence_id as u32),
                                Some(common_prefix as u32),
                                None,
                            )
                            .map_err(kv_error)?;
                    }
                    session.tokens.truncate(common_prefix);
                }
                incoming = target_tokens[common_prefix..].to_vec();
            } else {
                if session.resident {
                    clear_sequence(context, session.sequence_id)?;
                }
                session.tokens.clear();
                session.resident = false;
                incoming = target_tokens.clone();
            }
            // Regeneration guard: when the full-context prompt is already fully cached (an identical
            // prompt, e.g. a bounded retry resampling the same turn after an empty or leaked answer),
            // the caching logic above leaves `incoming` empty. Decoding zero tokens fails with
            // `n_tokens == 0`, so rewind the final cached token: re-decoding exactly one token
            // restores logits at the end of the prompt and lets the sampler draw a fresh
            // continuation (the sampler is re-seeded on every generation).
            if incoming.is_empty() && !session.tokens.is_empty() {
                let rewind_to = session.tokens.len() - 1;
                if session.resident {
                    context
                        .clear_kv_cache_seq(
                            Some(session.sequence_id as u32),
                            Some(rewind_to as u32),
                            None,
                        )
                        .map_err(kv_error)?;
                }
                incoming = vec![session.tokens[rewind_to]];
                session.tokens.truncate(rewind_to);
            }
            session.source_tokens = target_tokens;
            session.pinned_tokens = requested_pinned
                .min(maximum_session_tokens / 2)
                .min(session.source_tokens.len());
            session.system_prompt = request.system_prompt.clone();
        }

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| missing_session_error(&session_id))?;
        if is_new_context && !request.prompt_is_full_context {
            session.pinned_tokens = requested_pinned
                .min(maximum_session_tokens / 2)
                .min(incoming.len());
            session.system_prompt = request.system_prompt.clone();
        }
        let incoming_pinned = if session.tokens.is_empty() {
            session.pinned_tokens.min(incoming.len())
        } else {
            0
        };
        let shift_plan = plan_context_shift(
            session.tokens.len(),
            session.pinned_tokens.min(session.tokens.len()),
            incoming.len(),
            incoming_pinned,
            maximum_session_tokens,
        );
        shifted_tokens += shift_session(context, session, shift_plan.cached_tokens)?;
        if shift_plan.incoming_tokens > 0 {
            incoming
                .drain(incoming_pinned..incoming_pinned.saturating_add(shift_plan.incoming_tokens));
            shifted_tokens += shift_plan.incoming_tokens;
        }

        let needed = {
            let session = self
                .sessions
                .get(&session_id)
                .ok_or_else(|| missing_session_error(&session_id))?;
            if session.resident {
                incoming.len()
            } else {
                session.tokens.len().saturating_add(incoming.len())
            }
        };
        evicted_sessions += self.evict_lru_until_fit(context, &session_id, needed, context_size)?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| missing_session_error(&session_id))?;
        let cached_tokens = if session.resident {
            session.tokens.len()
        } else {
            0
        };
        let cold_start = !session.resident;
        let mut tokens_to_evaluate = Vec::with_capacity(needed);
        if !session.resident {
            tokens_to_evaluate.extend_from_slice(&session.tokens);
        }
        tokens_to_evaluate.extend_from_slice(&incoming);
        if let Err(error) = prefill::decode_tokens(
            context,
            session.sequence_id,
            cached_tokens,
            &tokens_to_evaluate,
            batch_size,
            cancellation,
            |evaluated, total| on_prefill_progress(evaluated, total),
        ) {
            let _ = clear_sequence(context, session.sequence_id);
            prefill::invalidate_failed_prefill(session);
            return Err(error);
        }
        session.tokens.extend(incoming);
        if !request.prompt_is_full_context {
            session.source_tokens.clone_from(&session.tokens);
        }
        session.resident = true;
        session.last_used = Instant::now();

        Ok(NativeSessionStats {
            session_id,
            cached_tokens,
            evaluated_tokens: tokens_to_evaluate.len(),
            context_tokens: session.tokens.len(),
            pinned_tokens: session.pinned_tokens,
            shifted_tokens,
            evicted_sessions,
            cold_start,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn generate(
        &mut self,
        model: &LlamaModel,
        context: &mut LlamaContext<'_>,
        request: NativeGenerationRequest,
        event_tx: mpsc::Sender<NativeTokenEvent>,
        context_size: usize,
        batch_size: usize,
        default_pinned_tokens: usize,
    ) -> Result<NativeGenerationResult, NativeRuntimeError> {
        let started = Instant::now();
        if request.cancellation.load(Ordering::Acquire) {
            return Ok(prefill::cancelled_generation_result(
                &request.session.session_id,
            ));
        }
        let cancelled_session_id = request.session.session_id.clone();
        let session_stats = match self.append(
            model,
            context,
            request.session.clone(),
            context_size,
            batch_size,
            default_pinned_tokens,
            Some(request.cancellation.as_ref()),
            |evaluated, _total| {
                let _ = event_tx.send(prefill::progress_event(
                    evaluated,
                    started.elapsed().as_millis(),
                ));
            },
        ) {
            Ok(stats) => stats,
            Err(error) if error.code == "local_inference_cancelled" => {
                return Ok(prefill::cancelled_generation_result(&cancelled_session_id));
            }
            Err(error) => return Err(error),
        };
        let session_id = session_stats.session_id.clone();
        let prompt_tokens = self
            .sessions
            .get(&session_id)
            .map(|session| session.tokens.clone())
            .ok_or_else(|| NativeRuntimeError {
                code: "llama_session_missing",
                message: "The llama.cpp session disappeared before generation.".to_string(),
            })?;
        let mut sampler = build_sampler(model, &request, &prompt_tokens)?;
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        // The gemma4 template opens a hidden `<|channel>thought` channel as the generation
        // prefix (see format_gemma4_chat_prompt). When it does, generation begins INSIDE the
        // thought channel and the opening marker is never emitted, so the sanitizer must
        // start already suppressing until the model switches to the visible `<|channel>text`.
        let start_in_thought = prompt_opens_reasoning_channel(&request.session.prompt);
        let mut sanitizer = TokenPieceSanitizer::new(true, start_in_thought);
        let mut text = String::new();
        let mut raw_text = String::new();
        let mut token_ids = Vec::with_capacity(request.max_new_tokens);
        let mut time_to_first_token_ms = 0;
        let maximum_session_tokens = context_size.saturating_sub(1).max(1);
        let has_grammar = request.grammar.is_some();

        for sequence in 1..=request.max_new_tokens {
            if request.cancellation.load(Ordering::Acquire) {
                break;
            }

            let token = if has_grammar {
                let mut token_data_array = context.token_data_array();
                token_data_array.apply_sampler(&sampler);
                token_data_array
                    .selected_token()
                    .ok_or_else(|| NativeRuntimeError {
                        code: "llama_grammar_sampling_failed",
                        message: "The grammar sampler failed to select any token candidate."
                            .to_string(),
                    })?
            } else {
                sampler.sample(context, -1)
            };
            sampler.accept(token);
            if model.is_eog_token(token) {
                break;
            }

            // Render with special tokens VISIBLE (special = true). With special = false,
            // llama.cpp collapses the channel/turn control tokens (<|channel>, <channel|>,
            // <|turn>, ...) to empty strings before the sanitizer ever sees them, which is
            // what let the model's hidden reasoning leak into the chat. Keeping them as text
            // lets the sanitizer detect the thought->text channel switch and strip reasoning.
            let piece = model
                .token_to_piece(token, &mut decoder, true, None)
                .map_err(|error| NativeRuntimeError {
                    code: "llama_token_decode_failed",
                    message: format!(
                        "llama.cpp could not decode generated token {}: {error}",
                        token.0
                    ),
                })?;
            raw_text.push_str(&piece);
            let sanitized = sanitizer.push(&piece);
            let elapsed_ms = started.elapsed().as_millis();
            if time_to_first_token_ms == 0 {
                time_to_first_token_ms = elapsed_ms;
            }
            token_ids.push(token.0);
            text.push_str(&sanitized);
            let _ = event_tx.send(NativeTokenEvent {
                sequence,
                token_id: token.0,
                text: sanitized,
                elapsed_ms,
            });

            self.evict_lru_until_fit(context, &session_id, 1, context_size)?;
            let session = self
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| missing_session_error(&session_id))?;
            if session.tokens.len() >= maximum_session_tokens {
                shift_session(context, session, 1)?;
            }
            let position = session.tokens.len();
            prefill::decode_generated_token(context, session.sequence_id, position, token)?;
            session.tokens.push(token);
            session.source_tokens.push(token);
            session.last_used = Instant::now();
        }

        Ok(NativeGenerationResult {
            text: {
                text.push_str(&sanitizer.finish());
                text
            },
            raw_text,
            token_ids,
            time_to_first_token_ms,
            cancelled: request.cancellation.load(Ordering::Acquire),
            session_stats,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn generate_multimodal(
        &mut self,
        model: &LlamaModel,
        context: &mut LlamaContext<'_>,
        multimodal: &MtmdContext,
        request: NativeGenerationRequest,
        event_tx: mpsc::Sender<NativeTokenEvent>,
        context_size: usize,
        batch_size: usize,
    ) -> Result<NativeGenerationResult, NativeRuntimeError> {
        let started = Instant::now();
        let session_id = normalize_session_id(&request.session.session_id);
        if request.cancellation.load(Ordering::Acquire) {
            return Ok(prefill::cancelled_generation_result(&session_id));
        }
        let marker_count = request
            .session
            .prompt
            .matches(mtmd_default_marker())
            .count();
        if marker_count != request.media.len() {
            return Err(NativeRuntimeError {
                code: "local_model_multimodal_marker_mismatch",
                message: "The approved images could not be matched to this request. Choose the images again."
                    .to_string(),
            });
        }
        if request
            .media
            .iter()
            .any(|media| !media.mime_type.starts_with("image/") || media.bytes.is_empty())
        {
            return Err(NativeRuntimeError {
                code: "local_model_multimodal_input_invalid",
                message: "The local vision route received an invalid image.".to_string(),
            });
        }

        context.clear_kv_cache();
        self.mark_all_dormant();
        let bitmaps = request
            .media
            .iter()
            .map(|media| {
                MtmdBitmap::from_buffer(multimodal, &media.bytes, false).map_err(|error| {
                    NativeRuntimeError {
                        code: "local_model_multimodal_image_decode_failed",
                        message: format!(
                            "The approved image '{}' could not be decoded: {error}",
                            media.name
                        ),
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let bitmap_refs = bitmaps.iter().collect::<Vec<_>>();
        let chunks = multimodal
            .tokenize(
                MtmdInputText {
                    text: request.session.prompt.clone(),
                    add_special: true,
                    parse_special: true,
                },
                &bitmap_refs,
            )
            .map_err(|error| NativeRuntimeError {
                code: "local_model_multimodal_tokenization_failed",
                message: format!("The local model could not prepare the approved image: {error}"),
            })?;
        let evaluated_tokens = chunks.total_tokens();
        let evaluated_positions = chunks.total_positions();
        if evaluated_tokens == 0
            || evaluated_positions <= 0
            || evaluated_tokens >= context_size
            || usize::try_from(evaluated_positions).unwrap_or(usize::MAX) >= context_size
        {
            return Err(NativeRuntimeError {
                code: "local_model_multimodal_context_exceeded",
                message: "The approved image and conversation do not fit in the selected local model's context."
                    .to_string(),
            });
        }
        let prompt_tokens = (0..chunks.len())
            .filter_map(|index| chunks.get(index))
            .filter_map(|chunk| chunk.text_tokens().map(ToOwned::to_owned))
            .flatten()
            .collect::<Vec<_>>();
        let n_batch = i32::try_from(batch_size).map_err(|_| NativeRuntimeError {
            code: "local_model_multimodal_batch_invalid",
            message: "The local image batch size is invalid.".to_string(),
        })?;
        let mut current_position = chunks
            .eval_chunks(multimodal, context, 0, 0, n_batch, true)
            .map_err(|error| NativeRuntimeError {
                code: "local_model_multimodal_evaluation_failed",
                message: format!("The local model could not evaluate the approved image: {error}"),
            })?;
        let _ = event_tx.send(prefill::progress_event(
            evaluated_tokens,
            started.elapsed().as_millis(),
        ));

        let mut sampler = build_sampler(model, &request, &prompt_tokens)?;
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        let start_in_thought = prompt_opens_reasoning_channel(&request.session.prompt);
        let mut sanitizer = TokenPieceSanitizer::new(true, start_in_thought);
        let mut text = String::new();
        let mut raw_text = String::new();
        let mut token_ids = Vec::with_capacity(request.max_new_tokens);
        let mut time_to_first_token_ms = 0;
        let has_grammar = request.grammar.is_some();

        for sequence in 1..=request.max_new_tokens {
            if request.cancellation.load(Ordering::Acquire)
                || usize::try_from(current_position).unwrap_or(usize::MAX)
                    >= context_size.saturating_sub(1)
            {
                break;
            }
            let token = if has_grammar {
                let mut token_data_array = context.token_data_array();
                token_data_array.apply_sampler(&sampler);
                token_data_array
                    .selected_token()
                    .ok_or_else(|| NativeRuntimeError {
                        code: "llama_grammar_sampling_failed",
                        message: "The grammar sampler failed to select any token candidate."
                            .to_string(),
                    })?
            } else {
                sampler.sample(context, -1)
            };
            sampler.accept(token);
            if model.is_eog_token(token) {
                break;
            }
            let piece = model
                .token_to_piece(token, &mut decoder, true, None)
                .map_err(|error| NativeRuntimeError {
                    code: "llama_token_decode_failed",
                    message: format!(
                        "llama.cpp could not decode generated token {}: {error}",
                        token.0
                    ),
                })?;
            raw_text.push_str(&piece);
            let sanitized = sanitizer.push(&piece);
            let elapsed_ms = started.elapsed().as_millis();
            if time_to_first_token_ms == 0 {
                time_to_first_token_ms = elapsed_ms;
            }
            token_ids.push(token.0);
            text.push_str(&sanitized);
            let _ = event_tx.send(NativeTokenEvent {
                sequence,
                token_id: token.0,
                text: sanitized,
                elapsed_ms,
            });
            let position = usize::try_from(current_position).map_err(|_| NativeRuntimeError {
                code: "local_model_multimodal_position_invalid",
                message: "The local image position became invalid.".to_string(),
            })?;
            prefill::decode_generated_token(context, 0, position, token)?;
            current_position = current_position.saturating_add(1);
        }

        text.push_str(&sanitizer.finish());
        let session_stats = NativeSessionStats {
            session_id,
            cached_tokens: 0,
            evaluated_tokens,
            context_tokens: usize::try_from(current_position).unwrap_or(context_size),
            pinned_tokens: 0,
            shifted_tokens: 0,
            evicted_sessions: 0,
            cold_start: true,
        };
        context.clear_kv_cache();
        self.mark_all_dormant();
        Ok(NativeGenerationResult {
            text,
            raw_text,
            token_ids,
            time_to_first_token_ms,
            cancelled: request.cancellation.load(Ordering::Acquire),
            session_stats,
        })
    }

    fn ensure_session_slot(&mut self, context: &mut LlamaContext<'_>, session_id: &str) {
        if self.sessions.contains_key(session_id) {
            return;
        }
        if self.sessions.len() >= self.max_sessions {
            if let Some(oldest) = self
                .sessions
                .iter()
                .min_by_key(|(_, session)| session.last_used)
                .map(|(id, _)| id.clone())
            {
                if let Some(session) = self.sessions.remove(&oldest) {
                    if session.resident {
                        let _ = clear_sequence(context, session.sequence_id);
                    }
                }
            }
        }
        self.sessions.insert(
            session_id.to_string(),
            SessionCache {
                sequence_id: 0,
                tokens: Vec::new(),
                source_tokens: Vec::new(),
                pinned_tokens: 0,
                system_prompt: None,
                resident: false,
                last_used: Instant::now(),
            },
        );
    }

    fn evict_lru_until_fit(
        &mut self,
        context: &mut LlamaContext<'_>,
        active_session_id: &str,
        needed_tokens: usize,
        context_size: usize,
    ) -> Result<usize, NativeRuntimeError> {
        let mut evicted = 0;
        while self.resident_tokens().saturating_add(needed_tokens) > context_size {
            let candidate = self
                .sessions
                .iter()
                .filter(|(id, session)| id.as_str() != active_session_id && session.resident)
                .min_by_key(|(_, session)| session.last_used)
                .map(|(id, _)| id.clone());
            let Some(candidate) = candidate else {
                break;
            };
            let session = self
                .sessions
                .get_mut(&candidate)
                .ok_or_else(|| missing_session_error(&candidate))?;
            clear_sequence(context, session.sequence_id)?;
            session.resident = false;
            evicted += 1;
        }
        Ok(evicted)
    }

    fn resident_tokens(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.resident)
            .map(|session| session.tokens.len())
            .sum()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ContextShiftPlan {
    cached_tokens: usize,
    incoming_tokens: usize,
}

fn plan_context_shift(
    cached_tokens: usize,
    pinned_cached_tokens: usize,
    incoming_tokens: usize,
    pinned_incoming_tokens: usize,
    maximum_tokens: usize,
) -> ContextShiftPlan {
    let overflow = cached_tokens
        .saturating_add(incoming_tokens)
        .saturating_sub(maximum_tokens);
    let cached_drop = overflow.min(cached_tokens.saturating_sub(pinned_cached_tokens));
    let remaining = overflow.saturating_sub(cached_drop);
    let incoming_drop = remaining.min(incoming_tokens.saturating_sub(pinned_incoming_tokens));
    ContextShiftPlan {
        cached_tokens: cached_drop,
        incoming_tokens: incoming_drop,
    }
}

fn common_prefix_len<T: PartialEq>(cached: &[T], next: &[T]) -> usize {
    cached
        .iter()
        .zip(next.iter())
        .take_while(|(cached, next)| cached == next)
        .count()
}

fn tokenize(
    model: &LlamaModel,
    text: &str,
    add_bos: AddBos,
) -> Result<Vec<LlamaToken>, NativeRuntimeError> {
    model
        .str_to_token(text, add_bos)
        .map_err(|error| NativeRuntimeError {
            code: "llama_tokenization_failed",
            message: format!("llama.cpp could not tokenize the session input: {error}"),
        })
}

fn build_sampler(
    model: &LlamaModel,
    request: &NativeGenerationRequest,
    prompt_tokens: &[LlamaToken],
) -> Result<LlamaSampler, NativeRuntimeError> {
    let mut samplers = Vec::new();
    let has_grammar = request.grammar.is_some();
    if let Some(grammar) = request.grammar.as_deref() {
        samplers.push(
            LlamaSampler::grammar(model, grammar, "root").map_err(|error| NativeRuntimeError {
                code: "llama_grammar_invalid",
                message: format!("llama.cpp rejected the structured-output grammar: {error}"),
            })?,
        );
    }
    samplers.push(LlamaSampler::penalties(
        64,
        request.repeat_penalty,
        0.0,
        0.0,
    ));
    if request.temperature <= 0.0 {
        samplers.push(LlamaSampler::greedy());
    } else {
        samplers.extend([
            LlamaSampler::top_k(request.top_k.max(1)),
            LlamaSampler::top_p(request.top_p.clamp(0.05, 1.0), 1),
            LlamaSampler::temp(request.temperature.clamp(0.01, 2.0)),
            LlamaSampler::dist(u32::MAX),
        ]);
    }
    let chain = LlamaSampler::chain_simple(samplers);

    // Seeding the chain with the prompt tokens primes the repetition-penalty
    // sampler with recent context — but `with_tokens` feeds them through EVERY
    // sampler's `accept`, including the grammar. A GBNF grammar describes the
    // model's OUTPUT (the workflow IR JSON), not the prompt, so accepting prompt
    // tokens drives the grammar state machine into an invalid state and llama.cpp
    // aborts the whole process (`llama_grammar_accept_impl` fatal error at
    // llama-grammar.cpp). The grammar must instead start at its `root` rule and
    // advance only on generated tokens (see `sampler.accept(token)` in
    // `generate`). So only seed the prompt when there is no grammar in the chain.
    if has_grammar {
        Ok(chain)
    } else {
        Ok(chain.with_tokens(prompt_tokens))
    }
}

const TOKEN_MARKERS: &[&str] = &[
    "<bos>",
    "<eos>",
    "<pad>",
    "<unk>",
    "<|endoftext|>",
    "<|eot_id|>",
    "<|start_header_id|>",
    "<|end_header_id|>",
    "<|channel>thought",
    "<|channel>text",
    "<|channel>final",
    "<|channel>answer",
    "<|channel>assistant",
    "<|channel>",
    "<|channel|>",
    "<channel|>",
    "<|turn>",
    "<|turn|>",
    "<turn|>",
    "<think>",
    "</think>",
    "<|think|>",
    // Remaining gemma4 control tokens. These must never surface as visible text now that
    // generation detokenizes with special tokens rendered (token_to_piece special = true);
    // otherwise stray tool/multimodal markers leak into the chat the same way reasoning did.
    "<|tool_call>",
    "<|tool_response>",
    "<|tool>",
    "<tool_call|>",
    "<tool_response|>",
    "<tool|>",
    "<|image|>",
    "<|audio|>",
    "<|video|>",
    "<|\"|>",
];

/// Detect whether a fully rendered prompt ends with an open hidden-reasoning channel.
///
/// The gemma4 chat template opens `<|channel>thought<channel|>` as the generation prefix
/// whenever thinking is disabled, so the model begins generating INSIDE the thought channel
/// and never emits the opening marker. In that case the streaming sanitizer must start in
/// the suppressed state and only reveal output once the model switches to the visible
/// `<|channel>text` channel. Prompts that do not open a thought channel (e.g. grammar-
/// constrained workflow prompts) return false and stream normally.
fn prompt_opens_reasoning_channel(prompt: &str) -> bool {
    let tail_start = prompt.rfind("<|turn>model").unwrap_or(0);
    let tail = &prompt[tail_start..];
    match (
        tail.rfind("<|channel>thought"),
        tail.rfind("<|channel>text"),
    ) {
        (Some(thought_idx), Some(text_idx)) => thought_idx > text_idx,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// Bare channel-name words. Some gemma4 checkpoints emit the channel label as ordinary
/// text adjacent to the `<|channel>`/`<channel|>` control tokens (e.g. the 12B emits
/// `thought<|channel><channel|>answer` instead of `<|channel>thought<channel|>answer`), so
/// the label word itself must be dropped when it sits against a channel marker.
const CHANNEL_LABELS: &[&str] = &[
    "thought",
    "analysis",
    "reasoning",
    "thinking",
    "text",
    "final",
    "answer",
    "commentary",
];

fn is_channel_label(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && CHANNEL_LABELS
            .iter()
            .any(|label| label.eq_ignore_ascii_case(trimmed))
}

fn needs_token_piece_boundary_space(previous: Option<char>, next: Option<char>) -> bool {
    let Some(previous_char) = previous else {
        return false;
    };
    let Some(next_char) = next else {
        return false;
    };
    if previous_char.is_whitespace() || next_char.is_whitespace() {
        return false;
    }
    if previous_char.is_alphanumeric() && next_char.is_alphanumeric() {
        return true;
    }
    matches!(
        previous_char,
        '.' | ',' | '!' | '?' | ';' | ':' | ')' | ']' | '}' | '"' | '\''
    ) && next_char.is_alphanumeric()
}

struct TokenPieceSanitizer {
    pending: String,
    strip_thoughts: bool,
    in_thought: bool,
    produced_visible: bool,
    separator_pending: bool,
    last_visible_char: Option<char>,
}

impl Default for TokenPieceSanitizer {
    fn default() -> Self {
        Self {
            pending: String::new(),
            strip_thoughts: false,
            in_thought: false,
            produced_visible: false,
            separator_pending: false,
            last_visible_char: None,
        }
    }
}

impl TokenPieceSanitizer {
    fn new(strip_thoughts: bool, start_in_thought: bool) -> Self {
        Self {
            pending: String::new(),
            strip_thoughts,
            in_thought: start_in_thought,
            produced_visible: false,
            separator_pending: false,
            last_visible_char: None,
        }
    }

    fn note_stripped_marker(&mut self) {
        if self
            .last_visible_char
            .is_some_and(|character| !character.is_whitespace())
        {
            self.separator_pending = true;
        }
    }

    fn push_visible_text(&mut self, output: &mut String, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.separator_pending {
            if needs_token_piece_boundary_space(self.last_visible_char, text.chars().next()) {
                output.push(' ');
            }
            self.separator_pending = false;
        }
        output.push_str(text);
        if !text.trim().is_empty() {
            self.produced_visible = true;
        }
        if let Some(character) = text.chars().next_back() {
            self.last_visible_char = Some(character);
        }
    }

    fn push(&mut self, piece: &str) -> String {
        let mut input = std::mem::take(&mut self.pending);
        input.push_str(piece);
        let mut output = String::new();

        while let Some(marker_start) = input.find('<') {
            let before_marker = &input[..marker_start];
            let candidate = &input[marker_start..];
            if !self.strip_thoughts || !self.in_thought {
                // Drop a leading bare channel-label word ("thought", "text", ...) emitted as
                // plain text immediately before a channel control token. Some gemma4
                // checkpoints announce the channel as `thought<|channel><channel|>` rather
                // than `<|channel>thought<channel|>`, which would otherwise leak the label.
                let drop_label = self.strip_thoughts
                    && !self.produced_visible
                    && is_channel_label(before_marker)
                    && (candidate.starts_with("<|channel>") || candidate.starts_with("<channel|>"));
                if !drop_label {
                    self.push_visible_text(&mut output, before_marker);
                }
            }

            // Check for channel/thought markers
            if candidate.starts_with("<|channel>thought") {
                self.note_stripped_marker();
                if self.strip_thoughts {
                    self.in_thought = true;
                }
                input = candidate["<|channel>thought".len()..].to_string();
                continue;
            }
            let mut matched_text_marker = false;
            let mut marker_len = 0;
            for prefix in [
                "<|channel>text",
                "<|channel>final",
                "<|channel>answer",
                "<|channel>assistant",
            ] {
                if candidate.starts_with(prefix) {
                    matched_text_marker = true;
                    marker_len = prefix.len();
                    break;
                }
            }
            if matched_text_marker {
                self.note_stripped_marker();
                if self.strip_thoughts {
                    self.in_thought = false;
                }
                input = candidate[marker_len..].to_string();
                continue;
            }
            if candidate.starts_with("<think>") || candidate.starts_with("<|think|>") {
                self.note_stripped_marker();
                if self.strip_thoughts {
                    self.in_thought = true;
                }
                let marker_len = if candidate.starts_with("<think>") {
                    7
                } else {
                    9
                };
                input = candidate[marker_len..].to_string();
                continue;
            }
            if candidate.starts_with("</think>") || candidate.starts_with("<|/think|>") {
                self.note_stripped_marker();
                if self.strip_thoughts {
                    self.in_thought = false;
                }
                let marker_len = if candidate.starts_with("</think>") {
                    8
                } else {
                    10
                };
                input = candidate[marker_len..].to_string();
                continue;
            }

            // "<|channel>" is itself a complete marker but also the opener of
            // "<|channel>text", "<|channel>thought", etc. If only the bare opener has
            // arrived, hold it and wait for the following channel-name piece, otherwise the
            // channel label leaks as visible text and the channel switch is missed.
            if candidate == "<|channel>" {
                self.pending = candidate.to_string();
                return output;
            }
            if let Some(marker) = TOKEN_MARKERS
                .iter()
                .find(|marker| candidate.starts_with(**marker))
            {
                self.note_stripped_marker();
                input = candidate[marker.len()..].to_string();
                continue;
            }
            if TOKEN_MARKERS
                .iter()
                .any(|marker| marker.starts_with(candidate))
            {
                self.pending = candidate.to_string();
                return output;
            }
            if !self.strip_thoughts || !self.in_thought {
                self.push_visible_text(&mut output, "<");
            }
            input = candidate[1..].to_string();
        }
        if !self.strip_thoughts || !self.in_thought {
            if self.strip_thoughts && !self.produced_visible && is_channel_label(&input) {
                // A lone leading channel-label word may be a stray header whose channel
                // marker arrives in the next piece. Hold it and decide on the next push.
                self.pending = input;
            } else {
                self.push_visible_text(&mut output, &input);
            }
        }
        output
    }

    fn finish(&mut self) -> String {
        let pending = std::mem::take(&mut self.pending);
        if TOKEN_MARKERS
            .iter()
            .any(|marker| marker.starts_with(&pending))
        {
            String::new()
        } else {
            if !self.strip_thoughts || !self.in_thought {
                let mut output = String::new();
                self.push_visible_text(&mut output, &pending);
                output
            } else {
                String::new()
            }
        }
    }
}

fn shift_session(
    context: &mut LlamaContext<'_>,
    session: &mut SessionCache,
    requested_drop: usize,
) -> Result<usize, NativeRuntimeError> {
    let removable = session.tokens.len().saturating_sub(session.pinned_tokens);
    let dropped = requested_drop.min(removable);
    if dropped == 0 {
        return Ok(0);
    }
    let start = session.pinned_tokens;
    let end = start + dropped;
    if session.resident {
        context
            .clear_kv_cache_seq(
                Some(session.sequence_id as u32),
                Some(start as u32),
                Some(end as u32),
            )
            .map_err(kv_error)?;
        context
            .kv_cache_seq_add(
                session.sequence_id,
                Some(end as u32),
                None,
                -(dropped as i32),
            )
            .map_err(kv_error)?;
    }
    session.tokens.drain(start..end);
    Ok(dropped)
}

fn clear_sequence(
    context: &mut LlamaContext<'_>,
    sequence_id: i32,
) -> Result<(), NativeRuntimeError> {
    context
        .clear_kv_cache_seq(Some(sequence_id as u32), None, None)
        .map(|_| ())
        .map_err(kv_error)
}

fn kv_error(error: impl std::fmt::Display) -> NativeRuntimeError {
    NativeRuntimeError {
        code: "llama_kv_cache_operation_failed",
        message: format!("llama.cpp KV-cache operation failed: {error}"),
    }
}

fn missing_session_error(session_id: &str) -> NativeRuntimeError {
    NativeRuntimeError {
        code: "llama_session_missing",
        message: format!("Native runtime session cache entry '{session_id}' was unavailable."),
    }
}

fn normalize_session_id(session_id: &str) -> String {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        format!("ephemeral-{}", unique_session_nonce())
    } else {
        trimmed.chars().take(128).collect()
    }
}

fn unique_session_nonce() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let counter = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
        ^ u128::from(counter)
}

fn reply_with_error(command: ModelCommand, error: NativeRuntimeError) {
    match command {
        #[cfg(test)]
        ModelCommand::AppendContext { response_tx, .. } => {
            let _ = response_tx.send(Err(error));
        }
        ModelCommand::Generate { response_tx, .. } => {
            let _ = response_tx.send(Err(error));
        }
        ModelCommand::EmbedText { response_tx, .. } => {
            let _ = response_tx.send(Err(error));
        }
        ModelCommand::FlushMemory { response_tx } => {
            let _ = response_tx.send(());
        }
        ModelCommand::Shutdown => {}
    }
}

#[derive(Debug)]
struct ReadyGguf {
    path: PathBuf,
    byte_count: u64,
    filesystem: String,
}

fn detect_hardware(backend: &LlamaBackend) -> Result<HardwareProfile, NativeRuntimeError> {
    let mut system = sysinfo::System::new_all();
    system.refresh_all();
    let logical_threads = thread::available_parallelism()
        .ok()
        .map(|value| value.get())
        .filter(|value| *value > 0)
        .or_else(|| (!system.cpus().is_empty()).then(|| system.cpus().len()))
        .ok_or_else(|| NativeRuntimeError {
            code: "llama_hardware_probe_unavailable",
            message: "Unable to observe a non-zero logical CPU count; native model runtime initialization was halted instead of fabricating a single-thread host."
                .to_string(),
        })?;
    let apple_silicon = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    let devices = list_llama_ggml_backend_devices();
    let metal_accelerator = metal_backend::preferred_metal_device(&devices);
    let accelerator = metal_accelerator.or_else(|| {
        devices.iter().find(|device| {
            matches!(
                device.device_type,
                LlamaBackendDeviceType::Gpu | LlamaBackendDeviceType::IntegratedGpu
            )
        })
    });
    let gpu_offload_available = backend.supports_gpu_offload() && accelerator.is_some();
    let metal_available =
        apple_silicon && backend.supports_gpu_offload() && metal_accelerator.is_some();

    Ok(HardwareProfile {
        operating_system: env::consts::OS.to_string(),
        architecture: env::consts::ARCH.to_string(),
        apple_silicon,
        metal_available,
        gpu_offload_available,
        mmap_available: backend.supports_mmap(),
        mlock_available: backend.supports_mlock(),
        accelerator_name: accelerator.map(|device| {
            if device.description.is_empty() {
                device.name.clone()
            } else {
                device.description.clone()
            }
        }),
        accelerator_memory_bytes: accelerator
            .map(|device| device.memory_total as u64)
            .unwrap_or_default(),
        logical_threads,
        total_memory_bytes: system.total_memory(),
    })
}

fn validate_gguf_readiness(path: &Path) -> Result<ReadyGguf, NativeRuntimeError> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
    {
        return Err(NativeRuntimeError {
            code: "llama_gguf_required",
            message: format!("{} is not a .gguf model.", path.display()),
        });
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if [".part", ".partial", ".download", ".tmp"]
        .iter()
        .any(|suffix| filename.ends_with(suffix))
    {
        return Err(NativeRuntimeError {
            code: "llama_model_write_incomplete",
            message: format!("{} is still marked as a partial download.", path.display()),
        });
    }

    let canonical = fs::canonicalize(path).map_err(|error| NativeRuntimeError {
        code: "llama_model_path_invalid",
        message: format!("Unable to resolve {}: {error}", path.display()),
    })?;
    let first_metadata = fs::metadata(&canonical).map_err(|error| NativeRuntimeError {
        code: "llama_model_metadata_failed",
        message: format!("Unable to inspect {}: {error}", canonical.display()),
    })?;
    if !first_metadata.is_file() || first_metadata.len() < MIN_GGUF_BYTES {
        return Err(NativeRuntimeError {
            code: "llama_model_write_incomplete",
            message: format!(
                "{} is not a complete regular GGUF file.",
                canonical.display()
            ),
        });
    }

    let mut file = File::open(&canonical).map_err(|error| NativeRuntimeError {
        code: "llama_model_open_failed",
        message: format!("Unable to open {}: {error}", canonical.display()),
    })?;
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic)
        .map_err(|error| NativeRuntimeError {
            code: "llama_model_header_failed",
            message: format!("Unable to read {}: {error}", canonical.display()),
        })?;
    if &magic != b"GGUF" {
        return Err(NativeRuntimeError {
            code: "llama_gguf_invalid",
            message: format!(
                "{} does not contain the GGUF magic header.",
                canonical.display()
            ),
        });
    }
    file.seek(SeekFrom::End(-1))
        .and_then(|_| {
            let mut tail = [0_u8; 1];
            file.read_exact(&mut tail)
        })
        .map_err(|error| NativeRuntimeError {
            code: "llama_model_write_incomplete",
            message: format!(
                "{} is not readable through its declared end: {error}",
                canonical.display()
            ),
        })?;

    thread::sleep(Duration::from_millis(COMPLETE_WRITE_PROBE_MS));
    let second_metadata = fs::metadata(&canonical).map_err(|error| NativeRuntimeError {
        code: "llama_model_metadata_failed",
        message: format!("Unable to re-inspect {}: {error}", canonical.display()),
    })?;
    if first_metadata.len() != second_metadata.len()
        || first_metadata.modified().ok() != second_metadata.modified().ok()
    {
        return Err(NativeRuntimeError {
            code: "llama_model_write_incomplete",
            message: format!(
                "{} changed while it was being validated; wait for the APFS write to finish.",
                canonical.display()
            ),
        });
    }

    let filesystem = filesystem_name(&canonical)?;
    #[cfg(target_os = "macos")]
    if !filesystem.eq_ignore_ascii_case("apfs")
        && env::var("OOMU_ALLOW_NON_APFS_MODELS").ok().as_deref() != Some("1")
    {
        return Err(NativeRuntimeError {
            code: "llama_model_apfs_required",
            message: format!(
                "{} is stored on {filesystem}; OOMU requires completed local GGUF assets on APFS.",
                canonical.display()
            ),
        });
    }

    Ok(ReadyGguf {
        path: canonical,
        byte_count: second_metadata.len(),
        filesystem,
    })
}

fn validate_loaded_model(
    model: &LlamaModel,
    ready: ReadyGguf,
    tensor_count: usize,
    hardware: &HardwareProfile,
    config: &RuntimeConfig,
) -> Result<NativeModelProfile, NativeRuntimeError> {
    let architecture = metadata(model, "general.architecture")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| NativeRuntimeError {
            code: "llama_model_architecture_missing",
            message: format!(
                "{} is valid GGUF but does not declare general.architecture.",
                ready.path.display()
            ),
        })?;
    let layer_count = model
        .n_layer()
        .max(metadata_u32(model, &format!("{architecture}.block_count")).unwrap_or_default());
    let embedding_length = u32::try_from(model.n_embd())
        .unwrap_or_default()
        .max(metadata_u32(model, &format!("{architecture}.embedding_length")).unwrap_or_default());
    if layer_count == 0 || embedding_length == 0 {
        return Err(NativeRuntimeError {
            code: "llama_model_architecture_invalid",
            message: format!(
                "{} declares architecture '{architecture}' without usable layers or embeddings.",
                ready.path.display()
            ),
        });
    }

    let per_layer_embedding_length = [
        format!("{architecture}.embedding_length_per_layer_input"),
        "gemma4.embedding_length_per_layer_input".to_string(),
        "general.embedding_length_per_layer_input".to_string(),
    ]
    .iter()
    .find_map(|key| metadata_u32(model, key));
    let requested_gpu_layers = if hardware.gpu_offload_available {
        config.requested_gpu_layers
    } else {
        0
    };
    let gpu_layers = requested_gpu_layers.min(layer_count);
    let gpu_offload_ratio = if layer_count == 0 {
        0.0
    } else {
        gpu_layers as f32 / layer_count as f32
    };

    Ok(NativeModelProfile {
        path: ready.path,
        architecture,
        name: metadata(model, "general.name").unwrap_or_else(|| "Local GGUF".to_string()),
        tensor_count,
        layer_count,
        embedding_length,
        per_layer_embedding_length,
        multi_layer_embeddings: per_layer_embedding_length.is_some(),
        parameter_count: model.n_params(),
        model_bytes: ready.byte_count,
        chat_template_present: metadata(model, "tokenizer.chat_template").is_some(),
        filesystem: ready.filesystem,
        device_label: if hardware.metal_available && gpu_layers > 0 {
            "llama.cpp Metal".to_string()
        } else {
            "llama.cpp CPU".to_string()
        },
        gpu_layers,
        gpu_offload_ratio,
        runtime_config: config.clone(),
    })
}

fn metadata(model: &LlamaModel, key: &str) -> Option<String> {
    model
        .meta_val_str(key)
        .ok()
        .map(|value| value.trim().trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
}

fn metadata_u32(model: &LlamaModel, key: &str) -> Option<u32> {
    metadata(model, key).and_then(|value| {
        value
            .trim_matches(|character: char| !character.is_ascii_digit())
            .parse()
            .ok()
    })
}

fn env_u32(name: &str) -> Option<u32> {
    env::var(name).ok()?.parse().ok()
}

fn env_u64(name: &str) -> Option<u64> {
    env::var(name).ok()?.parse().ok()
}

fn env_usize(name: &str) -> Option<usize> {
    env::var(name).ok()?.parse().ok()
}

fn env_i32(name: &str) -> Option<i32> {
    env::var(name).ok()?.parse().ok()
}

#[cfg(target_os = "macos")]
fn filesystem_name(path: &Path) -> Result<String, NativeRuntimeError> {
    use std::{
        ffi::{CStr, CString},
        os::unix::ffi::OsStrExt,
    };

    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| NativeRuntimeError {
        code: "llama_model_path_invalid",
        message: format!("{} contains an invalid null byte.", path.display()),
    })?;
    let mut stats = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(NativeRuntimeError {
            code: "llama_model_filesystem_failed",
            message: format!(
                "Unable to inspect the filesystem for the GGUF path: {}",
                std::io::Error::last_os_error()
            ),
        });
    }
    let stats = unsafe { stats.assume_init() };
    let name = unsafe { CStr::from_ptr(stats.f_fstypename.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    Ok(name)
}

#[cfg(not(target_os = "macos"))]
fn filesystem_name(_path: &Path) -> Result<String, NativeRuntimeError> {
    Ok("native".to_string())
}

#[cfg(test)]
mod tests;
