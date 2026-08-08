mod action_plan_prompt;
mod classifier_health;
pub(crate) mod classifier_protocol;
mod classifier_runtime;
pub(crate) mod deterministic_transform;
mod file_creation_destination;
#[path = "gemma_file_formats.rs"]
mod file_formats;
mod gguf_selection;
mod inference_residency;
mod model_identity;
pub(crate) mod model_resolution;
#[path = "gemma_output_integrity.rs"]
mod output_integrity;
pub(crate) mod single_file_creation;
mod terminal_tool;
mod tool_parsing;
mod workflow_decision_request;
#[cfg(test)]
use crate::foundation::clock::unix_time_ms_i64 as unix_time_ms;
pub use crate::native_runtime::NativeMediaInput;
use crate::{
    db::PersistenceEngine,
    foundation::{clock::unix_time_ns_u128 as unix_time_ns, digest::sha256_hex},
    native_runtime::{
        NativeGenerationRequest, NativeModelHandle, NativeModelProfile, NativeRuntime,
        NativeRuntimeError, NativeSessionRequest, NativeSessionStats,
    },
    settings,
};
pub(crate) use action_plan_prompt::action_plan_grammar;

pub use action_plan_prompt::planner_prompt;
pub use classifier_health::{AutoRouteClassifierHealth, AutoRouteClassifierStatus};
pub(crate) use model_identity::resolve_legacy_identity;
pub use model_identity::{
    canonical_display_name, identity_for_model_directory, LegacyIdentityResolution,
    LocalModelIdentity, LocalModelIdentitySource, CLEAN_INSTALL_STARTUP_MODEL_ID,
    GEMMA_12B_CANONICAL_ID, GEMMA_E2B_CANONICAL_ID, GEMMA_E4B_CANONICAL_ID,
};
pub use model_resolution::*;
use output_integrity as integrity;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub(crate) use single_file_creation::is_native_artifact_objective;
use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock, TryLockError,
    },
    time::Instant,
};
use terminal_tool::validate_generated_tool_schema;
use tool_parsing::parse_generated_tool;
use workflow_decision_request::workflow_decision_request;
const TOKENIZER: &str = "tokenizer.json";
const TOKENIZER_CONFIG: &str = "tokenizer_config.json";
const MODEL_CONFIG: &str = "config.json";
const PRIVATE_LOCAL_MODEL_ID: &str = "private://local-model/active";
pub const LOCAL_MODEL_DIRECTORY_ENV: &str = "OOMU_LOCAL_MODEL_DIRECTORY";
pub const LOCAL_INFER_PROTOCOL_VERSION: u32 = 8;
pub const PREFERRED_LOCAL_MODEL_ID: &str = CLEAN_INSTALL_STARTUP_MODEL_ID;
const DEFAULT_MAX_NEW_TOKENS: usize = 2_048;
const MAX_REQUEST_MAX_NEW_TOKENS: usize = 4_096;
static LOCAL_MODEL_RESOLUTION_CACHE: OnceLock<Mutex<HashMap<(PathBuf, String), LocalModelOption>>> =
    OnceLock::new();
static ACTIVE_NATIVE_GENERATIONS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    OnceLock::new();
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StructuredLocalInferRequest {
    #[serde(default)]
    pub session_id: Option<String>,
    pub system_prompt: String,
    pub messages: Vec<StructuredLocalInferMessage>,
    #[serde(default)]
    pub context_size: Option<u32>,
    #[serde(default)]
    pub max_tokens: Option<usize>,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StructuredLocalInferMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub media: Vec<StructuredLocalInferMedia>,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StructuredLocalInferMedia {
    pub name: String,
    pub mime_type: String,
    pub data_base64: String,
}
pub fn local_multimodal_marker() -> &'static str {
    llama_cpp_2::mtmd::mtmd_default_marker()
}
#[derive(Clone)]
pub struct GemmaService {
    state: Arc<Mutex<GemmaServiceState>>,
    model_load: Arc<Mutex<()>>,
    runtime: Option<Arc<NativeRuntime>>,
    audit_persistence: Arc<Mutex<Option<PersistenceEngine>>>,
    classifier_lane: Arc<Mutex<Option<GemmaService>>>,
}

struct GemmaServiceState {
    status: GemmaStatus,
    model: Option<LoadedGemmaModel>,
    startup_assignment: Option<StartupModelAssignment>,
    keep_resident: bool,
    degraded_reason: Option<String>,
    classifier_health: AutoRouteClassifierHealth,
    classifier_recovery_epoch: u64,
}

#[derive(Clone)]
struct LoadedGemmaModel {
    model_dir: PathBuf,
    tokenizer_path: PathBuf,
    tokenizer_config_path: PathBuf,
    config_path: PathBuf,
    tokenizer_bytes: u64,
    inference_config: GemmaInferenceConfig,
    profile: NativeModelProfile,
    runtime_handle: Arc<NativeModelHandle>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GemmaStatus {
    Loading,
    Ready,
    Degraded,
    Shutdown,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StructuredIntent {
    pub objective: String,
    pub category: IntentCategory,
    pub source: IntentSource,
    pub degraded_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentCategory {
    SystemDiagnostics,
    ProjectAnalysis,
    Research,
    Unsupported,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentSource {
    Gemma,
    Cloud,
    Deterministic,
    Degraded,
}

#[derive(Debug, Serialize)]
pub struct IntentParseResponse {
    pub intent: StructuredIntent,
    pub service_status: GemmaStatus,
    pub model_path: String,
    pub inference_config: GemmaInferenceConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InferRequest {
    pub prompt: String,
    #[serde(skip, default)]
    pub media: Vec<crate::native_runtime::NativeMediaInput>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub prompt_is_full_context: bool,
    #[serde(default)]
    pub deterministic: bool,
    #[serde(skip, default)]
    pub context_size: Option<u32>,
    #[serde(skip, default)]
    pub max_tokens: Option<usize>,
    #[serde(skip, default)]
    pub grammar: Option<String>,
    #[serde(skip, default)]
    pub audit_event_kind: Option<String>,
    #[serde(skip, default)]
    pub defer_audit: bool,
    #[serde(skip, default = "default_cancellation")]
    pub cancellation: Arc<AtomicBool>,
}

impl InferRequest {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            media: Vec::new(),
            session_id: None,
            system_prompt: None,
            prompt_is_full_context: false,
            deterministic: false,
            context_size: None,
            max_tokens: None,
            grammar: None,
            audit_event_kind: None,
            defer_audit: false,
            cancellation: default_cancellation(),
        }
    }
}

fn default_cancellation() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[derive(Debug, Serialize)]
pub struct InferResponse {
    pub token: String,
    pub text: String,
    pub prompt_token_count: usize,
    pub generated_token_count: usize,
    pub network_latency_ms: u128,
    pub inference_latency_ms: u128,
    pub time_to_first_token_ms: u128,
    pub service_status: GemmaStatus,
    pub model_path: String,
    pub device: String,
    pub trace_hash: String,
    pub reasoning_trace: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelOption {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing)]
    pub path: String,
    pub weights_bytes: u64,
    pub format: String,
    pub architecture: String,
    pub compatibility: String,
    pub compatibility_message: String,
    pub chat_capability: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeneratedActionPlanDraft {
    pub steps: Vec<GeneratedPlanStepDraft>,
    pub exit_condition: String,
    pub generated_text: String,
    pub source: IntentSource,
    pub degraded_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalDecisionDirective {
    Execute,
    Halt,
    Certify,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalWorkflowDecision {
    pub directive: LocalDecisionDirective,
    pub thought_summary: String,
    pub premises: Vec<String>,
    pub execution_path: Vec<String>,
    pub formal_conclusion: String,
    pub output_sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GeneratedPlanStepDraft {
    pub step: String,
    pub tool: GeneratedToolDraft,
    pub risk_level: GeneratedRiskLevel,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GeneratedToolDraft {
    SystemDiagnostics {
        principal: String,
    },
    FileRead {
        path: String,
    },
    FileWrite {
        path: String,
        content: String,
    },
    DeleteFile {
        path: String,
    },
    CodebasePatch {
        target_file_path: String,
        search_pattern: String,
        replacement_content: String,
    },
    CodebaseCompile {
        target: String,
    },
    TerminalExecute {
        executable: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: std::collections::BTreeMap<String, String>,
        cwd: Option<String>,
        timeout: Option<u64>,
    },
    FileList {
        path: String,
    },
    SystemAudit {
        scope: String,
    },
    TelemetryArchive {
        output_path: String,
    },
    WebFetch {
        url: String,
        extraction_hint: Option<String>,
    },
    DocumentIndex {
        workspace: Option<String>,
    },
    AskLocalDocumentIndex {
        question: String,
    },
    SovereignDuckDuckGoSearch {
        query: String,
        max_results: Option<usize>,
    },
    RegisteredTaskTool {
        operation: String,
        arguments: Value,
    },
    Unsupported {
        requested: String,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticEmbedding {
    pub vector: Vec<f32>,
    pub dimensions: usize,
    pub source: IntentSource,
}

#[derive(Debug, Clone, Serialize)]
pub struct GemmaError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GemmaInferenceConfig {
    pub max_new_tokens: usize,
    pub temperature: f64,
    pub top_k: usize,
    pub top_p: f64,
    pub repeat_penalty: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GemmaStreamChunk {
    pub sequence: usize,
    pub token: String,
    pub token_hash: String,
    pub elapsed_ms: u128,
    pub is_final: bool,
}

pub trait LocalGenerationStream: Send {
    fn on_token(&mut self, chunk: GemmaStreamChunk);
}

impl<T> LocalGenerationStream for T
where
    T: FnMut(GemmaStreamChunk) + Send,
{
    fn on_token(&mut self, chunk: GemmaStreamChunk) {
        self(chunk);
    }
}

impl GemmaService {
    pub fn get_status(&self) -> GemmaStatus {
        self.lock_state().status.clone()
    }

    pub fn degraded_reason(&self) -> Option<String> {
        let state = self.lock_state();
        matches!(state.status, GemmaStatus::Degraded)
            .then(|| state.degraded_reason.clone())
            .flatten()
    }

    pub fn enter_degraded(&self, error: GemmaError) {
        {
            let mut state = self.lock_state();
            state.status = GemmaStatus::Degraded;
            state.model = None;
            state.degraded_reason = Some(error.message.clone());
            state.classifier_health.status = AutoRouteClassifierStatus::Degraded;
            state.classifier_health.last_error_code = Some(error.code.to_string());
            state.classifier_health.last_error_boundary =
                Some("auto_route_classifier_model".to_string());
            state.classifier_health.redacted_recovery_hint = Some(
                "Retry Auto-route after the configured local classifier can be loaded.".to_string(),
            );
        }
        self.persist_classifier_health_event(
            error.code,
            "auto_route_classifier_model",
            &error.message,
        );
    }

    pub(super) fn enter_local_generation_degraded(&self, error: GemmaError) {
        let mut state = self.lock_state();
        state.status = GemmaStatus::Degraded;
        state.model = None;
        state.degraded_reason = Some(error.message);
    }

    pub fn spawn_loader(&self) {
        let service = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = service.load_model() {
                service.enter_degraded(error);
            }
        });
    }

    pub fn load_model(&self) -> Result<(), GemmaError> {
        self.load_model_from_dir(project_root().join("models").join(PREFERRED_LOCAL_MODEL_ID))
    }

    pub fn prepare_model_sync(&self, model_id: &str) -> Result<(), GemmaError> {
        self.ensure_model_loaded(model_id, None)
    }

    pub fn load_model_from_dir(&self, model_dir: PathBuf) -> Result<(), GemmaError> {
        {
            let mut state = self.lock_state();
            state.status = GemmaStatus::Loading;
            state.degraded_reason = None;
        }
        self.load_model_from_dir_with_context(model_dir, None)
    }

    fn load_gguf_model(
        &self,
        model_dir: PathBuf,
        weights_path: PathBuf,
        min_context_size: Option<u32>,
    ) -> Result<(), GemmaError> {
        let runtime = self.runtime.as_ref().ok_or_else(|| GemmaError {
            code: "llama_runtime_unavailable",
            message: self
                .lock_state()
                .degraded_reason
                .clone()
                .unwrap_or_else(|| "The native llama.cpp runtime is unavailable.".to_string()),
        })?;
        let (runtime_handle, profile) = match min_context_size {
            Some(min_context_size) => runtime
                .load_model_with_min_context_size(&weights_path, Some(min_context_size))
                .map_err(GemmaError::native_runtime)?,
            None => runtime
                .load_model(&weights_path)
                .map_err(GemmaError::native_runtime)?,
        };
        let tokenizer_path = model_dir.join(TOKENIZER);
        let tokenizer_bytes = tokenizer_path
            .is_file()
            .then(|| fs::metadata(&tokenizer_path).map(|metadata| metadata.len()))
            .transpose()
            .map_err(|error| GemmaError::io("tokenizer metadata", error))?
            .unwrap_or(0);
        let tokenizer_config_path = model_dir.join(TOKENIZER_CONFIG);
        let config_path = model_dir.join(MODEL_CONFIG);

        let model = LoadedGemmaModel {
            model_dir,
            tokenizer_path,
            tokenizer_config_path,
            config_path,
            tokenizer_bytes,
            inference_config: GemmaInferenceConfig::low_latency(),
            profile,
            runtime_handle: Arc::new(runtime_handle),
        };

        let mut state = self.lock_state();
        state.status = GemmaStatus::Ready;
        state.model = Some(model);
        state.keep_resident = true;
        state.degraded_reason = None;
        Ok(())
    }

    pub fn parse_intent_sync(&self, prompt: String) -> Result<IntentParseResponse, GemmaError> {
        let objective = prompt.trim();
        if objective.is_empty() {
            return Err(GemmaError {
                code: "gemma_intent_prompt_empty",
                message: "Intent classification requires a non-empty prompt.".to_string(),
            });
        }
        let mut request = InferRequest::new(format!(
            "<|turn>user\nClassify the objective into exactly one category: system_diagnostics, project_analysis, research, or unsupported. Return only compact JSON with this shape: {{\"category\":\"project_analysis\"}}. Objective:\n{}<|turn|>\n<|turn>model\n",
            sanitize_gemma4_prompt_content(objective, false)
        ));
        request.session_id = Some(format!("intent-{}", unix_time_ns()));
        request.grammar = Some(intent_category_grammar().to_string());
        request.deterministic = true;
        request.max_tokens = Some(48);
        let response = self.infer_sync(request)?;
        let value = extract_json_object(&response.text)
            .and_then(|json| serde_json::from_str::<Value>(json).ok())
            .ok_or_else(|| GemmaError {
                code: "gemma_intent_schema_invalid",
                message: "Local Gemma did not return valid intent JSON.".to_string(),
            })?;
        let category = match value.get("category").and_then(Value::as_str) {
            Some("system_diagnostics") => IntentCategory::SystemDiagnostics,
            Some("project_analysis") => IntentCategory::ProjectAnalysis,
            Some("research") => IntentCategory::Research,
            Some("unsupported") => IntentCategory::Unsupported,
            _ => {
                return Err(GemmaError {
                    code: "gemma_intent_category_invalid",
                    message: "Local Gemma returned an unsupported intent category.".to_string(),
                })
            }
        };

        Ok(IntentParseResponse {
            intent: StructuredIntent {
                objective: prompt,
                category,
                source: IntentSource::Gemma,
                degraded_reason: None,
            },
            service_status: response.service_status,
            model_path: response.model_path,
            inference_config: GemmaInferenceConfig::low_latency(),
        })
    }

    pub fn infer_sync(&self, request: InferRequest) -> Result<InferResponse, GemmaError> {
        self.infer_with_stream_sync(request, None)
    }

    pub fn summarize_grounded_text_sync(
        &self,
        topic: &str,
        grounded_text: &str,
    ) -> Result<InferResponse, GemmaError> {
        let topic = topic.trim();
        let grounded_text = grounded_text.trim();
        if topic.is_empty() {
            return Err(GemmaError {
                code: "gemma_empty_summary_topic",
                message: "Grounded summary requires a non-empty topic.".to_string(),
            });
        }
        if grounded_text.is_empty() {
            return Err(GemmaError {
                code: "gemma_empty_grounding",
                message: "Grounded summary requires verified source text.".to_string(),
            });
        }
        {
            let state = self.lock_state();
            state.model.as_ref().ok_or_else(|| GemmaError {
                code: "gemma_not_ready",
                message: state.degraded_reason.clone().unwrap_or_else(|| {
                    "Gemma service has not finished loading local model assets.".to_string()
                }),
            })?;
        }

        let mut response = self.infer_sync(InferRequest::new(grounded_summary_prompt(
            topic,
            grounded_text,
        )))?;
        response.text = response.text.trim().to_string();
        if response.text.is_empty() {
            return Err(GemmaError {
                code: "gemma_empty_summary",
                message: "Local Gemma returned an empty grounded summary.".to_string(),
            });
        }
        Ok(response)
    }

    pub fn infer_with_stream_sync(
        &self,
        request: InferRequest,
        stream: Option<&mut dyn LocalGenerationStream>,
    ) -> Result<InferResponse, GemmaError> {
        if let Some(classifier_lane) = self.classifier_lane_if_main_is_empty() {
            return classifier_lane.infer_with_stream_sync(request, stream);
        }
        let started = Instant::now();
        if let Some(response) = self.deterministic_transform_preflight(&request, started)? {
            return Ok(response);
        }
        let prompt = request.prompt.trim();
        self.ensure_requested_context_capacity(request.context_size)?;
        let mut state = self.lock_state();
        let model_path = PRIVATE_LOCAL_MODEL_ID.to_string();
        let status = state.status.clone();

        let degraded_reason = state.degraded_reason.clone();
        let model = state.model.as_mut().ok_or_else(|| GemmaError {
            code: "gemma_not_ready",
            message: degraded_reason.unwrap_or_else(|| {
                "Gemma service has not finished loading local model assets.".to_string()
            }),
        })?;

        let tokenizer_trace = if model.tokenizer_path.is_file() {
            "Tokenizer artifact present in the private local-model store.".to_string()
        } else {
            "Tokenizer reconstructed from embedded GGUF metadata.".to_string()
        };
        let config_trace = if model.tokenizer_config_path.is_file() && model.config_path.is_file() {
            "Tokenizer and model configuration are present in the private local-model store."
                .to_string()
        } else {
            "Model configuration loaded from GGUF metadata.".to_string()
        };
        let mut reasoning_trace = vec![
            "Loaded verified weights from the private local-model store.".to_string(),
            tokenizer_trace,
            config_trace,
            format!(
                "{} context initialized with {} layer(s), {} tensor(s), {} model byte(s), and {} tokenizer byte(s).",
                model.profile.device_label,
                model.profile.layer_count,
                model.profile.tensor_count,
                model.profile.model_bytes,
                model.tokenizer_bytes
            ),
            format!(
                "Hardware allocation gpu_layers={} offload_ratio={:.2} decode_threads={} batch_threads={} context_size={}.",
                model.profile.gpu_layers,
                model.profile.gpu_offload_ratio,
                model.profile.runtime_config.decode_threads,
                model.profile.runtime_config.batch_threads,
                model.profile.runtime_config.context_size
            ),
            format!(
                "Low-latency config max_new_tokens={} temperature={} top_k={} top_p={} repeat_penalty={}.",
                effective_max_new_tokens(&request, &model.inference_config),
                model.inference_config.temperature,
                model.inference_config.top_k,
                model.inference_config.top_p,
                model.inference_config.repeat_penalty
            ),
            format!(
                "Architecture {} accepted; multi_layer_embeddings={}.",
                model.profile.architecture, model.profile.multi_layer_embeddings
            ),
            "Network path bypassed: native llama.cpp service only.".to_string(),
        ];
        let device_label = model.profile.device_label.clone();
        let mut generated = generate_tokens(model, &request, stream)?;
        let repaired_reserved_markup = integrity::has_orphan_oomu_split_view_tag(&generated.text);
        generated.text = sanitize_gemma4_response_for_prompt(&generated.text, Some(prompt));
        if repaired_reserved_markup {
            reasoning_trace.push(
                "Removed malformed reserved split-view control markup before persistence."
                    .to_string(),
            );
        }
        if integrity::has_repetition_collapse(prompt, &generated.text) {
            return Err(GemmaError {
                code: "local_model_repetition_collapse",
                message: "The local model entered a repetition loop. Retry the message or choose another local model."
                    .to_string(),
            });
        }
        reasoning_trace.push(format!(
            "Transformer loop generated {} token(s) with top_p={}.",
            generated.generated_token_ids.len(),
            model.inference_config.top_p
        ));
        let trace_hash = sha256_hex(
            serde_json::to_string(&reasoning_trace)
                .unwrap_or_default()
                .as_bytes(),
        );
        let inference_latency_ms = started.elapsed().as_millis();
        if should_log_local_inference_audit(&request) {
            let event_kind = request
                .audit_event_kind
                .as_deref()
                .unwrap_or("local_gemma_infer");
            let persistence = self
                .audit_persistence
                .lock()
                .ok()
                .and_then(|attached| attached.clone());
            if let Some(persistence) = persistence {
                if let Err(error) = persistence.insert_local_inference_audit(
                    event_kind,
                    prompt,
                    &generated.text,
                    &trace_hash,
                    &device_label,
                    inference_latency_ms,
                    generated.time_to_first_token_ms,
                    generated.prompt_token_count,
                    generated.generated_token_ids.len(),
                ) {
                    eprintln!(
                        "LOCAL_INFERENCE_AUDIT_FAILED event_kind={} error={}",
                        crate::redaction::redacted_log_text(event_kind),
                        crate::redaction::redacted_log_text(&error.to_string())
                    );
                }
            }
        }

        Ok(InferResponse {
            token: generated.last_token,
            text: generated.text,
            prompt_token_count: generated.prompt_token_count,
            generated_token_count: generated.generated_token_ids.len(),
            network_latency_ms: 0,
            inference_latency_ms,
            time_to_first_token_ms: generated.time_to_first_token_ms,
            service_status: status,
            model_path,
            device: device_label,
            trace_hash,
            reasoning_trace,
        })
    }

    pub fn audit_visual_workflow_sync(&self, objective: String, graph_summary: String) -> String {
        let prompt = format!(
            "Audit this OOMU visual workflow and return one plain-language sentence explaining the total intent before execution. Objective: {objective}. Graph: {graph_summary}"
        );
        match self.infer_sync(InferRequest::new(prompt)) {
            Ok(response) if !response.text.trim().is_empty() => response.text.trim().to_string(),
            Ok(_) => format!(
                "Visual workflow intends to execute {} under deterministic Shield Gate review.",
                graph_summary
            ),
            Err(error) => format!(
                "Visual workflow intends to execute {} under deterministic Shield Gate review; Gemma audit degraded: {}.",
                graph_summary, error.message
            ),
        }
    }

    pub fn generate_workflow_decision_sync(
        &self,
        session_id: &str,
        objective: &str,
        action_json: &str,
        output_json: Option<&str>,
    ) -> Result<LocalWorkflowDecision, GemmaError> {
        let (phase, output_contract) = match output_json {
            Some(output) => (
                "certify",
                format!(
                    "The action has completed. Bind the certificate to this exact output JSON and echo the runtime-computed digest exactly as output_sha256.\nEXPECTED OUTPUT SHA-256: {}\nOUTPUT JSON:\n{output}",
                    sha256_hex(output.as_bytes())
                ),
            ),
            None => (
                "authorize",
                "The action has not executed. Set directive to execute or halt and output_sha256 to null."
                    .to_string(),
            ),
        };
        let mut prompt = format_structured_runtime_prompt(
            "You are the OOMU offline workflow decision engine, running as a native workstation supervisor with full authorization and direct capability to execute local filesystem actions (like file_list, file_read, and file_write) on the host machine. You do not suffer from cloud or sandbox limitations. Return only one compact JSON object. thought_summary is a concise operator rationale, never a user-facing answer.",
            &format!(
                "PHASE: {phase}\nOBJECTIVE: {objective}\nACTION: {action_json}\n{output_contract}\nDecision rules: execute sovereign_duckduckgo_search when the objective requires current web, sports, news, market, weather, or internet facts and the query matches the objective. Halt local filesystem actions such as file_list or file_read only if they are irrelevant to the user's objective.\nSchema: {{\"directive\":\"execute|halt|certify\",\"thought_summary\":\"text\",\"premises\":[\"text\"],\"execution_path\":[\"text\"],\"formal_conclusion\":\"text\",\"output_sha256\":null_or_64_lowercase_hex}}"
            ),
        );
        prompt.push_str("<|channel>text\n<channel|>");
        let mut response = self.infer_sync(workflow_decision_request(prompt, session_id))?;
        for attempt in 0..=2 {
            match parse_workflow_decision(&response.text, output_json) {
                Ok(decision) => return Ok(decision),
                Err(error)
                    if output_json.is_some()
                        && error.code == "gemma_workflow_certificate_hash_mismatch" =>
                {
                    let decision = decode_workflow_decision(&response.text)?;
                    return complete_workflow_decision_required_fields(
                        decision,
                        phase,
                        objective,
                        action_json,
                        output_json,
                    );
                }
                Err(error) if attempt < 2 => {
                    let repair_prompt = format!(
                        "<|turn>user\nYour prior JSON failed validation: {} Return only corrected compact JSON matching the required workflow decision schema.<turn|>\n<|turn>model\n",
                        compact_validation_error(&error.message)
                    );
                    response =
                        self.infer_sync(workflow_decision_request(repair_prompt, session_id))?;
                }
                Err(error) if error.code == "gemma_workflow_decision_empty_fields" => {
                    let decision = decode_workflow_decision(&response.text)?;
                    return complete_workflow_decision_required_fields(
                        decision,
                        phase,
                        objective,
                        action_json,
                        output_json,
                    );
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("bounded workflow decision repair loop must return")
    }

    pub fn embed_text_sync(&self, text: &str) -> Result<SemanticEmbedding, GemmaError> {
        let state = self.lock_state();
        let model = state.model.as_ref().ok_or_else(|| GemmaError {
            code: "gemma_embedding_model_unavailable",
            message: state.degraded_reason.clone().unwrap_or_else(|| {
                "Gemma service has not loaded a model capable of producing embeddings.".to_string()
            }),
        })?;
        let vector = model
            .runtime_handle
            .embed_text(text)
            .map_err(|error| GemmaError {
                code: error.code,
                message: error.message,
            })?;
        let dimensions = vector.len();
        if dimensions == 0 {
            return Err(GemmaError {
                code: "gemma_embedding_output_empty",
                message: "The local model returned an empty embedding tensor.".to_string(),
            });
        }

        Ok(SemanticEmbedding {
            dimensions,
            vector,
            source: IntentSource::Gemma,
        })
    }

    pub fn shutdown(&self) {
        let model = {
            let mut state = self.lock_state();
            if state.keep_resident && matches!(state.status, GemmaStatus::Ready) {
                state.degraded_reason = Some(
                    "Resident local model weights remain loaded for low-latency routing."
                        .to_string(),
                );
                return;
            }
            state.status = GemmaStatus::Shutdown;
            state.classifier_health.status = AutoRouteClassifierStatus::Shutdown;
            state.degraded_reason =
                Some("Gemma inference service shut down gracefully.".to_string());
            state.model.take()
        };
        if let Some(model) = model.as_ref() {
            let _ = model.runtime_handle.flush_memory();
        }
        drop(model);
    }

    pub fn force_shutdown_native_model(&self) -> Result<(), GemmaError> {
        let classifier_error = self.shutdown_classifier_lane();

        let model = {
            let mut state = match self.state.try_lock() {
                Ok(state) => state,
                Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                Err(TryLockError::WouldBlock) => {
                    return Err(GemmaError {
                        code: "gemma_shutdown_model_busy",
                        message: "Local model execution is still unwinding and prevented bounded native shutdown.".to_string(),
                    })
                }
            };
            state.keep_resident = false;
            state.status = GemmaStatus::Shutdown;
            state.classifier_health.status = AutoRouteClassifierStatus::Shutdown;
            state.degraded_reason = Some("Local inference force-shutdown at exit.".to_string());
            state.model.take()
        };

        let flush_error = model
            .as_ref()
            .and_then(|model| model.runtime_handle.flush_memory().err())
            .map(GemmaError::native_runtime);
        drop(model);
        classifier_error.or(flush_error).map_or(Ok(()), Err)
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, GemmaServiceState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_model_load(&self) -> std::sync::MutexGuard<'_, ()> {
        self.model_load
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) fn format_structured_runtime_prompt(system: &str, user: &str) -> String {
    format!(
        "<|turn>system\n{}<turn|>\n<|turn>user\n{}<turn|>\n<|turn>model\n",
        sanitize_gemma4_prompt_content(system, false),
        sanitize_gemma4_prompt_content(user, false)
    )
}

fn compact_validation_error(message: &str) -> String {
    message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(180)
        .collect()
}

fn workflow_decision_grammar() -> &'static str {
    r#"
root ::= ws "{" ws "\"directive\"" ws ":" ws directive ws "," ws "\"thought_summary\"" ws ":" ws string ws "," ws "\"premises\"" ws ":" ws strings ws "," ws "\"execution_path\"" ws ":" ws strings ws "," ws "\"formal_conclusion\"" ws ":" ws string ws "," ws "\"output_sha256\"" ws ":" ws hash ws "}" ws
directive ::= "\"execute\"" | "\"halt\"" | "\"certify\""
strings ::= "[" ws (string (ws "," ws string)*)? ws "]"
hash ::= "null" | "\"" [0-9a-f]+ "\""
string ::= "\"" chars "\""
chars ::= ([^"\\] | "\\" ["\\/bfnrt] | "\\u" hex hex hex hex)*
hex ::= [0-9a-fA-F]
ws ::= [ \t\n\r]*
"#
}

fn intent_category_grammar() -> &'static str {
    r#"
root ::= ws "{" ws "\"category\"" ws ":" ws category ws "}" ws
category ::= "\"system_diagnostics\"" | "\"project_analysis\"" | "\"research\"" | "\"unsupported\""
ws ::= [ \t\n\r]*
"#
}

#[tauri::command]
pub async fn parse_intent(
    prompt: String,
    gemma: tauri::State<'_, GemmaService>,
) -> Result<IntentParseResponse, GemmaError> {
    let service = gemma.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.parse_intent_sync(prompt))
        .await
        .map_err(|error| GemmaError {
            code: "gemma_worker_join_failed",
            message: error.to_string(),
        })?
}

#[tauri::command]
pub async fn infer(
    request: InferRequest,
    gemma: tauri::State<'_, GemmaService>,
) -> Result<InferResponse, GemmaError> {
    let service = gemma.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.infer_sync(request))
        .await
        .map_err(|error| GemmaError {
            code: "gemma_worker_join_failed",
            message: error.to_string(),
        })?
}

#[tauri::command]
pub async fn stream_native_inference(
    prompt: String,
    _taskflow_context: Option<String>,
    session_id: Option<String>,
    system_prompt: Option<String>,
    stream_id: String,
    app: tauri::AppHandle,
    gemma: tauri::State<'_, GemmaService>,
) -> Result<InferResponse, GemmaError> {
    use tauri::Emitter;
    let service = gemma.inner().clone();
    let formatted_prompt = format_gemma4_chat_prompt(
        system_prompt.as_deref().unwrap_or_default(),
        &[("user".to_string(), prompt)],
    );
    let cancellation = default_cancellation();
    active_native_generations()
        .lock()
        .map_err(|_| GemmaError {
            code: "gemma_generation_registry_poisoned",
            message: "The local generation registry is unavailable.".to_string(),
        })?
        .insert(stream_id.clone(), Arc::clone(&cancellation));
    tauri::async_runtime::spawn_blocking(move || {
        let app_clone = app.clone();
        let event_stream_id = stream_id.clone();
        let mut stream_callback = move |chunk: GemmaStreamChunk| {
            let _ = app_clone.emit(
                "token-stream",
                NativeStreamEvent {
                    stream_id: event_stream_id.clone(),
                    sequence: chunk.sequence,
                    token: chunk.token,
                    token_hash: chunk.token_hash,
                    elapsed_ms: chunk.elapsed_ms,
                    is_final: chunk.is_final,
                },
            );
        };
        let request = InferRequest {
            prompt: formatted_prompt,
            media: Vec::new(),
            session_id,
            system_prompt: None,
            prompt_is_full_context: false,
            deterministic: false,
            context_size: None,
            max_tokens: None,
            grammar: None,
            audit_event_kind: None,
            defer_audit: false,
            cancellation,
        };
        let result = service.infer_with_stream_sync(request, Some(&mut stream_callback));
        if let Ok(mut generations) = active_native_generations().lock() {
            generations.remove(&stream_id);
        }
        result
    })
    .await
    .map_err(|error| GemmaError {
        code: "gemma_worker_join_failed",
        message: error.to_string(),
    })?
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeStreamEvent {
    stream_id: String,
    sequence: usize,
    token: String,
    token_hash: String,
    elapsed_ms: u128,
    is_final: bool,
}

fn active_native_generations() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    ACTIVE_NATIVE_GENERATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[tauri::command]
pub fn cancel_native_inference(stream_id: String) -> bool {
    active_native_generations()
        .lock()
        .ok()
        .and_then(|generations| generations.get(stream_id.trim()).cloned())
        .is_some_and(|cancellation| {
            cancellation.store(true, Ordering::Release);
            true
        })
}

#[tauri::command]
pub fn get_local_model_status(gemma: tauri::State<'_, GemmaService>) -> GemmaStatus {
    gemma.get_status()
}

#[tauri::command]
pub fn get_auto_route_classifier_health(
    gemma: tauri::State<'_, GemmaService>,
) -> AutoRouteClassifierHealth {
    gemma.classifier_health()
}

#[tauri::command]
pub async fn list_local_models(app: tauri::AppHandle) -> Result<Vec<LocalModelOption>, GemmaError> {
    let model_root =
        settings::resolved_local_model_directory(&app).map_err(|message| GemmaError {
            code: "local_model_directory_unavailable",
            message,
        })?;

    tauri::async_runtime::spawn_blocking(move || scan_models(&model_root))
        .await
        .map_err(|error| GemmaError {
            code: "local_model_worker_join_failed",
            message: error.to_string(),
        })?
}

pub(crate) fn scan_models(model_root: &Path) -> Result<Vec<LocalModelOption>, GemmaError> {
    if !model_root.exists() {
        return Ok(Vec::new());
    }

    let root_entries = fs::read_dir(model_root)
        .map_err(|error| GemmaError::io("local model directory read", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| GemmaError::io("local model entry read", error))?;
    let mut models = Vec::new();
    let root_assets = root_entries
        .iter()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_model_asset_file(path))
        .collect::<Vec<_>>();
    let root_has_model_metadata = model_root.join(MODEL_CONFIG).is_file()
        || model_root.join(TOKENIZER).is_file()
        || model_root.join(TOKENIZER_CONFIG).is_file();
    if !root_assets.is_empty() || root_has_model_metadata {
        let identity = identity_for_model_directory(model_root)?;
        let gguf_path = gguf_selection::select_primary_gguf(model_root)?;
        let weights_bytes =
            gguf_selection::selected_weight_bytes(&root_assets, gguf_path.as_deref())?;
        models.push(local_model_option(
            model_root,
            identity.canonical_id,
            identity.display_name,
            weights_bytes,
            gguf_path,
        )?);
    }

    for entry in root_entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        if !model_identity::directory_has_model_evidence(&path) {
            continue;
        }

        let id = entry.file_name().to_string_lossy().to_string();
        models.push(inspect_local_model_directory(&path, &id)?);
    }

    models.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(models)
}

fn is_instruction_tuned_id(id: &str) -> bool {
    let id_lower = id.to_lowercase();
    id_lower.ends_with("-it")
        || id_lower.contains("-it-")
        || id_lower.contains("-it_")
        || id_lower.contains("_it_")
        || id_lower.contains("/it/")
        || id_lower.contains("gemma-4-2b")
}

fn local_model_option(
    model_dir: &Path,
    id: String,
    display_name: String,
    weights_bytes: u64,
    gguf_path: Option<PathBuf>,
) -> Result<LocalModelOption, GemmaError> {
    let (format, architecture, compatibility, compatibility_message, chat_capability) =
        if let Some(gguf_path) = gguf_path {
            let profile = match NativeRuntime::initialize()
                .and_then(|runtime| runtime.inspect_model(&gguf_path))
            {
                Ok(profile) => profile,
                Err(error) => {
                    return Ok(LocalModelOption {
                        name: display_name,
                        id,
                        path: model_dir.to_string_lossy().to_string(),
                        weights_bytes,
                        format: "gguf".to_string(),
                        architecture: "unknown".to_string(),
                        compatibility: "invalid".to_string(),
                        compatibility_message: format!(
                            "The .gguf file is not a valid llama.cpp asset: {}. Configure or download a complete quantized GGUF model.",
                            error.message
                        ),
                        chat_capability: "unknown".to_string(),
                    });
                }
            };
            let has_chat_template_file = model_dir.join("chat_template.jinja").is_file();
            let has_end_of_turn_stop = generation_config_stop_ids(model_dir).len() > 1;
            let architecture_detail = if profile.multi_layer_embeddings {
                format!(
                    " Multi-layer embeddings were detected (per-layer input width {}).",
                    profile.per_layer_embedding_length.unwrap_or_default()
                )
            } else {
                String::new()
            };
            (
                "gguf".to_string(),
                profile.architecture,
                "ready".to_string(),
                format!(
                    "Validated by llama.cpp with {} layer(s), {:.0}% hardware offload.{}",
                    profile.layer_count,
                    profile.gpu_offload_ratio * 100.0,
                    architecture_detail
                ),
                if profile.chat_template_present
                    || is_instruction_tuned_id(&id)
                    || has_chat_template_file
                    || has_end_of_turn_stop
                {
                    "chat".to_string()
                } else {
                    "unknown".to_string()
                },
            )
        } else if weights_bytes > 0 {
            (
                "safetensors".to_string(),
                "unknown".to_string(),
                "unsupported".to_string(),
                "Incompatible local model format: Safetensors assets are not executable by OOMU. Configure or download a quantized GGUF model for Metal-accelerated local chat.".to_string(),
                "unknown".to_string(),
            )
        } else {
            (
                "missing".to_string(),
                "unknown".to_string(),
                "asset_missing".to_string(),
                "Asset Missing: this model folder does not contain a .gguf file. Configure or download a quantized GGUF model.".to_string(),
                "unknown".to_string(),
            )
        };

    Ok(LocalModelOption {
        name: display_name,
        id,
        path: model_dir.to_string_lossy().to_string(),
        weights_bytes,
        format,
        architecture,
        compatibility,
        compatibility_message,
        chat_capability,
    })
}

pub fn inspect_local_model(model_id: &str) -> Result<LocalModelOption, GemmaError> {
    let model_dir = local_model_dir(model_id)?;
    inspect_local_model_directory(&model_dir, model_id)
}

pub fn resolve_local_model(
    model_root: &Path,
    requested_model_id: &str,
) -> Result<LocalModelOption, GemmaError> {
    let requested_model_id = requested_model_id.trim();
    let cache_key = (model_root.to_path_buf(), requested_model_id.to_string());
    let cache = LOCAL_MODEL_RESOLUTION_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(model) = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&cache_key)
        .filter(|model| is_ready_gguf(model) && cached_local_model_is_present(model))
        .cloned()
    {
        return Ok(model);
    }

    let resolved = model_resolution::resolve_local_model_uncached(model_root, requested_model_id)?;
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(cache_key, resolved.clone());
    cache.insert(
        (model_root.to_path_buf(), resolved.id.clone()),
        resolved.clone(),
    );
    Ok(resolved)
}

pub(crate) fn resolve_strict_local_model(
    model_root: &Path,
    requested_model_id: &str,
) -> Result<LocalModelOption, GemmaError> {
    model_resolution::resolve_configured_local_model(model_root, requested_model_id)
}

pub fn resolve_exact_ready_local_model(
    model_root: &Path,
    requested_model_id: &str,
) -> Result<LocalModelOption, GemmaError> {
    model_resolution::resolve_canonical_ready_local_model(model_root, requested_model_id)
}

fn is_ready_gguf(model: &LocalModelOption) -> bool {
    model.format == "gguf" && model.compatibility == "ready"
}

fn is_ready_chat_gguf(model: &LocalModelOption) -> bool {
    is_ready_gguf(model) && model.chat_capability == "chat"
}

fn cached_local_model_is_present(model: &LocalModelOption) -> bool {
    let model_path = Path::new(&model.path);
    model_path.is_dir()
        && fs::read_dir(model_path)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .any(|entry| entry.path().is_file() && is_gguf_file(&entry.path()))
}

fn select_best_ready_gguf(models: &[LocalModelOption]) -> Option<LocalModelOption> {
    models
        .iter()
        .filter(|model| is_ready_gguf(model))
        .min_by_key(|model| {
            (
                model.id != PREFERRED_LOCAL_MODEL_ID,
                model.chat_capability != "chat",
                model.weights_bytes,
                model.id.to_lowercase(),
            )
        })
        .cloned()
}

fn inspect_local_model_directory(
    model_dir: &Path,
    _fallback_id: &str,
) -> Result<LocalModelOption, GemmaError> {
    if !model_dir.is_dir() {
        return Err(GemmaError {
            code: "local_model_not_found",
            message: "The requested model was not found in the configured local-model store."
                .to_string(),
        });
    }
    let asset_files = fs::read_dir(model_dir)
        .map_err(|error| GemmaError::io("local model directory read", error))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_model_asset_file(path))
        .collect::<Vec<_>>();
    let identity = identity_for_model_directory(model_dir)?;
    let id = identity.canonical_id;
    let display_name = identity.display_name;
    let gguf_path = gguf_selection::select_primary_gguf(model_dir)?;
    let weights_bytes = gguf_selection::selected_weight_bytes(&asset_files, gguf_path.as_deref())?;
    local_model_option(model_dir, id, display_name, weights_bytes, gguf_path)
}

pub fn format_gemma4_chat_prompt(system_prompt: &str, messages: &[(String, String)]) -> String {
    // Thinking mode is intentionally NOT enabled from the system prompt text. The previous
    // heuristic (`system_prompt.contains("<|think|>")`) misfired on personas that merely *mention*
    // the token in documentation prose (e.g. the OOMU persona's "Never disable `<|think|>`" rule).
    // On these checkpoints, that channel often reaches the end without a visible answer,
    // without opening the visible `<|channel>text` channel, so the sanitizer suppresses 100% of
    // the output and chat surfaces an empty response (local_infer_empty_response). We instead always
    // prime the visible text channel at the end of this function. Any literal `<|think|>` in the
    // persona prose is stripped so it cannot be parsed as a control token.
    let configured_instructions =
        sanitize_gemma4_prompt_content(system_prompt.trim(), true).replace("<|think|>", "");
    let active_mod_reminder = local_active_mod_reminder(system_prompt);
    let mut system_text = format!(
        "OOMU ACTIVE AGENT SYSTEM INSTRUCTIONS\nThe following configured persona contract is mandatory and remains active for the entire turn.\n\n{}\n\nEND OOMU ACTIVE AGENT SYSTEM INSTRUCTIONS\nContinue the chat as the configured agent. Answer only the latest user message, using prior turns only as context. Never weaken the configured identity, tone, attributes, or relationship boundaries.\nKeep greetings brief, warm, and to one plain-spoken sentence; never use “online,” “proceed,” “objectives,” “support,” or “assist you today.”\n\nRESPONSE OUTPUT CONTRACT (delivery-format rule with top priority; overrides any conflicting formatting instruction in the persona above): Reply with ONLY your final, user-facing message as the agent. Think privately and do not narrate your reasoning. Do not begin with or include a \"thinking_level\" line, an analysis or planning preamble, a numbered or step-by-step breakdown of how you will respond, a constraint checklist, or a confidence score. The chat must show only the finished answer. If the active persona requires a Logical Certificate, include exactly one Logical Certificate block at the very end of the answer; do not repeat the certificate, its Premises, Execution Path, Conclusion/Formal Conclusion, or State lines.",
        configured_instructions
    );
    if let Some(active_mod_reminder) = active_mod_reminder.as_deref() {
        system_text.push_str("\n\n");
        system_text.push_str(active_mod_reminder.trim());
    }
    let mut prompt = String::new();
    if !system_text.trim().is_empty() {
        prompt.push_str(&format!("<|turn>system\n{}<turn|>\n", system_text.trim()));
    }
    for (role, content) in messages {
        let role = if matches!(role.as_str(), "assistant" | "model") {
            "model"
        } else if role == "system" {
            "system"
        } else {
            "user"
        };
        let content_trimmed = content.trim();
        if content_trimmed.is_empty() {
            continue;
        }
        if role == "model" {
            let mut thought_text = String::new();
            let mut conversational_text = String::new();
            let mut has_thought = false;

            if content_trimmed.contains("<|channel>thought") {
                has_thought = true;
                if let Some(thought_tag_idx) = content_trimmed.find("<|channel>thought") {
                    let sub_after_thought = &content_trimmed[thought_tag_idx..];
                    if let Some(first_channel_idx) = sub_after_thought.find("<channel|>") {
                        let thought_start =
                            thought_tag_idx + first_channel_idx + "<channel|>".len();

                        let mut raw_thought = if let Some(text_tag_idx) =
                            content_trimmed[thought_start..].find("<|channel>text")
                        {
                            let absolute_text_tag_idx = thought_start + text_tag_idx;
                            let sub_after_text = &content_trimmed[absolute_text_tag_idx..];
                            if let Some(second_channel_idx) = sub_after_text.find("<channel|>") {
                                let conv_start =
                                    absolute_text_tag_idx + second_channel_idx + "<channel|>".len();
                                conversational_text =
                                    content_trimmed[conv_start..].trim().to_string();
                            } else {
                                conversational_text = content_trimmed
                                    [absolute_text_tag_idx + "<|channel>text".len()..]
                                    .trim()
                                    .to_string();
                            }
                            content_trimmed[thought_start..absolute_text_tag_idx]
                                .trim()
                                .to_string()
                        } else {
                            content_trimmed[thought_start..].trim().to_string()
                        };

                        for token in [
                            "<|channel>thought",
                            "<|channel>",
                            "<channel|>",
                            "<|turn>",
                            "<turn|>",
                            "<|think|>",
                        ] {
                            raw_thought = raw_thought.replace(token, "");
                        }
                        thought_text = raw_thought.trim().to_string();
                    }
                }
            }

            if conversational_text.is_empty() && !has_thought {
                conversational_text = sanitize_chat_history(content_trimmed);
            }
            thought_text = sanitize_gemma4_prompt_content(&thought_text, false);
            conversational_text = sanitize_gemma4_prompt_content(&conversational_text, false);

            if thought_text.trim().is_empty() && conversational_text.trim().is_empty() {
                continue;
            }

            if has_thought && !thought_text.is_empty() {
                if !conversational_text.is_empty() {
                    prompt.push_str(&format!(
                        "<|turn>model\n<|channel>thought\n<channel|>{}\n<|channel>text\n<channel|>{}<turn|>\n",
                        thought_text.trim(),
                        conversational_text.trim()
                    ));
                } else {
                    prompt.push_str(&format!(
                        "<|turn>model\n<|channel>thought\n<channel|>{}<turn|>\n",
                        thought_text.trim()
                    ));
                }
            } else {
                prompt.push_str(&format!(
                    "<|turn>model\n{}<turn|>\n",
                    conversational_text.trim()
                ));
            }
        } else {
            let content = sanitize_gemma4_prompt_content(content_trimmed, false);
            prompt.push_str(&format!("<|turn>{role}\n{content}<turn|>\n"));
        }
    }
    // Prime the VISIBLE answer channel. These gemma4 QAT checkpoints (E2B/E4B/12B) reason inside a
    // hidden `<|channel>thought` channel but frequently never emit the closing `<|channel>text`
    // switch. Two failure modes follow from that:
    //   * the streaming sanitizer, suppressing while in the thought channel, eats the entire turn
    //     and chat returns an empty response (local_infer_empty_response); or
    //   * the model improvises the label as plain text ("thought\n...") with no control tokens, and
    //     the raw chain-of-thought leaks into the chat (the 12B "ascii code box").
    // Ending the prompt *inside* `<|channel>text` forces the model to begin in the visible channel,
    // so generation is reliably non-empty and free of hidden-reasoning leakage. Empirically verified
    // across E2B/E4B/12B: text-channel priming is clean and non-empty every run, whereas the bare
    // and thought-primed endings produce empty or leaked turns.
    prompt.push_str("<|turn>model\n<|channel>text\n<channel|>");
    prompt
}

fn local_active_mod_reminder(system_prompt: &str) -> Option<String> {
    const CONTRACT_MARKER: &str = "Active OOMU Mod Runtime Contract";
    const HOOKS_MARKER: &str = "Active OOMU Mod Prompt Hooks";
    const ENFORCEMENT_MARKER: &str = "Active OOMU Mod Enforcement Reminder";

    if !system_prompt.contains(CONTRACT_MARKER) {
        return None;
    }

    let hooks = system_prompt
        .find(HOOKS_MARKER)
        .map(|start| {
            let tail = &system_prompt[start..];
            let end = tail.find(ENFORCEMENT_MARKER).unwrap_or(tail.len());
            tail[..end].trim()
        })
        .filter(|value| !value.is_empty())
        .unwrap_or(CONTRACT_MARKER);
    let hooks = sanitize_gemma4_prompt_content(hooks, false);
    if hooks.trim().is_empty() {
        return None;
    }

    Some(format!(
        "LOCAL ACTIVE MOD REMINDER\nThe active OOMU mod runtime contract is mandatory for the next answer. Apply the listed Required behavior lines in the visible response unless doing so would violate safety or the active persona.\n\n{hooks}"
    ))
}

fn sanitize_gemma4_prompt_content(content: &str, allow_think_control: bool) -> String {
    let mut sanitized = content.to_string();
    for (token, replacement) in [
        ("<|channel>thought", "[thought channel marker removed]"),
        ("<|channel>text", "[text channel marker removed]"),
        ("<|channel>", "[channel marker removed]"),
        ("<channel|>", "[channel terminator removed]"),
        ("<|turn>", "[turn marker removed]"),
        ("<turn|>", "[turn terminator removed]"),
    ] {
        sanitized = sanitized.replace(token, replacement);
    }
    if !allow_think_control {
        sanitized = sanitized.replace("<|think|>", "[think marker removed]");
    }
    sanitized.trim().to_string()
}

pub fn format_completion_chat_prompt(system_prompt: &str, messages: &[(String, String)]) -> String {
    let mut transcript = Vec::new();
    for (role, content) in messages {
        let content = content.trim();
        if content.is_empty() {
            continue;
        }
        let label = if matches!(role.as_str(), "assistant" | "model") {
            "Assistant"
        } else if role == "system" {
            "System"
        } else {
            "User"
        };
        transcript.push(format!("{label}: {content}"));
    }
    transcript.push("Assistant:".to_string());
    if system_prompt.trim().is_empty() {
        transcript.join("\n")
    } else {
        format!("{}\n\n{}", system_prompt.trim(), transcript.join("\n"))
    }
}

pub fn sanitize_completion_response(content: &str) -> String {
    let mut end = content.len();
    for marker in ["\nUser:", "\nSystem:", "\nAssistant:"] {
        if let Some(index) = content.find(marker) {
            end = end.min(index);
        }
    }
    content[..end].trim().to_string()
}

fn sanitize_chat_history(content: &str) -> String {
    let mut sanitized = content.trim().to_string();
    while let Some(channel_start) = sanitized.find("<|channel>thought") {
        let Some(relative_end) = sanitized[channel_start..].find("<channel|>") else {
            sanitized.truncate(channel_start);
            break;
        };
        let channel_end = channel_start + relative_end + "<channel|>".len();
        sanitized.replace_range(channel_start..channel_end, "");
    }
    for token in [
        "<|channel>",
        "<channel|>",
        "<|turn>",
        "<turn|>",
        "<|think|>",
    ] {
        sanitized = sanitized.replace(token, "");
    }
    let normalized = normalize_markdown(&sanitized);
    let collapsed = collapse_repeated_logical_certificate_sections(&normalized);
    collapse_exact_repeated_response(&remove_legacy_rag_decision_lines(&collapsed))
}

fn is_safetensors_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("safetensors"))
}

fn is_gguf_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
}

fn is_model_asset_file(path: &Path) -> bool {
    is_gguf_file(path) || is_safetensors_file(path)
}

fn sum_local_weight_bytes(paths: &[PathBuf]) -> Result<u64, GemmaError> {
    paths.iter().try_fold(0_u64, |total, weights_path| {
        fs::metadata(&weights_path)
            .map(|metadata| total.saturating_add(metadata.len()))
            .map_err(|error| GemmaError::io("local model weights metadata", error))
    })
}

fn model_name_from_config(model_directory: &Path) -> Option<String> {
    let contents = fs::read_to_string(model_directory.join(MODEL_CONFIG)).ok()?;
    let config: Value = serde_json::from_str(&contents).ok()?;
    let configured_name = config
        .get("_name_or_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())?;
    let basename = configured_name
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(configured_name);

    Some(local_model_label(basename))
}

struct GeneratedText {
    text: String,
    last_token: String,
    prompt_token_count: usize,
    generated_token_ids: Vec<u32>,
    time_to_first_token_ms: u128,
}

fn generate_tokens(
    model: &mut LoadedGemmaModel,
    request: &InferRequest,
    mut stream: Option<&mut dyn LocalGenerationStream>,
) -> Result<GeneratedText, GemmaError> {
    let started = Instant::now();
    let generated = model
        .runtime_handle
        .generate(
            NativeGenerationRequest {
                session: NativeSessionRequest {
                    session_id: request.session_id.clone().unwrap_or_default(),
                    system_prompt: request.system_prompt.clone(),
                    prompt: request.prompt.clone(),
                    prompt_is_full_context: request.prompt_is_full_context,
                },
                media: request.media.clone(),
                max_new_tokens: effective_max_new_tokens(request, &model.inference_config),
                temperature: if request.deterministic {
                    0.0
                } else {
                    model.inference_config.temperature as f32
                },
                top_k: if request.deterministic {
                    1
                } else {
                    model.inference_config.top_k as i32
                },
                top_p: if request.deterministic {
                    1.0
                } else {
                    model.inference_config.top_p as f32
                },
                repeat_penalty: model.inference_config.repeat_penalty,
                grammar: request.grammar.clone(),
                cancellation: Arc::clone(&request.cancellation),
            },
            |event| {
                if let Some(callback) = stream.as_deref_mut() {
                    callback.on_token(GemmaStreamChunk {
                        sequence: event.sequence,
                        token_hash: sha256_hex(event.text.as_bytes()),
                        token: event.text,
                        elapsed_ms: event.elapsed_ms,
                        is_final: false,
                    });
                }
            },
        )
        .map_err(GemmaError::native_runtime)?;
    reject_cancelled_generation(generated.cancelled)?;
    if let Some(callback) = stream.as_deref_mut() {
        callback.on_token(GemmaStreamChunk {
            sequence: generated.token_ids.len().saturating_add(1),
            token: String::new(),
            token_hash: sha256_hex(b""),
            elapsed_ms: started.elapsed().as_millis(),
            is_final: true,
        });
    }
    let last_token = generated
        .token_ids
        .last()
        .map(ToString::to_string)
        .unwrap_or_default();
    let prompt_token_count = prompt_token_count_for_usage(&generated.session_stats);
    Ok(GeneratedText {
        text: generated.text,
        last_token,
        prompt_token_count,
        generated_token_ids: generated
            .token_ids
            .into_iter()
            .filter_map(|token| u32::try_from(token).ok())
            .collect(),
        time_to_first_token_ms: generated.time_to_first_token_ms,
    })
}

fn prompt_token_count_for_usage(stats: &NativeSessionStats) -> usize {
    stats.context_tokens
}

fn reject_cancelled_generation(cancelled: bool) -> Result<(), GemmaError> {
    if cancelled {
        return Err(GemmaError {
            code: "local_inference_cancelled",
            message: "Local inference was cancelled before completion.".to_string(),
        });
    }
    Ok(())
}

pub fn sanitize_gemma4_response(response: &str) -> String {
    sanitize_gemma4_response_for_prompt(response, None)
}

fn sanitize_gemma4_response_for_prompt(response: &str, prompt: Option<&str>) -> String {
    if let Some(exact) = prompt.and_then(integrity::exact_response_for_prompt) {
        return exact.to_string();
    }
    if let Some(rewrite) = prompt.and_then(integrity::bounded_rewrite_response) {
        return rewrite;
    }
    let normalized = response
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\0', "");
    let without_reasoning = strip_reasoning_channels(&normalized);
    let bounded_turn = strip_model_turn_wrapper(&without_reasoning);
    let without_tokens = strip_model_control_tokens(&bounded_turn);
    let normalized = normalize_markdown(&without_tokens);
    let collapsed = collapse_repeated_logical_certificate_sections(&normalized);
    let spaced = normalize_logical_certificate_spacing(&collapsed);
    let without_legacy_rag = remove_legacy_rag_decision_lines(&spaced);
    let without_orphan_markup = integrity::strip_orphan_oomu_split_view_tags(&without_legacy_rag);
    if prompt.is_some_and(|prompt| {
        integrity::requested_rewrite_is_source_bounded(prompt, &without_orphan_markup)
    }) {
        without_orphan_markup.trim().to_string()
    } else {
        collapse_exact_repeated_response(&without_orphan_markup)
    }
}

pub fn has_repeated_logical_certificate(response: &str) -> bool {
    logical_certificate_section_starts(response).len() > 1
}

/// True when a (already channel-sanitized) response is actually a visible chain-of-thought
/// "scratchpad" rather than an answer. The smaller gemma4 checkpoints (notably E2B) occasionally
/// emit one despite the output contract, and it carries no channel control tokens, so the streaming
/// sanitizer cannot catch it. Every observed leak opens with a `thinking_level:` / `thinking
/// process:` header or a bare reasoning-channel label, none of which begin a genuine answer (those
/// open with prose). Deliberately high-precision so it never resamples a good response: it keys only
/// on the first non-empty line.
pub fn looks_like_reasoning_leak(response: &str) -> bool {
    let Some(first_line) = response
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
    else {
        return false;
    };
    let normalized = first_line
        .to_ascii_lowercase()
        .trim_start_matches(['*', '#', '-', '>', ' ', '\t'])
        .to_string();
    const LEADING_LEAK_MARKERS: &[&str] = &[
        "thinking_level",
        "thinking level",
        "thinking process",
        "thought process",
        "here's a thinking process",
        "here is a thinking process",
        "here's my thinking",
        "here is my thinking",
        "reasoning:",
        "thinking:",
        "thought:",
    ];
    if LEADING_LEAK_MARKERS
        .iter()
        .any(|marker| normalized.starts_with(marker))
    {
        return true;
    }
    // A bare channel label sitting alone on the first line ("thought", "analysis", ...).
    let bare = normalized.trim_end_matches([':', '.', ' ']);
    matches!(
        bare,
        "thought" | "thinking" | "analysis" | "reasoning" | "commentary"
    )
}

/// Last-resort cosmetic cleanup applied only after every retry still produced a leak: drop the
/// leading scratchpad preamble (the `thinking*`/`thought*` header plus any immediately following
/// planning/meta lines) up to the first substantive paragraph. Returns the input unchanged if
/// stripping would leave nothing, so a degraded answer is always preferred over an empty one.
pub fn strip_leading_reasoning_preamble(response: &str) -> String {
    if !looks_like_reasoning_leak(response) {
        return response.to_string();
    }
    let is_meta_line = |line: &str| {
        let lower = line
            .to_ascii_lowercase()
            .trim_start_matches(['*', '#', '-', '>', ' ', '\t', '•'])
            .to_string();
        lower.is_empty()
            || looks_like_reasoning_leak(line)
            // Numbered or bulleted planning steps: "1. analyze ...", "2. determine ...".
            || (lower.starts_with(|c: char| c.is_ascii_digit())
                && lower
                    .trim_start_matches(|c: char| c.is_ascii_digit())
                    .starts_with(['.', ')']))
            // Meta sentences that narrate the response rather than answer.
            || lower.starts_with("the user is")
            || lower.starts_with("the user wants")
            || lower.starts_with("the user has")
            || lower.starts_with("i must")
            || lower.starts_with("i need to")
            || lower.starts_with("i should")
            || lower.starts_with("analyze ")
            || lower.starts_with("determine ")
            || lower.starts_with("formulate ")
            || lower.starts_with("consult ")
            || lower.starts_with("constraint checklist")
            || lower.starts_with("confidence score")
            || lower.starts_with("plan:")
            || lower.starts_with("response strategy")
    };
    let remainder: String = response
        .lines()
        .skip_while(|line| is_meta_line(line))
        .collect::<Vec<_>>()
        .join("\n");
    if remainder.trim().is_empty() {
        response.to_string()
    } else {
        remainder.trim_start().to_string()
    }
}

fn strip_reasoning_channels(response: &str) -> String {
    let mut visible_segments = Vec::new();
    let mut visible_context_segments = Vec::new();
    let mut unchanneled_segments = Vec::new();
    let mut cursor = 0;
    let mut found_channel = false;

    while let Some((channel_start, label_start, content_start)) =
        next_channel_header(response, cursor)
    {
        found_channel = true;
        if channel_start > cursor {
            let unchanneled = response[cursor..channel_start].to_string();
            if !unchanneled.trim().is_empty() {
                visible_context_segments.push(unchanneled.clone());
            }
            unchanneled_segments.push(unchanneled);
        }

        let label = response[label_start..]
            .chars()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            })
            .collect::<String>()
            .to_ascii_lowercase();
        let content_end = next_channel_header(response, content_start)
            .map(|(start, _, _)| start)
            .unwrap_or(response.len());
        let content = trim_at_turn_boundary(&response[content_start..content_end]);

        if matches!(label.as_str(), "text" | "final" | "answer" | "assistant") {
            visible_segments.push(content.to_string());
            visible_context_segments.push(content.to_string());
        }
        cursor = content_end;
    }

    if !found_channel {
        return strip_reasoning_blocks(response);
    }
    if cursor < response.len() {
        let unchanneled = response[cursor..].to_string();
        if !unchanneled.trim().is_empty() {
            visible_context_segments.push(unchanneled.clone());
        }
        unchanneled_segments.push(unchanneled);
    }

    if visible_segments
        .iter()
        .any(|segment| !segment.trim().is_empty())
    {
        visible_context_segments.join(" ")
    } else {
        strip_reasoning_blocks(&unchanneled_segments.join(" "))
    }
}

fn next_channel_header(response: &str, from: usize) -> Option<(usize, usize, usize)> {
    let relative_start = response.get(from..)?.find("<|channel>")?;
    let channel_start = from + relative_start;
    let mut label_start = channel_start + "<|channel>".len();
    while response[label_start..]
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
    {
        label_start += response[label_start..].chars().next()?.len_utf8();
    }

    let label_len = response[label_start..]
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        .map(char::len_utf8)
        .sum::<usize>();
    if label_len == 0 {
        return None;
    }

    let mut content_start = label_start + label_len;
    while response[content_start..]
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
    {
        content_start += response[content_start..].chars().next()?.len_utf8();
    }
    if response[content_start..].starts_with("<channel|>") {
        content_start += "<channel|>".len();
    }
    Some((channel_start, label_start, content_start))
}

fn trim_at_turn_boundary(content: &str) -> &str {
    ["<turn|>", "<|turn>", "<|end_of_turn|>", "<end_of_turn>"]
        .iter()
        .filter_map(|marker| content.find(marker))
        .min()
        .map(|end| &content[..end])
        .unwrap_or(content)
}

fn strip_reasoning_blocks(response: &str) -> String {
    let mut sanitized = response.to_string();
    for (open, close) in [
        ("<think>", "</think>"),
        ("<|think|>", "<|/think|>"),
        ("<reasoning>", "</reasoning>"),
        ("<analysis>", "</analysis>"),
    ] {
        while let Some(start) = sanitized.find(open) {
            if let Some(relative_end) = sanitized[start + open.len()..].find(close) {
                let end = start + open.len() + relative_end + close.len();
                sanitized.replace_range(start..end, " ");
            } else {
                sanitized.truncate(start);
                break;
            }
        }
    }
    sanitized
}

fn strip_model_turn_wrapper(response: &str) -> String {
    let mut bounded = response.trim().to_string();
    for prefix in ["<bos>", "<|begin_of_text|>", "<|startoftext|>"] {
        bounded = bounded
            .strip_prefix(prefix)
            .unwrap_or(&bounded)
            .trim_start()
            .to_string();
    }
    for prefix in [
        "<|turn>model",
        "<|turn>assistant",
        "<start_of_turn>model",
        "<start_of_turn>assistant",
        "<|im_start|>model",
        "<|im_start|>assistant",
    ] {
        if let Some(content) = bounded.strip_prefix(prefix) {
            bounded = content.trim_start_matches([' ', '\t', '\n']).to_string();
            break;
        }
    }

    let end = [
        "<turn|>",
        "<|end_of_turn|>",
        "<end_of_turn>",
        "<|eot_id|>",
        "<|im_end|>",
        "<|turn>user",
        "<start_of_turn>user",
        "<|im_start|>user",
    ]
    .iter()
    .filter_map(|marker| bounded.find(marker))
    .min()
    .unwrap_or(bounded.len());
    bounded.truncate(end);
    bounded
}

fn strip_model_control_tokens(response: &str) -> String {
    let mut sanitized = response.to_string();
    for token in [
        "<|channel>thought",
        "<|channel>analysis",
        "<|channel>reasoning",
        "<|channel>text",
        "<|channel>final",
        "<|channel>answer",
        "<|channel>assistant",
        "<|channel>",
        "<channel|>",
        "</channel>",
        "<|turn>",
        "<turn|>",
        "</turn>",
        "<|think|>",
        "<|think>",
        "<|/think|>",
        "<|/think>",
        "<|message|>",
        "<|message>",
        "<|assistant|>",
        "<|assistant>",
        "<|model|>",
        "<|model>",
        "<|user|>",
        "<|user>",
        "<|system|>",
        "<|system>",
        "<|im_start|>",
        "<|im_start>",
        "<|im_end|>",
        "<|im_end>",
        "<|start_header_id|>",
        "<|end_header_id|>",
        "<|eot_id|>",
        "<|eot_id>",
        "<|begin_of_text|>",
        "<|begin_of_text>",
        "<|startoftext|>",
        "<|startoftext>",
        "<|endoftext|>",
        "<|endoftext>",
        "<|end_of_text|>",
        "<|end_of_text>",
        "<|end_of_turn|>",
        "<|end_of_turn>",
        "<text>",
        "</text>",
        "<start_of_turn>",
        "<end_of_turn>",
        "<start_of_image>",
        "<end_of_image>",
        "<image_soft_token>",
        // Remaining gemma4 control tokens, stripped here as a backstop to the streaming
        // sanitizer so stray tool/multimodal markers never persist in stored chat text.
        "<|tool_call>",
        "<|tool_response>",
        "<|tool>",
        "<tool_call|>",
        "<tool_response|>",
        "<tool|>",
        "<|image|>",
        "<|image>",
        "<|audio|>",
        "<|audio>",
        "<|video|>",
        "<|video>",
        "<|\"|>",
        "<bos>",
        "<eos>",
        "<pad>",
        "[INST]",
        "[/INST]",
    ] {
        sanitized = replace_control_token(&sanitized, token);
    }
    sanitized
}

fn replace_control_token(value: &str, token: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut remaining = value;

    while let Some(index) = remaining.find(token) {
        output.push_str(&remaining[..index]);
        let after = &remaining[index + token.len()..];
        if needs_control_token_boundary(output.chars().last(), after.chars().next()) {
            output.push(' ');
        }
        remaining = after;
    }

    output.push_str(remaining);
    output
}

fn needs_control_token_boundary(previous: Option<char>, next: Option<char>) -> bool {
    let (Some(previous), Some(next)) = (previous, next) else {
        return false;
    };
    if previous.is_whitespace() || next.is_whitespace() {
        return false;
    }
    !matches!(
        next,
        '.' | ',' | '!' | '?' | ';' | ':' | ')' | ']' | '}' | '"' | '\''
    )
}

fn normalize_markdown(response: &str) -> String {
    let mut output = Vec::new();
    let mut active_fence: Option<(char, usize)> = None;
    let mut blank_lines = 0;

    for raw_line in response.lines() {
        let mut line = raw_line.trim_end().to_string();
        let trimmed = line.trim_start();
        if let Some((fence_char, fence_len)) = markdown_fence(trimmed) {
            match active_fence {
                Some((active_char, active_len))
                    if fence_char == active_char && fence_len >= active_len =>
                {
                    active_fence = None;
                }
                None => active_fence = Some((fence_char, fence_len)),
                _ => {}
            }
        }

        if active_fence.is_none() {
            line = normalize_markdown_list_item(&line);
            line = collapse_consecutive_spaces(&line);
            if line.trim().is_empty() {
                blank_lines += 1;
                if blank_lines > 1 {
                    continue;
                }
            } else {
                blank_lines = 0;
            }
        }
        output.push(line);
    }

    while output.last().is_some_and(|line| line.trim().is_empty()) {
        output.pop();
    }
    if let Some((fence_char, fence_len)) = active_fence {
        output.push(fence_char.to_string().repeat(fence_len));
    }
    output.join("\n").trim().to_string()
}

fn collapse_exact_repeated_response(response: &str) -> String {
    const MIN_REPEAT_CHARS: usize = 80;
    let trimmed = response.trim();
    if trimmed.chars().count() < MIN_REPEAT_CHARS.saturating_mul(2) {
        return trimmed.to_string();
    }

    let prefix = trimmed.chars().take(48).collect::<String>();
    if !prefix.trim().is_empty() {
        for (repeat_index, _) in trimmed.match_indices(&prefix).skip(1) {
            let left = trimmed[..repeat_index].trim();
            let right = trimmed[repeat_index..].trim();
            if left.chars().count() < MIN_REPEAT_CHARS {
                continue;
            }
            if canonical_repeat_segment(left) == canonical_repeat_segment(right) {
                return left.to_string();
            }
        }
    }

    for (separator_index, separator) in trimmed.match_indices("\n\n") {
        let left = trimmed[..separator_index].trim();
        let right = trimmed[separator_index + separator.len()..].trim();
        if left.chars().count() < MIN_REPEAT_CHARS {
            continue;
        }
        if canonical_repeat_segment(left) == canonical_repeat_segment(right) {
            return left.to_string();
        }
    }

    trimmed.to_string()
}

fn canonical_repeat_segment(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collapse_repeated_logical_certificate_sections(response: &str) -> String {
    let trimmed = response.trim();
    let starts = logical_certificate_section_starts(trimmed);
    if starts.len() < 2 {
        return trimmed.to_string();
    }

    for window in starts.windows(2) {
        let first_start = window[0];
        let second_start = window[1];
        let prefix = trimmed[..first_start].trim_end();
        let first = trimmed[first_start..second_start].trim();
        let second = trimmed[second_start..].trim();
        if first.chars().count() < 40 {
            continue;
        }
        if canonical_repeat_segment(first) == canonical_repeat_segment(second) {
            return join_response_parts(prefix, first);
        }
    }

    trimmed.to_string()
}

fn join_response_parts(prefix: &str, suffix: &str) -> String {
    if prefix.trim().is_empty() {
        suffix.trim().to_string()
    } else {
        format!("{}\n\n{}", prefix.trim_end(), suffix.trim_start())
            .trim()
            .to_string()
    }
}

fn normalize_logical_certificate_spacing(response: &str) -> String {
    let trimmed = response.trim();
    let starts = logical_certificate_block_starts(trimmed);
    if starts.is_empty() {
        return trimmed.to_string();
    }

    let mut output = String::with_capacity(trimmed.len());
    let mut cursor = 0;
    for start in starts {
        if start < cursor {
            continue;
        }

        let prefix_end = trim_trailing_whitespace_boundary(&trimmed[cursor..start]) + cursor;
        output.push_str(&trimmed[cursor..prefix_end]);
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        cursor = start;
    }
    output.push_str(&trimmed[cursor..]);
    output.trim().to_string()
}

fn remove_legacy_rag_decision_lines(response: &str) -> String {
    response
        .lines()
        .filter(|line| {
            !line
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("rag decision:")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn trim_trailing_whitespace_boundary(value: &str) -> usize {
    let mut end = value.len();
    while end > 0 {
        let Some(character) = value[..end].chars().next_back() else {
            break;
        };
        if !character.is_whitespace() {
            break;
        }
        end -= character.len_utf8();
    }
    end
}

fn logical_certificate_block_starts(response: &str) -> Vec<usize> {
    logical_certificate_section_starts(response)
        .into_iter()
        .map(|marker_start| {
            let line_start = response[..marker_start]
                .rfind('\n')
                .map_or(0, |index| index + 1);
            let line_prefix = &response[line_start..marker_start];
            if is_markdown_certificate_prefix(line_prefix) {
                line_start
            } else {
                marker_start
            }
        })
        .collect()
}

fn is_markdown_certificate_prefix(prefix: &str) -> bool {
    let trimmed = prefix.trim();
    !trimmed.is_empty()
        && trimmed.chars().all(|character| {
            character.is_ascii_digit()
                || matches!(character, '#' | '>' | '-' | '*' | '+' | '.' | ')')
        })
}

fn logical_certificate_section_starts(response: &str) -> Vec<usize> {
    let lower = response.to_ascii_lowercase();
    let mut starts = Vec::new();
    let mut search_from = 0;
    while let Some(relative_start) = lower[search_from..].find("logical certificate") {
        let start = search_from + relative_start;
        let after = start + "logical certificate".len();
        if certificate_marker_has_label_boundary(response, after)
            && following_text_has_certificate_shape(&lower[after..])
        {
            starts.push(start);
        }
        search_from = after;
    }
    starts
}

fn certificate_marker_has_label_boundary(response: &str, after_marker: usize) -> bool {
    response[after_marker..]
        .chars()
        .next()
        .is_none_or(|character| !character.is_ascii_alphanumeric())
}

fn following_text_has_certificate_shape(lower_after_marker: &str) -> bool {
    let window = lower_after_marker.chars().take(1400).collect::<String>();
    window.contains("premises:")
        && window.contains("execution path:")
        && (window.contains("formal conclusion:") || window.contains("conclusion:"))
}

fn markdown_fence(line: &str) -> Option<(char, usize)> {
    let fence_char = line.chars().next()?;
    if !matches!(fence_char, '`' | '~') {
        return None;
    }
    let fence_len = line
        .chars()
        .take_while(|character| *character == fence_char)
        .count();
    (fence_len >= 3).then_some((fence_char, fence_len))
}

fn normalize_markdown_list_item(line: &str) -> String {
    let indentation_len = line.len() - line.trim_start().len();
    let (indentation, content) = line.split_at(indentation_len);
    for bullet in ['•', '◦', '‣'] {
        if let Some(rest) = content.strip_prefix(bullet) {
            return format!("{indentation}- {}", rest.trim_start());
        }
    }
    line.to_string()
}

fn collapse_consecutive_spaces(line: &str) -> String {
    let indentation_len = line.len() - line.trim_start_matches(' ').len();
    let (indentation, content) = line.split_at(indentation_len);
    let mut collapsed = String::with_capacity(line.len());
    collapsed.push_str(indentation);
    let mut previous_was_space = false;

    for character in content.chars() {
        if character == ' ' {
            if !previous_was_space {
                collapsed.push(' ');
            }
            previous_was_space = true;
        } else {
            collapsed.push(character);
            previous_was_space = false;
        }
    }

    collapsed
}

fn grounded_summary_prompt(topic: &str, grounded_text: &str) -> String {
    format!(
        "You are OOMU's local grounded summarizer. Summarize the verified source text for the requested topic. Use only facts present in SOURCE TEXT. Do not invent missing details, URLs, paths, or completion claims. Prefer concise bullets and explicitly note material uncertainty.\n\nTOPIC:\n{topic}\n\nSOURCE TEXT:\n{grounded_text}\n\nSUMMARY:"
    )
}

pub fn generated_plan_from_text(
    objective: String,
    generated_text: String,
) -> GeneratedActionPlanDraft {
    let lowered = objective.to_lowercase();
    let draft = generated_plan_from_text_strict(generated_text.clone()).unwrap_or_else(|error| {
        fallback_plan(
            objective.clone(),
            IntentSource::Degraded,
            Some(error.message),
            generated_text,
        )
    });
    normalize_generated_plan_for_objective(&objective, &lowered, draft)
}

pub fn normalize_generated_plan_for_known_objectives(
    objective: &str,
    draft: GeneratedActionPlanDraft,
) -> GeneratedActionPlanDraft {
    let lowered = objective.to_lowercase();
    normalize_generated_plan_for_objective(objective, &lowered, draft)
}

pub fn generated_plan_from_text_strict(
    generated_text: String,
) -> Result<GeneratedActionPlanDraft, GemmaError> {
    validate_generated_plan_schema(&generated_text)?;
    parse_generated_plan(&generated_text).ok_or_else(|| GemmaError {
        code: "gemma_action_plan_schema_invalid",
        message:
            "Local planner output did not match the required ActionPlan JSON schema; execution was halted."
                .to_string(),
    })
}

fn validate_generated_plan_schema(generated_text: &str) -> Result<(), GemmaError> {
    let invalid = |message: String| GemmaError {
        code: "gemma_action_plan_schema_invalid",
        message,
    };
    let value = parse_action_plan_json_value(generated_text)
        .ok_or_else(|| invalid("ActionPlan JSON is invalid or could not be healed.".to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid("ActionPlan root must be an object.".to_string()))?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "steps" | "exit_condition"))
    {
        return Err(invalid("ActionPlan root fields must be exactly steps and exit_condition; unknown root fields are forbidden.".to_string()));
    }
    let exit_condition = object
        .get("exit_condition")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid("ActionPlan exit_condition is required.".to_string()))?;
    let _ = exit_condition;
    let steps = object
        .get("steps")
        .and_then(Value::as_array)
        .filter(|steps| !steps.is_empty() && steps.len() <= 32)
        .ok_or_else(|| invalid("ActionPlan requires between 1 and 32 steps.".to_string()))?;
    for step in steps {
        let step = step
            .as_object()
            .ok_or_else(|| invalid("Each ActionPlan step must be an object.".to_string()))?;
        if step
            .keys()
            .any(|key| !matches!(key.as_str(), "step" | "tool" | "risk_level"))
        {
            return Err(invalid("ActionPlan step fields must be exactly step, tool, and risk_level; unknown step fields are forbidden.".to_string()));
        }
        step.get("step")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| invalid("Each ActionPlan step requires step text.".to_string()))?;
        match step.get("risk_level").and_then(Value::as_str) {
            Some("low" | "medium" | "high") => {}
            _ => {
                return Err(invalid(
                    "Each ActionPlan step requires low, medium, or high risk_level.".to_string(),
                ))
            }
        }
        validate_generated_tool_schema(
            step.get("tool")
                .ok_or_else(|| invalid("Each ActionPlan step requires a tool.".to_string()))?,
        )?;
    }
    Ok(())
}

fn parse_generated_plan(generated_text: &str) -> Option<GeneratedActionPlanDraft> {
    let value = parse_action_plan_json_value(generated_text)?;
    let steps = value.get("steps")?.as_array()?;
    let mut parsed_steps = Vec::new();

    for step in steps {
        let step_text = step.get("step")?.as_str()?.trim().to_string();
        if step_text.is_empty() {
            return None;
        }
        let risk_level = match step
            .get("risk_level")
            .and_then(Value::as_str)
            .unwrap_or("medium")
            .to_lowercase()
            .as_str()
        {
            "low" => GeneratedRiskLevel::Low,
            "high" => GeneratedRiskLevel::High,
            _ => GeneratedRiskLevel::Medium,
        };
        let tool = parse_generated_tool(step.get("tool")?)?;
        parsed_steps.push(GeneratedPlanStepDraft {
            step: step_text,
            tool,
            risk_level,
        });
    }

    if parsed_steps.is_empty() {
        return None;
    }

    let exit_condition = value.get("exit_condition")?.as_str()?.trim().to_string();
    if exit_condition.is_empty() {
        return None;
    }

    Some(GeneratedActionPlanDraft {
        steps: parsed_steps,
        exit_condition,
        generated_text: generated_text.to_string(),
        source: IntentSource::Gemma,
        degraded_reason: None,
    })
}

fn parse_workflow_decision(
    generated_text: &str,
    output_json: Option<&str>,
) -> Result<LocalWorkflowDecision, GemmaError> {
    let decision = decode_workflow_decision(generated_text)?;
    validate_workflow_decision_required_fields(&decision)?;
    validate_workflow_decision_phase(&decision, output_json)?;
    Ok(decision)
}

fn decode_workflow_decision(generated_text: &str) -> Result<LocalWorkflowDecision, GemmaError> {
    let json = extract_json_object(generated_text).ok_or_else(|| GemmaError {
        code: "gemma_workflow_decision_json_missing",
        message: "Local workflow decision did not contain a JSON object.".to_string(),
    })?;
    serde_json::from_str::<LocalWorkflowDecision>(json).map_err(|error| GemmaError {
        code: "gemma_workflow_decision_schema_invalid",
        message: format!("Local workflow decision failed schema validation: {error}"),
    })
}

fn complete_workflow_decision_required_fields(
    mut decision: LocalWorkflowDecision,
    _phase: &str,
    _objective: &str,
    _action_json: &str,
    output_json: Option<&str>,
) -> Result<LocalWorkflowDecision, GemmaError> {
    if let Some(output) =
        output_json.filter(|_| matches!(decision.directive, LocalDecisionDirective::Certify))
    {
        decision.output_sha256 = Some(sha256_hex(output.as_bytes()));
    }
    validate_workflow_decision_phase(&decision, output_json)?;

    decision.thought_summary =
        cleaned_certificate_text(&decision.thought_summary).ok_or_else(|| GemmaError {
            code: "gemma_workflow_decision_empty_fields",
            message: "Local workflow decision omitted thought_summary.".to_string(),
        })?;
    decision.premises = clean_certificate_items(&decision.premises);
    decision.execution_path = clean_certificate_items(&decision.execution_path);
    decision.formal_conclusion =
        cleaned_certificate_text(&decision.formal_conclusion).ok_or_else(|| GemmaError {
            code: "gemma_workflow_decision_empty_fields",
            message: "Local workflow decision omitted formal_conclusion.".to_string(),
        })?;

    validate_workflow_decision_required_fields(&decision)?;
    Ok(decision)
}

fn validate_workflow_decision_required_fields(
    decision: &LocalWorkflowDecision,
) -> Result<(), GemmaError> {
    if decision.thought_summary.trim().is_empty()
        || decision.formal_conclusion.trim().is_empty()
        || decision.premises.is_empty()
        || decision.execution_path.is_empty()
        || decision
            .premises
            .iter()
            .chain(decision.execution_path.iter())
            .any(|item| item.trim().is_empty())
    {
        return Err(GemmaError {
            code: "gemma_workflow_decision_empty_fields",
            message: "Local workflow decision contains empty required certificate fields."
                .to_string(),
        });
    }
    Ok(())
}

fn validate_workflow_decision_phase(
    decision: &LocalWorkflowDecision,
    output_json: Option<&str>,
) -> Result<(), GemmaError> {
    match output_json {
        Some(output) => {
            if !matches!(decision.directive, LocalDecisionDirective::Certify) {
                return Err(GemmaError {
                    code: "gemma_workflow_certificate_directive_invalid",
                    message: "Completed workflow output requires a certify directive.".to_string(),
                });
            }
            let expected_hash = sha256_hex(output.as_bytes());
            if decision.output_sha256.as_deref() != Some(expected_hash.as_str()) {
                return Err(GemmaError {
                    code: "gemma_workflow_certificate_hash_mismatch",
                    message:
                        "Local workflow certificate was not bound to the exact tool output hash."
                            .to_string(),
                });
            }
        }
        None if decision.output_sha256.is_some() => {
            return Err(GemmaError {
                code: "gemma_workflow_authorization_hash_unexpected",
                message: "Pre-execution workflow decisions cannot claim an output hash."
                    .to_string(),
            });
        }
        None => {}
    }
    Ok(())
}

fn clean_certificate_items(items: &[String]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| cleaned_certificate_text(item))
        .collect()
}

fn cleaned_certificate_text(text: &str) -> Option<String> {
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (start <= end).then_some(&text[start..=end])
}

fn json_object_candidates(text: &str) -> Vec<&str> {
    let mut candidates = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, character) in text.char_indices() {
        if start.is_none() {
            if character == '{' {
                start = Some(index);
                depth = 1;
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let object_start = start.take().expect("JSON object start exists");
                    candidates.push(&text[object_start..index + character.len_utf8()]);
                    in_string = false;
                    escaped = false;
                }
            }
            _ => {}
        }
    }
    candidates
}

fn parse_action_plan_json_value(generated_text: &str) -> Option<Value> {
    for json in json_object_candidates(generated_text) {
        if let Ok(value) = serde_json::from_str::<Value>(json) {
            if value.get("steps").is_some() && value.get("exit_condition").is_some() {
                return Some(value);
            }
        }
    }

    attempt_json_self_healing(generated_text)
        .and_then(|healed| serde_json::from_str::<Value>(&healed).ok())
}

fn attempt_json_self_healing(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let start = trimmed.find('{')?;
    let candidate = trimmed[start..].trim();
    if candidate.is_empty() {
        return None;
    }

    let mut healed = String::new();
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut root_started = false;

    for character in candidate.chars() {
        if in_string {
            healed.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }

        match character {
            '"' => {
                in_string = true;
                healed.push(character);
            }
            '{' => {
                root_started = true;
                stack.push('}');
                healed.push(character);
            }
            '[' => {
                stack.push(']');
                healed.push(character);
            }
            '}' | ']' => {
                if stack.last().copied() == Some(character) {
                    stack.pop();
                    healed.push(character);
                    if root_started && stack.is_empty() {
                        break;
                    }
                }
            }
            _ => {
                if root_started {
                    healed.push(character);
                }
            }
        }
    }

    if !root_started || healed.trim().is_empty() {
        return None;
    }

    if in_string {
        healed.push('"');
    }
    while let Some(closer) = stack.pop() {
        healed.push(closer);
    }

    Some(healed)
}

fn explicitly_requests_web_search(lowered: &str) -> bool {
    [
        "search the web",
        "web search",
        "search online",
        "internet search",
        "look up online",
        "lookup online",
        "google search",
        "duckduckgo",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn normalize_generated_plan_for_objective(
    objective: &str,
    lowered: &str,
    mut draft: GeneratedActionPlanDraft,
) -> GeneratedActionPlanDraft {
    if single_file_creation::is_objective(lowered) {
        if single_file_creation::preserves_deterministic_draft(&draft) {
            draft.exit_condition =
                "Exit only after the exact requested file and its content digest are verified."
                    .to_string();
            draft.degraded_reason = None;
            return draft;
        }
        match grounded_create_file_step(objective, lowered) {
            Ok(step) => {
                if !single_file_creation::preserves_grounded_draft(&draft, &step) {
                    draft.steps = vec![step];
                    draft.source = IntentSource::Deterministic;
                }
                draft.exit_condition =
                    "Exit only after the exact requested file and its content digest are verified."
                        .to_string();
                draft.degraded_reason = None;
            }
            Err(deficit) => {
                draft.steps = vec![GeneratedPlanStepDraft {
                    step: deficit.clone(),
                    tool: GeneratedToolDraft::Unsupported { requested: deficit },
                    risk_level: GeneratedRiskLevel::Low,
                }];
                draft.exit_condition =
                    "Exit without requesting approval or writing until every missing file detail is supplied."
                        .to_string();
                draft.source = IntentSource::Deterministic;
                draft.degraded_reason = None;
            }
        }
        return draft;
    }
    if matches!(&draft.source, IntentSource::Degraded) {
        return draft;
    }

    if let Some(output_path) = telemetry_archive_output_path(objective, lowered) {
        if explicitly_requests_web_search(lowered) {
            repair_telemetry_archive_step(&mut draft, &output_path);
        } else {
            draft.steps = vec![telemetry_archive_plan_step(&output_path)];
        }
    }

    draft
}

fn file_creation_intent(lowered: &str) -> bool {
    let words = lowered
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<std::collections::HashSet<_>>();
    ["create", "make", "generate", "produce", "save", "write"]
        .iter()
        .any(|word| words.contains(word))
}

fn grounded_create_file_step(
    objective: &str,
    lowered: &str,
) -> Result<GeneratedPlanStepDraft, String> {
    let format = requested_file_format(lowered)
        .ok_or_else(|| "What format should the file use?".to_string())?;
    let content = requested_file_content(objective).or_else(|| {
        (lowered.contains("empty file") || lowered.contains("blank file")).then(String::new)
    });
    let destination_path = file_creation_destination::inferred_file_destination(
        objective,
        lowered,
        format,
        content.as_deref(),
    );
    let mut missing = Vec::new();
    if destination_path.is_none() {
        missing.push("its exact path and file name");
    }
    if content.is_none() {
        missing.push("what it should contain");
    }
    if !missing.is_empty() {
        return Err(format!("Please provide {}.", missing.join(" and ")));
    }
    let destination_path = destination_path.expect("checked destination");
    let content = content.expect("checked content");
    let title = Path::new(&destination_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Please provide an exact file name.".to_string())?
        .to_string();
    Ok(GeneratedPlanStepDraft {
        step: format!(
            "Create {} at {}.",
            format.to_ascii_uppercase(),
            destination_path
        ),
        tool: GeneratedToolDraft::RegisteredTaskTool {
            operation: "create_file".to_string(),
            arguments: serde_json::json!({"file":{
                "title":title,
                "content":content,
                "locale":"en-US",
                "format":format,
                "destinationPath":destination_path,
            }}),
        },
        risk_level: GeneratedRiskLevel::High,
    })
}

fn requested_file_format(lowered: &str) -> Option<&'static str> {
    file_formats::requested_file_formats(lowered)
        .into_iter()
        .next()
}

fn requested_file_content(objective: &str) -> Option<String> {
    for (open, close) in [('“', '”'), ('‘', '’'), ('"', '"'), ('\'', '\'')] {
        let mut search_from = 0;
        while let Some(relative_start) = objective[search_from..].find(open) {
            let start = search_from + relative_start + open.len_utf8();
            let Some(relative_end) = objective[start..].find(close) else {
                break;
            };
            let end = start + relative_end;
            let candidate = objective[start..end].trim();
            if !candidate.is_empty()
                && candidate.chars().count() <= 100_000
                && !candidate.starts_with('/')
                && !candidate.starts_with("~/")
            {
                return Some(candidate.to_string());
            }
            search_from = end + close.len_utf8();
        }
    }
    let lowered = objective.to_ascii_lowercase();
    lowered
        .contains("hello world")
        .then(|| "Hello World".to_string())
}

fn telemetry_archive_plan_step(output_path: &Path) -> GeneratedPlanStepDraft {
    GeneratedPlanStepDraft {
        step: format!(
            "Collect and package the requested local system audit into {}.",
            output_path.display()
        ),
        tool: GeneratedToolDraft::TelemetryArchive {
            output_path: output_path.display().to_string(),
        },
        risk_level: GeneratedRiskLevel::High,
    }
}

fn telemetry_archive_output_path(objective: &str, lowered: &str) -> Option<PathBuf> {
    if !(lowered.contains("telemetry") && lowered.contains(".tar.gz")) {
        return None;
    }
    let archive_path = objective
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|character: char| {
                matches!(
                    character,
                    '"' | '\'' | '`' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '.'
                )
            })
        })
        .find_map(|token| {
            let lowered = token.to_ascii_lowercase();
            lowered.ends_with(".tar.gz").then(|| PathBuf::from(token))
        })?;

    if archive_path.is_absolute() || archive_path.starts_with("~") {
        return Some(archive_path);
    }

    let is_bare_filename = archive_path.components().count() == 1;
    let names_workspace_testing_directory = lowered.contains("testing directory")
        || lowered.contains("testing folder")
        || lowered.contains("test directory")
        || lowered.contains("test folder");
    (is_bare_filename && names_workspace_testing_directory).then(|| {
        crate::shield_gate::development_repo_root()
            .join("planning")
            .join("testing")
            .join(archive_path)
    })
}

fn repair_telemetry_archive_step(draft: &mut GeneratedActionPlanDraft, output_path: &Path) {
    let output_path_text = output_path.display().to_string();
    let mut has_archive_step = false;

    for step in &mut draft.steps {
        match &mut step.tool {
            GeneratedToolDraft::TelemetryArchive { output_path } => {
                *output_path = output_path_text.clone();
                step.risk_level = GeneratedRiskLevel::High;
                has_archive_step = true;
            }
            GeneratedToolDraft::FileWrite { path, .. }
                if path.to_ascii_lowercase().ends_with(".tar.gz") =>
            {
                step.tool = GeneratedToolDraft::TelemetryArchive {
                    output_path: output_path_text.clone(),
                };
                step.risk_level = GeneratedRiskLevel::High;
                has_archive_step = true;
            }
            _ => {}
        }
    }

    if !has_archive_step {
        draft.steps.push(telemetry_archive_plan_step(output_path));
    }
}

fn fallback_plan(
    objective: String,
    source: IntentSource,
    degraded_reason: Option<String>,
    generated_text: String,
) -> GeneratedActionPlanDraft {
    let steps = vec![GeneratedPlanStepDraft {
        step: "Request clarification because no verified executable plan was produced.".to_string(),
        tool: GeneratedToolDraft::Unsupported {
            requested: format!("Clarification required for objective: {}", objective.trim()),
        },
        risk_level: GeneratedRiskLevel::Low,
    }];

    GeneratedActionPlanDraft {
        steps,
        exit_condition:
            "Exit after every generated step has completed or the Shield Gate halts execution."
                .to_string(),
        generated_text,
        source,
        degraded_reason,
    }
}

/// Stop-token ids declared by the checkpoint's generation_config.json.
/// Instruction-tuned Gemma 4 checkpoints list end-of-turn ids here (e.g. [1, 106, 50]);
/// base checkpoints list only the plain <eos> id.
fn generation_config_stop_ids(model_dir: &Path) -> Vec<u32> {
    let path = model_dir.join("generation_config.json");
    let Ok(raw) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    match value.get("eos_token_id") {
        Some(Value::Number(id)) => id.as_u64().map(|id| vec![id as u32]).unwrap_or_default(),
        Some(Value::Array(ids)) => ids
            .iter()
            .filter_map(Value::as_u64)
            .map(|id| id as u32)
            .collect(),
        _ => Vec::new(),
    }
}

fn local_model_label(id: &str) -> String {
    id.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn local_model_dir(id: &str) -> Result<PathBuf, GemmaError> {
    let model_root = env::var_os(LOCAL_MODEL_DIRECTORY_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(settings::resolved_local_model_directory_headless);
    local_model_dir_under_root(&model_root, id)
}

fn local_model_dir_under_root(model_root: &Path, id: &str) -> Result<PathBuf, GemmaError> {
    let id = id.trim();
    let valid = !id.is_empty()
        && !id.starts_with('.')
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });
    if !valid {
        return Err(GemmaError {
            code: "invalid_local_model_id",
            message: "Local model id must be a single models directory name.".to_string(),
        });
    }

    let root_contains_gguf = fs::read_dir(model_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| entry.path().is_file() && is_gguf_file(&entry.path()));
    if root_contains_gguf {
        let root_identity = identity_for_model_directory(model_root)?;
        let root_name_matches = model_root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|root_id| root_id.eq_ignore_ascii_case(id));
        if root_identity.canonical_id.eq_ignore_ascii_case(id)
            || (root_name_matches && model_identity::is_opaque_storage_reference(id))
        {
            return Ok(model_root.to_path_buf());
        }
    }

    Ok(model_root.join(id))
}

fn project_root() -> PathBuf {
    crate::settings::app_data_root()
}

impl GemmaError {
    fn io(operation: &'static str, error: std::io::Error) -> Self {
        Self {
            code: "gemma_io_error",
            message: format!("{operation} failed: {error}"),
        }
    }

    fn native_runtime(error: NativeRuntimeError) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}

impl GemmaInferenceConfig {
    fn low_latency() -> Self {
        let max_new_tokens = env::var("OOMU_MAX_NEW_TOKENS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(DEFAULT_MAX_NEW_TOKENS)
            .clamp(1, MAX_REQUEST_MAX_NEW_TOKENS);
        // These gemma4 QAT checkpoints (especially the 12B) have a peaky distribution that
        // frequently samples an immediate end-of-turn at high temperature, producing empty
        // responses. 0.4 keeps responses varied while sampling content reliably; override
        // with OOMU_TEMPERATURE if needed.
        let temperature = env::var("OOMU_TEMPERATURE")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.4)
            .clamp(0.0, 2.0);
        let top_k = env::var("OOMU_TOP_K")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(64)
            .clamp(1, 256);
        let top_p = env::var("OOMU_TOP_P")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.95)
            .clamp(0.05, 1.0);
        let repeat_penalty = env::var("OOMU_REPEAT_PENALTY")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(1.12)
            .clamp(1.0, 2.0);
        Self {
            max_new_tokens,
            temperature,
            top_k,
            top_p,
            repeat_penalty,
        }
    }
}

fn effective_max_new_tokens(request: &InferRequest, config: &GemmaInferenceConfig) -> usize {
    request
        .max_tokens
        .unwrap_or(config.max_new_tokens)
        .clamp(1, MAX_REQUEST_MAX_NEW_TOKENS)
}

fn should_log_local_inference_audit(request: &InferRequest) -> bool {
    !request.defer_audit
}

#[cfg(test)]
#[path = "gemma_spreadsheet_tests.rs"]
mod spreadsheet_tests;

#[cfg(test)]
#[path = "tests/gemma_artifact_creation.rs"]
mod artifact_creation_tests;

#[cfg(test)]
#[path = "tests/gemma.rs"]
mod tests;
