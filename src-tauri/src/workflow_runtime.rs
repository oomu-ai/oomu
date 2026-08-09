use crate::{
    db::{PersistenceEngine, WorkflowScheduleRecord},
    foundation::clock::unix_time_ms_i64 as unix_time_ms,
    gemma::{format_structured_runtime_prompt, GemmaService, InferRequest},
    knowledge::KnowledgeStore,
    mcp::client::{McpClientError, McpClientRegistry, McpToolApprovalBinding},
    mcp_result::McpToolCallResult,
    tool_security::{audit_workspace_execution_payload, classify_mcp_tool_call},
    workflow_ir::{
        AgentNode, CompiledInstruction, CompiledWorkflow, ConditionalNode, ExecutionInstance,
        ExecutionStatus, LoopNode, McpToolNode, NodeExecutionPayload, OutputNode,
        PermissionDeniedBehavior, RouterNode, WorkflowCompletionKind, WorkflowEdge, WorkflowIr,
        WorkflowNode,
    },
};
use rand_core::{OsRng, RngCore};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::Emitter;
mod approval_events;
pub(crate) mod evidence_input;
mod model_resolution;
mod native_mcp_execution;
mod native_notification;
mod scheduled_execution;
mod system_action;
mod task_tools;
pub(crate) use approval_events::dispatch_approval_request;
use model_resolution::resolved_gemma_runtime_model;
pub(crate) use scheduled_execution::retry_scheduled_workflow;
use scheduled_execution::{
    resolve_scheduled_project_context, scheduled_run_request, ScheduledExecutionContext,
};
use system_action::{execute_system_action_node, SystemActionFailureContext};
#[cfg(test)]
use system_action::{high_risk_action, run_system_action};
const LARGE_OUTPUT_BYTES: usize = 64 * 1024;
const APPLE_APP_WORKFLOW_TIMEOUT_MS: u64 = crate::workflow_ir::MEDIUM_TIMEOUT_MS;
const SYSTEM_CALENDAR_WORKFLOW_TIMEOUT_MS: u64 = 75_000;
const SYNC_KNOWLEDGE_VAULT_TOOL: &str = "sync_knowledge_vault";
const WORKFLOW_WORKSPACE_DIR: &str = "workspace";
const EMPTY_COMPLETION_MEDIA_TYPE: &str = "application/vnd.oomu.workflow-completion+json";
pub const OOMU_ENV_ALLOWLIST: &[&str] = &["LANG", "PATH", "USER", "TZ"];
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RunWorkflowRequest {
    pub workflow_id: String,
    #[serde(default)]
    pub workflow_version: Option<u32>,
    #[serde(default)]
    pub preflight_mode: WorkflowPreflightMode,
    #[serde(default)]
    pub inputs: HashMap<String, InputBinding>,
    #[serde(default)]
    pub outputs: HashMap<String, OutputBinding>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowPreflightMode {
    #[default]
    Skipped,
    TaskflowAudit,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum InputBinding {
    Manual { value: Value },
    LocalFile { path: String },
    Environment { name: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "destination", rename_all = "snake_case")]
pub enum OutputBinding {
    Ui,
    LocalDirectory {
        directory: String,
        #[serde(default)]
        file_name: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PayloadEnvelope {
    pub media_type: String,
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub asset_path: Option<String>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunWorkflowResponse {
    pub instance: ExecutionInstance,
    pub execution_order: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_request: Option<ApprovalRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion: Option<WorkflowCompletion>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCompletion {
    pub kind: WorkflowCompletionKind,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResolvePermissionRequest {
    pub instance_id: String,
    pub approval_token: String,
    pub decision: PermissionDecision,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Approve,
    Reject,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub instance_id: String,
    pub workflow_id: String,
    pub node_id: String,
    pub message: String,
    pub context: Value,
    pub approval_token: String,
    pub approve_command: Value,
    pub reject_command: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRuntimeError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
}

#[derive(Debug)]
struct ModelOutput {
    text: String,
    prompt_tokens: u64,
    completion_tokens: u64,
}

trait RuntimeModel: Clone + Send + Sync + 'static {
    fn execute_agent(
        &self,
        session_id: &str,
        system_prompt: &str,
        variables: &Map<String, Value>,
    ) -> Result<ModelOutput, WorkflowRuntimeError>;

    fn classify_route(
        &self,
        session_id: &str,
        router: &RouterNode,
        input: &Value,
    ) -> Result<ModelOutput, WorkflowRuntimeError>;

    fn evaluate_condition(
        &self,
        _session_id: &str,
        _conditional: &ConditionalNode,
        _input: &Value,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        Err(WorkflowRuntimeError::execution(
            "Conditional evaluation model is unavailable.".to_string(),
        ))
    }

    fn repair_system_action(
        &self,
        _session_id: &str,
        _failure: &SystemActionFailureContext,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        Err(WorkflowRuntimeError::execution(
            "System action self-healing model is unavailable.".to_string(),
        ))
    }
}

trait RuntimeExternalTools: Clone + Send + Sync + 'static {
    fn ensure_mcp_server_ready(
        &self,
        server_name: &str,
        timeout_ms: u64,
    ) -> Result<(), WorkflowRuntimeError>;

    fn prepare_mcp_tool_approval_binding(
        &self,
        _server_name: &str,
        _tool_name: &str,
        _arguments: Value,
        _timeout_ms: u64,
    ) -> Result<Option<McpToolApprovalBinding>, WorkflowRuntimeError> {
        Ok(None)
    }

    fn execute_mcp_tool(
        &self,
        execution_id: &str,
        node_id: &str,
        label: &str,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
        timeout_ms: u64,
        approval_binding: Option<McpToolApprovalBinding>,
        human_approved: bool,
    ) -> Result<Value, WorkflowRuntimeError>;

    fn execute_sync_knowledge_vault(
        &self,
        _arguments: Value,
    ) -> Result<Value, WorkflowRuntimeError> {
        Err(WorkflowRuntimeError::execution(
            "Knowledge vault sync is unavailable in this workflow runtime.".to_string(),
        ))
    }
}

// Bound from the user's generation setting, never from a classifier identity.
#[derive(Clone)]
struct GemmaRuntimeModel {
    gemma: GemmaService,
    model_id: String,
}

#[derive(Clone)]
struct McpRuntimeTools {
    registry: McpClientRegistry,
    persistence: PersistenceEngine,
    knowledge_tools: Option<KnowledgeRuntimeTools>,
    app: Option<tauri::AppHandle>,
}

#[derive(Clone)]
struct KnowledgeRuntimeTools;

impl RuntimeModel for GemmaRuntimeModel {
    fn execute_agent(
        &self,
        session_id: &str,
        system_prompt: &str,
        variables: &Map<String, Value>,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        let variables_json =
            serde_json::to_string(variables).map_err(WorkflowRuntimeError::serialization)?;
        let prompt = format_structured_runtime_prompt(
            system_prompt,
            &format!("Runtime variables:\n{variables_json}"),
        );
        self.infer(session_id, prompt)
    }

    fn classify_route(
        &self,
        session_id: &str,
        router: &RouterNode,
        input: &Value,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        let ports = router
            .routes
            .iter()
            .map(|route| format!("{}: {}", route.port, route.condition))
            .collect::<Vec<_>>()
            .join("\n");
        let system = "Select exactly one workflow route. Return only its port name with no prose.";
        let user = format!(
            "Expression: {}\nRoutes:\n{}\nIncoming JSON:\n{}",
            router.expression, ports, input
        );
        self.infer(session_id, format_structured_runtime_prompt(system, &user))
    }

    fn evaluate_condition(
        &self,
        session_id: &str,
        conditional: &ConditionalNode,
        input: &Value,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        let system = "Evaluate one workflow condition. Return only true or false with no prose.";
        let user = format!(
            "Condition: {}\nIncoming JSON or text:\n{}",
            conditional.condition, input
        );
        self.infer(session_id, format_structured_runtime_prompt(system, &user))
    }

    fn repair_system_action(
        &self,
        session_id: &str,
        failure: &SystemActionFailureContext,
    ) -> Result<ModelOutput, WorkflowRuntimeError> {
        let system = "Analyze one failed local system action. Return only compact JSON with keys command, args, workingDirectory, explanation. Keep the command low-risk and surgical.";
        let user = format!(
            "Action type: {:?}\nCommand: {}\nArgs: {}\nWorking directory: {}\nExit code: {:?}\nStdout:\n{}\nStderr:\n{}",
            failure.action_type,
            failure.command,
            serde_json::to_string(&failure.args).unwrap_or_else(|_| "[]".to_string()),
            failure.working_directory,
            failure.exit_code,
            failure.stdout,
            failure.stderr
        );
        self.infer(session_id, format_structured_runtime_prompt(system, &user))
    }
}

impl RuntimeExternalTools for McpRuntimeTools {
    fn ensure_mcp_server_ready(
        &self,
        server_name: &str,
        timeout_ms: u64,
    ) -> Result<(), WorkflowRuntimeError> {
        let registry = self.registry.clone();
        let server_name = server_name.to_string();
        let error_server_name = server_name.clone();
        let call = async move {
            tokio::time::timeout(
                Duration::from_millis(timeout_ms),
                registry.ensure_server_connected(&server_name),
            )
            .await
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle.block_on(call),
            Err(_) => tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| WorkflowRuntimeError::runtime(error.to_string()))?
                .block_on(call),
        }
        .map_err(|_| {
            WorkflowRuntimeError::mcp_server_unreachable(
                &error_server_name,
                format!("connection check timed out after {timeout_ms}ms"),
            )
        })?
        .map_err(|error| {
            WorkflowRuntimeError::mcp_server_unreachable(&error_server_name, error.message)
        })
    }

    fn execute_mcp_tool(
        &self,
        execution_id: &str,
        node_id: &str,
        label: &str,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
        timeout_ms: u64,
        approval_binding: Option<McpToolApprovalBinding>,
        human_approved: bool,
    ) -> Result<Value, WorkflowRuntimeError> {
        if task_tools::is_registered_task_server(server_name) {
            let app = self.app.as_ref().ok_or_else(|| {
                WorkflowRuntimeError::execution(
                    "Registered Workflow tools require the OOMU desktop app.".to_string(),
                )
            })?;
            return task_tools::execute_registered_task_tool(
                app,
                execution_id,
                node_id,
                label,
                tool_name,
                arguments,
                timeout_ms,
            );
        }
        if native_notification::is_tool(server_name, tool_name) {
            if let Some(app) = self.app.as_ref() {
                return native_notification::execute(app, execution_id, &arguments, human_approved);
            }
        }

        if is_native_calendar_workflow_tool(server_name, tool_name) {
            let node_id = node_id.to_string();
            let label = label.to_string();
            let registry = self.registry.clone();
            let app = self.app.clone();
            let call = async move {
                tokio::time::timeout(
                    Duration::from_millis(timeout_ms),
                    crate::mcp::client::read_system_calendar_for_workflow(
                        arguments,
                        Some(registry),
                        app,
                    ),
                )
                .await
            };
            let result = match tokio::runtime::Handle::try_current() {
                Ok(handle) => handle.block_on(call),
                Err(_) => tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| WorkflowRuntimeError::runtime(error.to_string()))?
                    .block_on(call),
            }
            .map_err(|_| WorkflowRuntimeError::node_timeout(&node_id, &label, timeout_ms))?
            .map_err(WorkflowRuntimeError::execution)?;

            if result.is_error {
                return Err(WorkflowRuntimeError::calendar_read(&result));
            }

            return serde_json::to_value(result).map_err(WorkflowRuntimeError::serialization);
        }

        let result = native_mcp_execution::execute_blocking(
            self.registry.clone(),
            self.persistence.clone(),
            execution_id,
            node_id,
            label,
            server_name,
            tool_name,
            arguments,
            timeout_ms,
            approval_binding,
            human_approved,
        )?;

        serde_json::to_value(result).map_err(WorkflowRuntimeError::serialization)
    }

    fn prepare_mcp_tool_approval_binding(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
        timeout_ms: u64,
    ) -> Result<Option<McpToolApprovalBinding>, WorkflowRuntimeError> {
        if task_tools::is_registered_task_server(server_name) {
            task_tools::validate_registered_task_arguments(tool_name, &arguments)?;
            return Ok(None);
        }
        if is_native_calendar_workflow_tool(server_name, tool_name)
            || native_notification::is_tool(server_name, tool_name)
        {
            return Ok(None);
        }

        let registry = self.registry.clone();
        let server_name = server_name.to_string();
        let error_server_name = server_name.clone();
        let tool_name = tool_name.to_string();
        let call = async move {
            tokio::time::timeout(
                Duration::from_millis(timeout_ms),
                registry.prepare_tool_approval_binding_for_review(
                    &server_name,
                    &tool_name,
                    arguments,
                ),
            )
            .await
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle.block_on(call),
            Err(_) => tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| WorkflowRuntimeError::runtime(error.to_string()))?
                .block_on(call),
        }
        .map_err(|_| {
            WorkflowRuntimeError::mcp_server_unreachable(
                &error_server_name,
                format!("permission review timed out after {timeout_ms}ms"),
            )
        })?
        .map_err(|error| workflow_error_from_mcp_client(&error_server_name, error))
    }

    fn execute_sync_knowledge_vault(
        &self,
        arguments: Value,
    ) -> Result<Value, WorkflowRuntimeError> {
        let Some(knowledge_tools) = &self.knowledge_tools else {
            return Err(WorkflowRuntimeError::execution(
                "Knowledge vault sync is unavailable in this workflow runtime.".to_string(),
            ));
        };
        knowledge_tools.execute_sync_knowledge_vault(arguments)
    }
}

impl KnowledgeRuntimeTools {
    fn execute_sync_knowledge_vault(
        &self,
        _arguments: Value,
    ) -> Result<Value, WorkflowRuntimeError> {
        Err(WorkflowRuntimeError::execution(
            "Knowledge vault sync requires an exact native picker grant and cannot run from a workflow path argument."
                .to_string(),
        ))
    }
}

fn workflow_error_from_mcp_client(
    server_name: &str,
    error: McpClientError,
) -> WorkflowRuntimeError {
    if error.code == "mcp_transport_error" || looks_like_mcp_transport_failure(&error.message) {
        return WorkflowRuntimeError::mcp_server_unreachable(server_name, error.message);
    }

    WorkflowRuntimeError::execution(error.message)
}

fn looks_like_mcp_transport_failure(message: &str) -> bool {
    let value = message.to_lowercase();
    [
        "broken pipe",
        "connection refused",
        "connection reset",
        "connection aborted",
        "connection closed",
        "econnrefused",
        "not connected",
        "socket",
        "timed out",
        "transport",
        "unreachable",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

impl GemmaRuntimeModel {
    fn infer(&self, session_id: &str, prompt: String) -> Result<ModelOutput, WorkflowRuntimeError> {
        let mut request = InferRequest::new(prompt);
        request.session_id = Some(session_id.to_string());
        request.deterministic = true;
        let response = self
            .gemma
            .infer_model_sync(&self.model_id, request)
            .map_err(WorkflowRuntimeError::inference)?;
        Ok(ModelOutput {
            completion_tokens: response.generated_token_count as u64,
            prompt_tokens: response.prompt_token_count as u64,
            text: response.text,
        })
    }
}

fn is_sync_knowledge_vault_mcp_tool(server_name: &str, tool_name: &str) -> bool {
    normalize_runtime_identifier(tool_name) == SYNC_KNOWLEDGE_VAULT_TOOL
        && matches!(
            normalize_runtime_identifier(server_name).as_str(),
            "system" | "native" | "local" | "oomu" | "oomu_system"
        )
}

fn normalize_runtime_identifier(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn require_durable_workflow_actuation(
    persistence: &PersistenceEngine,
    operation: &str,
) -> Result<(), WorkflowRuntimeError> {
    persistence
        .require_durable_store(operation)
        .map_err(|message| {
            WorkflowRuntimeError::new("workflow_volatile_persistence_blocked", message)
        })
}

#[tauri::command]
pub async fn run_workflow(
    request: RunWorkflowRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    gemma: tauri::State<'_, GemmaService>,
    _knowledge: tauri::State<'_, KnowledgeStore>,
    mcp_registry: tauri::State<'_, McpClientRegistry>,
    app: tauri::AppHandle,
) -> Result<RunWorkflowResponse, WorkflowRuntimeError> {
    require_durable_workflow_actuation(persistence.inner(), "compiled workflow actuation")?;
    let persistence = persistence.inner().clone();
    let gemma = gemma.inner().clone();
    let model = resolved_gemma_runtime_model(&app, gemma.clone())?;
    let external_tools = McpRuntimeTools {
        registry: mcp_registry.inner().clone(),
        persistence: persistence.clone(),
        knowledge_tools: Some(KnowledgeRuntimeTools),
        app: Some(app.clone()),
    };
    let progress_app = app.clone();
    let response = tauri::async_runtime::spawn_blocking(move || {
        run_persisted_workflow(
            request,
            &persistence,
            &model,
            &external_tools,
            &crate::settings::app_data_root().join("workflow-runs"),
            Some(progress_app),
            None,
        )
    })
    .await
    .map_err(|error| WorkflowRuntimeError::runtime(error.to_string()))??;
    dispatch_approval_request(&app, response.approval_request.as_ref());
    Ok(response)
}

pub(crate) async fn resolve_workflow_permission_without_reconciliation(
    request: ResolvePermissionRequest,
    persistence: PersistenceEngine,
    gemma: GemmaService,
    mcp_registry: McpClientRegistry,
    app: tauri::AppHandle,
) -> Result<RunWorkflowResponse, WorkflowRuntimeError> {
    require_durable_workflow_actuation(
        &persistence,
        "workflow permission resolution and actuation",
    )?;
    let model = resolved_gemma_runtime_model(&app, gemma.clone())?;
    let external_tools = McpRuntimeTools {
        registry: mcp_registry,
        persistence: persistence.clone(),
        knowledge_tools: Some(KnowledgeRuntimeTools),
        app: Some(app.clone()),
    };
    let progress_app = app.clone();
    let worker_persistence = persistence.clone();
    let response = tauri::async_runtime::spawn_blocking(move || {
        resolve_persisted_permission(
            request,
            &worker_persistence,
            &model,
            &external_tools,
            &crate::settings::app_data_root().join("workflow-runs"),
            Some(progress_app),
        )
    })
    .await
    .map_err(|error| WorkflowRuntimeError::runtime(error.to_string()))??;
    Ok(response)
}

#[tauri::command]
pub async fn list_pending_workflow_approvals(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<ApprovalRequest>, WorkflowRuntimeError> {
    let persistence = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || pending_workflow_approvals(&persistence))
        .await
        .map_err(|error| WorkflowRuntimeError::runtime(error.to_string()))?
}

fn pending_workflow_approvals(
    persistence: &PersistenceEngine,
) -> Result<Vec<ApprovalRequest>, WorkflowRuntimeError> {
    let connection = persistence
        .open_connection()
        .map_err(WorkflowRuntimeError::database)?;
    let mut statement = connection
        .prepare(
            "SELECT id FROM execution_instances WHERE status = 'AwaitingApproval' ORDER BY updated_at_ms ASC, id ASC",
        )
        .map_err(WorkflowRuntimeError::database)?;
    let instance_ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(WorkflowRuntimeError::database)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(WorkflowRuntimeError::database)?;

    instance_ids
        .into_iter()
        .map(|instance_id| {
            persistence
                .load_execution_instance(&instance_id)
                .map_err(WorkflowRuntimeError::database)
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|instances| {
            instances
                .iter()
                .filter_map(approval_request_from_instance)
                .collect()
        })
}

#[tauri::command]
pub async fn reveal_workflow_output_file(path: String) -> Result<(), WorkflowRuntimeError> {
    tauri::async_runtime::spawn_blocking(move || reveal_path_in_file_manager(&path))
        .await
        .map_err(|error| WorkflowRuntimeError::runtime(error.to_string()))?
}

pub fn run_scheduled_workflow(
    schedule: &WorkflowScheduleRecord,
    persistence: &PersistenceEngine,
    gemma: GemmaService,
    _knowledge: KnowledgeStore,
    mcp_registry: McpClientRegistry,
    app: tauri::AppHandle,
    workspace_root: &Path,
) -> Result<RunWorkflowResponse, WorkflowRuntimeError> {
    require_durable_workflow_actuation(persistence, "scheduled workflow actuation")?;
    let compiled = persistence
        .load_compiled_workflow(&schedule.workflow_id, schedule.workflow_version)
        .map_err(WorkflowRuntimeError::database)?;
    let capabilities =
        crate::workflow_ir::review::workflow_review_capabilities(&compiled.workflow_ir);
    let scheduled_context = resolve_scheduled_project_context(
        schedule,
        persistence,
        capabilities.project_file_read || capabilities.project_file_write,
        workspace_root,
    )?;
    let request = scheduled_run_request(schedule, &compiled, &scheduled_context)?;
    let model = resolved_gemma_runtime_model(&app, gemma)?;
    let external_tools = McpRuntimeTools {
        registry: mcp_registry,
        persistence: persistence.clone(),
        knowledge_tools: Some(KnowledgeRuntimeTools),
        app: Some(app.clone()),
    };
    run_persisted_workflow(
        request,
        persistence,
        &model,
        &external_tools,
        workspace_root,
        None,
        Some(&scheduled_context),
    )
}

pub(crate) fn resolve_scheduled_permission_without_reconciliation(
    request: ResolvePermissionRequest,
    persistence: &PersistenceEngine,
    gemma: GemmaService,
    mcp_registry: McpClientRegistry,
    app: tauri::AppHandle,
) -> Result<RunWorkflowResponse, WorkflowRuntimeError> {
    require_durable_workflow_actuation(persistence, "scheduled workflow permission resolution")?;
    let model = resolved_gemma_runtime_model(&app, gemma)?;
    let external_tools = McpRuntimeTools {
        registry: mcp_registry,
        persistence: persistence.clone(),
        knowledge_tools: Some(KnowledgeRuntimeTools),
        app: Some(app.clone()),
    };
    resolve_persisted_permission(
        request,
        persistence,
        &model,
        &external_tools,
        &crate::settings::app_data_root().join("workflow-runs"),
        None,
    )
}

fn run_persisted_workflow(
    request: RunWorkflowRequest,
    persistence: &PersistenceEngine,
    model: &impl RuntimeModel,
    external_tools: &impl RuntimeExternalTools,
    workspace_root: &Path,
    progress_app: Option<tauri::AppHandle>,
    scheduled_context: Option<&ScheduledExecutionContext>,
) -> Result<RunWorkflowResponse, WorkflowRuntimeError> {
    validate_request(&request)?;
    let compiled = persistence
        .load_compiled_workflow(&request.workflow_id, request.workflow_version)
        .map_err(WorkflowRuntimeError::database)?;
    let mut instance = new_instance(&compiled.workflow_ir, &request)?;
    if let Some(context) = scheduled_context {
        persistence
            .insert_scheduled_execution_instance(
                &instance,
                &context.project_id,
                &context.schedule_id,
                context.scheduled_for_ms,
            )
            .map_err(WorkflowRuntimeError::database)?;
    } else {
        persistence
            .insert_direct_workflow_execution_instance(&instance)
            .map_err(WorkflowRuntimeError::database)?;
    }

    let mut checkpoint = |current: &ExecutionInstance| {
        persistence
            .update_execution_instance(current)
            .map_err(WorkflowRuntimeError::database)
    };
    let mut progress = |current: &ExecutionInstance,
                        node_id: &str,
                        step_index: usize,
                        status: &str,
                        message: &str| {
        if let Some(app) = &progress_app {
            dispatch_workflow_progress(app, current, node_id, step_index, status, message);
        }
    };
    let mut result = execute_workflow(
        &compiled,
        &request,
        model,
        external_tools,
        workspace_root,
        &mut instance,
        &mut checkpoint,
        &mut progress,
        Some(persistence),
        None,
    );
    if let Err(error) = &mut result {
        if instance.status != ExecutionStatus::AwaitingApproval {
            instance.status = ExecutionStatus::Failed;
            instance.error = Some(json!({ "code": error.code, "message": error.message }));
            instance.active_node_id = None;
            finish_timing(&mut instance, true);
        }
    }
    persistence
        .update_execution_instance(&instance)
        .map_err(WorkflowRuntimeError::database)?;
    match result {
        Ok(outcome) => Ok(run_workflow_response(
            instance,
            outcome.execution_order,
            outcome.approval_request,
        )),
        Err(_) => {
            let execution_order = recorded_execution_order(&compiled.workflow_ir, &instance);
            Ok(run_workflow_response(instance, execution_order, None))
        }
    }
}

fn resolve_persisted_permission(
    request: ResolvePermissionRequest,
    persistence: &PersistenceEngine,
    model: &impl RuntimeModel,
    external_tools: &impl RuntimeExternalTools,
    workspace_root: &Path,
    progress_app: Option<tauri::AppHandle>,
) -> Result<RunWorkflowResponse, WorkflowRuntimeError> {
    validate_approval_request(&request)?;
    let awaiting = persistence
        .load_execution_instance(&request.instance_id)
        .map_err(WorkflowRuntimeError::database)?;
    verify_approval_token(&awaiting, &request.approval_token)?;
    let mut instance = persistence
        .claim_execution_instance_for_approval(&request.instance_id)
        .map_err(|_| WorkflowRuntimeError::approval_consumed())?;
    let run_request: RunWorkflowRequest = serde_json::from_value(instance.input_payload.clone())
        .map_err(WorkflowRuntimeError::serialization)?;
    let compiled = persistence
        .load_compiled_workflow(&instance.workflow_id, Some(instance.workflow_version))
        .map_err(WorkflowRuntimeError::database)?;
    let permission_node_id = instance
        .active_node_id
        .clone()
        .ok_or_else(WorkflowRuntimeError::approval_state_invalid)?;
    task_tools::record_bound_mcp_approval(&request, &instance, persistence, &permission_node_id)?;
    let mut checkpoint = |current: &ExecutionInstance| {
        persistence
            .update_execution_instance(current)
            .map_err(WorkflowRuntimeError::database)
    };
    let mut progress = |current: &ExecutionInstance,
                        node_id: &str,
                        step_index: usize,
                        status: &str,
                        message: &str| {
        if let Some(app) = &progress_app {
            dispatch_workflow_progress(app, current, node_id, step_index, status, message);
        }
    };
    let mut result = execute_workflow(
        &compiled,
        &run_request,
        model,
        external_tools,
        workspace_root,
        &mut instance,
        &mut checkpoint,
        &mut progress,
        Some(persistence),
        Some(ResumePermission {
            node_id: permission_node_id,
            decision: request.decision,
        }),
    );
    if let Err(error) = &mut result {
        instance.status = ExecutionStatus::Failed;
        instance.error = Some(json!({ "code": error.code, "message": error.message }));
        instance.active_node_id = None;
        finish_timing(&mut instance, true);
    }
    persistence
        .update_execution_instance(&instance)
        .map_err(WorkflowRuntimeError::database)?;
    match result {
        Ok(outcome) => Ok(run_workflow_response(
            instance,
            outcome.execution_order,
            outcome.approval_request,
        )),
        Err(_) => {
            let execution_order = recorded_execution_order(&compiled.workflow_ir, &instance);
            Ok(run_workflow_response(instance, execution_order, None))
        }
    }
}

#[derive(Debug)]
struct ExecutionOutcome {
    execution_order: Vec<String>,
    approval_request: Option<ApprovalRequest>,
}

fn completed_empty_envelope() -> Value {
    json!({
        "mediaType": EMPTY_COMPLETION_MEDIA_TYPE,
        "data": {
            "kind": "empty_collection",
            "items": [],
        },
        "assetPath": null,
        "metadata": {
            "completionKind": "empty_collection",
        }
    })
}

fn is_completed_empty_envelope(value: &Value) -> bool {
    value.get("mediaType").and_then(Value::as_str) == Some(EMPTY_COMPLETION_MEDIA_TYPE)
        && value.pointer("/data/kind").and_then(Value::as_str) == Some("empty_collection")
        && value
            .pointer("/data/items")
            .and_then(Value::as_array)
            .is_some_and(|items| items.is_empty())
        && value
            .pointer("/metadata/completionKind")
            .and_then(Value::as_str)
            == Some("empty_collection")
}

fn run_workflow_response(
    instance: ExecutionInstance,
    execution_order: Vec<String>,
    approval_request: Option<ApprovalRequest>,
) -> RunWorkflowResponse {
    let completion = (instance.status == ExecutionStatus::Completed)
        .then(|| {
            instance
                .output_payload
                .as_ref()
                .filter(|payload| is_completed_empty_envelope(payload))
                .map(|_| WorkflowCompletion {
                    kind: WorkflowCompletionKind::EmptyCollection,
                })
        })
        .flatten();
    RunWorkflowResponse {
        instance,
        execution_order,
        approval_request,
        completion,
    }
}

fn recorded_execution_order(ir: &WorkflowIr, instance: &ExecutionInstance) -> Vec<String> {
    // Preserve the stable order of durably checkpointed failed runs.
    topological_sort(ir)
        .unwrap_or_else(|_| {
            let mut node_ids = instance.node_payloads.keys().cloned().collect::<Vec<_>>();
            node_ids.sort();
            node_ids
        })
        .into_iter()
        .filter(|node_id| instance.node_payloads.contains_key(node_id))
        .collect()
}

struct ResumePermission {
    node_id: String,
    decision: PermissionDecision,
}

fn execute_workflow(
    compiled: &CompiledWorkflow,
    request: &RunWorkflowRequest,
    model: &impl RuntimeModel,
    external_tools: &impl RuntimeExternalTools,
    workspace_root: &Path,
    instance: &mut ExecutionInstance,
    checkpoint: &mut impl FnMut(&ExecutionInstance) -> Result<(), WorkflowRuntimeError>,
    progress: &mut impl FnMut(&ExecutionInstance, &str, usize, &str, &str),
    approval_ledger: Option<&PersistenceEngine>,
    resume_permission: Option<ResumePermission>,
) -> Result<ExecutionOutcome, WorkflowRuntimeError> {
    compiled
        .workflow_ir
        .validate()
        .map_err(WorkflowRuntimeError::invalid_ir)?;
    let order = topological_sort(&compiled.workflow_ir)?;
    let node_by_id = compiled
        .workflow_ir
        .nodes
        .iter()
        .map(|node| (node.id(), node))
        .collect::<HashMap<_, _>>();
    if let Err(error) = ensure_workflow_sandbox_ready() {
        fail_instance_before_execution(instance, &error, checkpoint)?;
        return Err(error);
    }
    let incoming = edges_by_target(&compiled.workflow_ir.edges);
    let outgoing = edges_by_source(&compiled.workflow_ir.edges);
    let mut selected_edges = instance.selected_edges.clone();
    let mut memory = instance.memory.clone();
    let mut executed = Vec::new();
    instance.status = ExecutionStatus::Running;
    instance.started_at_ms.get_or_insert_with(unix_time_ms);
    instance.pause_context = None;
    instance.updated_at_ms = unix_time_ms();
    checkpoint(instance)?;

    let mut loop_managed_nodes = HashSet::new();
    let mut ready_mcp_servers = HashSet::new();

    for (step_index, node_id) in order.iter().enumerate() {
        let node = node_by_id[&node_id.as_str()];
        if loop_managed_nodes.contains(node.id()) {
            continue;
        }
        if instance
            .node_payloads
            .get(node.id())
            .is_some_and(|payload| payload.status == ExecutionStatus::Completed)
        {
            continue;
        }
        let is_input = matches!(node, WorkflowNode::Input(_));
        let reachable = is_input
            || incoming
                .get(node.id())
                .is_some_and(|edges| edges.iter().any(|edge| selected_edges.contains(&edge.id)));
        if !reachable {
            continue;
        }
        instance.active_node_id = Some(node.id().to_string());
        let started = Instant::now();
        let input = incoming_payload(
            node.id(),
            &incoming,
            &selected_edges,
            &instance.node_payloads,
        );
        progress(
            instance,
            node.id(),
            step_index,
            "running",
            "Running workflow node.",
        );
        let node_step = match node {
            WorkflowNode::Input(input_node) => {
                let binding = request.inputs.get(input_node.id.as_str()).ok_or_else(|| {
                    WorkflowRuntimeError::input(format!(
                        "Input node {} requires a per-run input binding.",
                        input_node.id
                    ))
                })?;
                let envelope = resolve_input(binding)?;
                memory.insert(input_node.output_key.clone(), envelope.clone());
                memory.insert("workflow.input".to_string(), envelope.clone());
                Ok(ActionNodeStep::Completed(NodeOutcome::output(
                    envelope,
                    vec!["out".to_string()],
                )))
            }
            WorkflowNode::Agent(agent) => execute_agent_with_timeout(
                agent,
                compiled.instructions.get(&agent.id),
                model,
                &memory,
                workspace_root,
                &instance.id,
            )
            .map(ActionNodeStep::Completed),
            WorkflowNode::Router(router) => {
                execute_router_with_timeout(router, model, input.as_ref(), &memory, &instance.id)
                    .map(ActionNodeStep::Completed)
            }
            WorkflowNode::Conditional(conditional) => execute_conditional_with_timeout(
                conditional,
                model,
                input.as_ref(),
                &memory,
                &instance.id,
            )
            .map(ActionNodeStep::Completed),
            WorkflowNode::Loop(loop_node) => {
                let loop_result = execute_loop_node(
                    loop_node,
                    &order,
                    &node_by_id,
                    &incoming,
                    &outgoing,
                    compiled,
                    model,
                    external_tools,
                    workspace_root,
                    instance,
                    input.clone(),
                    &memory,
                    &selected_edges,
                    step_index,
                    approval_ledger,
                    &mut ready_mcp_servers,
                    progress,
                );
                match loop_result {
                    Ok(loop_result) => {
                        for body_node_id in &loop_result.body_node_ids {
                            loop_managed_nodes.insert(body_node_id.clone());
                        }
                        executed.extend(loop_result.execution_order);
                        memory = loop_result.memory;
                        Ok(ActionNodeStep::Completed(loop_result.outcome))
                    }
                    Err(error) => Err(error),
                }
            }
            WorkflowNode::McpTool(mcp_tool) => {
                let approved_permission_node = task_tools::approved_permission_predecessor(
                    mcp_tool,
                    &incoming,
                    &selected_edges,
                    &node_by_id,
                    &instance.node_payloads,
                );
                execute_mcp_tool_node(
                    mcp_tool,
                    external_tools,
                    instance,
                    input.clone(),
                    &memory,
                    &selected_edges,
                    elapsed_ms(started),
                    approval_ledger,
                    resume_permission.as_ref(),
                    approved_permission_node.as_deref(),
                    &mut ready_mcp_servers,
                )
            }
            WorkflowNode::SystemAction(system_action) => execute_system_action_node(
                system_action,
                model,
                external_tools,
                workspace_root,
                instance,
                input.clone(),
                &memory,
                &selected_edges,
                elapsed_ms(started),
                resume_permission.as_ref(),
            ),
            WorkflowNode::Permission(permission) => {
                if let Some(resume) = resume_permission
                    .as_ref()
                    .filter(|resume| resume.node_id == permission.id)
                {
                    if resume.decision == PermissionDecision::Reject
                        && matches!(permission.on_denied, PermissionDeniedBehavior::Fail)
                    {
                        Err(WorkflowRuntimeError::permission_rejected(
                            &permission.reason,
                        ))
                    } else {
                        let port = if resume.decision == PermissionDecision::Approve {
                            "approved"
                        } else {
                            "denied"
                        };
                        Ok(ActionNodeStep::Completed(NodeOutcome::output(
                            json!({
                                "mediaType": "application/json",
                                "data": {
                                    "decision": resume.decision,
                                    "message": permission.reason,
                                },
                                "assetPath": null,
                                "metadata": {}
                            }),
                            vec![port.to_string()],
                        )))
                    }
                } else {
                    let exact_effect = task_tools::exact_permission_effect_context(
                        permission,
                        &outgoing,
                        &node_by_id,
                        &memory,
                    )?;
                    pause_for_permission(
                        instance,
                        permission,
                        exact_effect,
                        input.clone(),
                        &memory,
                        &selected_edges,
                        elapsed_ms(started),
                    )
                    .map(ActionNodeStep::Paused)
                }
            }
            WorkflowNode::Output(output) => execute_output(
                output,
                request.outputs.get(&output.id),
                &memory,
                &instance.id,
                &compiled.workflow_ir,
            )
            .map(ActionNodeStep::Completed),
        };
        let outcome = match node_step {
            Ok(ActionNodeStep::Completed(outcome)) => outcome,
            Ok(ActionNodeStep::Paused(approval)) => {
                progress(
                    instance,
                    node.id(),
                    step_index,
                    "halted",
                    "Awaiting workflow approval.",
                );
                checkpoint(instance)?;
                return Ok(ExecutionOutcome {
                    execution_order: executed,
                    approval_request: Some(approval),
                });
            }
            Err(error) => {
                let message = error.message.clone();
                instance.node_payloads.insert(
                    node.id().to_string(),
                    NodeExecutionPayload {
                        status: ExecutionStatus::Failed,
                        input: input.clone(),
                        output: None,
                        error: Some(json!({
                            "code": error.code,
                            "boundary": error.boundary,
                            "message": message,
                        })),
                        latency_ms: elapsed_ms(started),
                        prompt_tokens: 0,
                        completion_tokens: 0,
                    },
                );
                instance.status = ExecutionStatus::Failed;
                instance.updated_at_ms = unix_time_ms();
                checkpoint(instance)?;
                progress(instance, node.id(), step_index, "halted", &error.message);
                return Err(error);
            }
        };
        let completes_empty = outcome.completion_kind == WorkflowCompletionKind::EmptyCollection;
        let latency_ms = elapsed_ms(started);
        instance.prompt_tokens += outcome.prompt_tokens;
        instance.completion_tokens += outcome.completion_tokens;
        instance.total_tokens = instance.prompt_tokens + instance.completion_tokens;
        instance.node_payloads.insert(
            node.id().to_string(),
            NodeExecutionPayload {
                status: ExecutionStatus::Completed,
                input,
                output: Some(outcome.output.clone()),
                error: None,
                latency_ms,
                prompt_tokens: outcome.prompt_tokens,
                completion_tokens: outcome.completion_tokens,
            },
        );
        memory.insert(
            format!("nodes.{}.output", node.id()),
            outcome.output.clone(),
        );
        memory.insert(format!("{}.output", node.id()), outcome.output.clone());
        if let WorkflowNode::Agent(agent) = node {
            memory.insert(agent.output_key.clone(), outcome.output.clone());
        }
        let legacy_empty = if completes_empty {
            None
        } else {
            legacy_unguarded_empty_completion(
                compiled,
                node.id(),
                &memory,
                &incoming,
                &outgoing,
                &node_by_id,
            )
        };
        let terminal_empty = completes_empty || legacy_empty.is_some();
        if terminal_empty {
            let terminal_output = if completes_empty {
                outcome.output.clone()
            } else {
                completed_empty_envelope()
            };
            memory.insert("workflow.output".to_string(), terminal_output.clone());
            instance.output_payload = Some(terminal_output);
        } else {
            if !matches!(node, WorkflowNode::Output(_)) {
                memory.insert("workflow.output".to_string(), outcome.output.clone());
            }
            if matches!(node, WorkflowNode::Output(_)) {
                instance.output_payload = Some(outcome.output.clone());
            }
        }
        if !terminal_empty {
            for edge in outgoing.get(node.id()).into_iter().flatten() {
                if outcome.ports.iter().any(|port| port == &edge.source_port) {
                    selected_edges.insert(edge.id.clone());
                }
            }
        }
        executed.push(node.id().to_string());
        instance.memory = memory.clone();
        instance.selected_edges = selected_edges.clone();
        instance.updated_at_ms = unix_time_ms();
        if terminal_empty {
            instance.status = ExecutionStatus::Completed;
            instance.active_node_id = None;
            instance.pause_context = None;
            instance.error = None;
            finish_timing(instance, true);
        }
        checkpoint(instance)?;
        let progress_message = legacy_empty.as_ref().map_or_else(
            || "Workflow node completed.".to_string(),
            |empty| {
                format!(
                    "Nothing matched: {} is empty, so {} and downstream node {} were skipped.",
                    empty.collection_reference, empty.reference, empty.consumer_node_id
                )
            },
        );
        progress(
            instance,
            node.id(),
            step_index,
            "success",
            &progress_message,
        );
        if terminal_empty {
            return Ok(ExecutionOutcome {
                execution_order: executed,
                approval_request: None,
            });
        }
    }

    if instance.output_payload.is_none() {
        return Err(WorkflowRuntimeError::execution(
            "No reachable Output node completed.".to_string(),
        ));
    }
    instance.status = ExecutionStatus::Completed;
    instance.active_node_id = None;
    finish_timing(instance, true);
    Ok(ExecutionOutcome {
        execution_order: executed,
        approval_request: None,
    })
}

#[derive(Debug)]
struct NodeOutcome {
    output: Value,
    ports: Vec<String>,
    prompt_tokens: u64,
    completion_tokens: u64,
    completion_kind: WorkflowCompletionKind,
}

enum ActionNodeStep {
    Completed(NodeOutcome),
    Paused(ApprovalRequest),
}

struct LoopExecutionResult {
    outcome: NodeOutcome,
    body_node_ids: HashSet<String>,
    execution_order: Vec<String>,
    memory: HashMap<String, Value>,
}

impl NodeOutcome {
    fn output(output: Value, ports: Vec<String>) -> Self {
        Self {
            output,
            ports,
            prompt_tokens: 0,
            completion_tokens: 0,
            completion_kind: WorkflowCompletionKind::Result,
        }
    }

    fn empty_collection() -> Self {
        Self {
            output: completed_empty_envelope(),
            ports: Vec::new(),
            prompt_tokens: 0,
            completion_tokens: 0,
            completion_kind: WorkflowCompletionKind::EmptyCollection,
        }
    }
}

fn ensure_workflow_sandbox_ready() -> Result<(), WorkflowRuntimeError> {
    crate::mcp::bootstrap::ensure_default_mcp_sandbox_dir()
        .map(|_| ())
        .map_err(WorkflowRuntimeError::mcp_sandbox)
}

fn fail_instance_before_execution(
    instance: &mut ExecutionInstance,
    error: &WorkflowRuntimeError,
    checkpoint: &mut impl FnMut(&ExecutionInstance) -> Result<(), WorkflowRuntimeError>,
) -> Result<(), WorkflowRuntimeError> {
    instance.status = ExecutionStatus::Failed;
    instance.active_node_id = None;
    instance.error = Some(json!({
        "code": error.code,
        "boundary": error.boundary,
        "message": error.message,
    }));
    finish_timing(instance, true);
    checkpoint(instance)
}

fn execute_mcp_tool_node(
    mcp_tool: &McpToolNode,
    external_tools: &impl RuntimeExternalTools,
    instance: &mut ExecutionInstance,
    input: Option<Value>,
    memory: &HashMap<String, Value>,
    selected_edges: &HashSet<String>,
    latency_ms: u64,
    approval_ledger: Option<&PersistenceEngine>,
    resume_permission: Option<&ResumePermission>,
    approved_permission_node: Option<&str>,
    ready_mcp_servers: &mut HashSet<String>,
) -> Result<ActionNodeStep, WorkflowRuntimeError> {
    let arguments = normalize_mcp_text_writer_arguments(
        &mcp_tool.tool_name,
        resolve_json_templates(&mcp_tool.arguments, memory)?,
    );
    let timeout_ms = mcp_tool_timeout_ms(mcp_tool);
    let audit_payload = json!({
        "actionType": "mcp_tool",
        "serverName": &mcp_tool.server_name,
        "toolName": &mcp_tool.tool_name,
        "arguments": &arguments,
    })
    .to_string();
    audit_workspace_execution_payload(&audit_payload)
        .map_err(|violation| WorkflowRuntimeError::execution(violation.message))?;
    if is_sync_knowledge_vault_mcp_tool(&mcp_tool.server_name, &mcp_tool.tool_name) {
        return execute_sync_knowledge_vault_node(
            &mcp_tool.id,
            &mcp_tool.label,
            external_tools,
            arguments,
            timeout_ms,
            json!({
                "serverName": mcp_tool.server_name,
                "toolName": mcp_tool.tool_name,
            }),
        );
    }
    let classification = classify_mcp_tool_call(&mcp_tool.server_name, &mcp_tool.tool_name, None);
    ensure_mcp_server_ready_once(mcp_tool, external_tools, ready_mcp_servers)?;
    let approval_binding = external_tools.prepare_mcp_tool_approval_binding(
        &mcp_tool.server_name,
        &mcp_tool.tool_name,
        arguments.clone(),
        timeout_ms,
    )?;
    let requires_review = classification.requires_human_approval() || approval_binding.is_some();
    let workflow_version_approval_material = requires_review
        .then(|| {
            task_tools::workflow_version_mcp_approval_material(
                mcp_tool,
                &arguments,
                approval_binding.as_ref(),
            )
        })
        .transpose()?
        .flatten();
    let reviewed_routine_scope = task_tools::reviewed_routine_scope_for_call(
        approval_ledger,
        &instance.id,
        mcp_tool,
        &arguments,
    )?;
    #[cfg(test)]
    let test_auto_approved = crate::tool_security::auto_approve_mcp_test_enabled()
        && classification.tier.requires_human_approval()
        && !approval_binding
            .as_ref()
            .is_some_and(|binding| binding.requires_native_shield);
    #[cfg(not(test))]
    let test_auto_approved = false;
    let mut human_approved = test_auto_approved;
    if !test_auto_approved && requires_review {
        let approval_material =
            task_tools::workflow_mcp_approval_material(&arguments, approval_binding.as_ref());
        match resume_decision_for_node(&mcp_tool.id, resume_permission) {
            Some(PermissionDecision::Approve) => {
                if let Some(approval_ledger) = approval_ledger {
                    let approved = approval_ledger
                        .verify_workflow_approval(
                            &instance.id,
                            &mcp_tool.id,
                            &mcp_tool.tool_name,
                            &approval_material,
                        )
                        .map_err(WorkflowRuntimeError::database)?;
                    if !approved {
                        return Err(WorkflowRuntimeError::approval_not_verified(
                            &mcp_tool.id,
                            &mcp_tool.tool_name,
                        ));
                    }
                }
                human_approved = true;
            }
            Some(PermissionDecision::Reject) => {
                return Err(WorkflowRuntimeError::permission_rejected(&format!(
                    "MCP tool {} / {} was rejected.",
                    mcp_tool.server_name, mcp_tool.tool_name
                )));
            }
            None => {
                let ledger_approved = match approval_ledger {
                    Some(approval_ledger) => {
                        let workflow_approved = approval_ledger
                            .verify_workflow_approval(
                                &instance.id,
                                &mcp_tool.id,
                                &mcp_tool.tool_name,
                                &approval_material,
                            )
                            .map_err(WorkflowRuntimeError::database)?;
                        let workflow_version_approved =
                            match workflow_version_approval_material.as_ref() {
                                Some(material) => approval_ledger
                                    .verify_workflow_version_approval(
                                        &instance.workflow_id,
                                        instance.workflow_version,
                                        &mcp_tool.id,
                                        &mcp_tool.server_name,
                                        &mcp_tool.tool_name,
                                        material,
                                    )
                                    .map_err(WorkflowRuntimeError::database)?,
                                None => false,
                            };
                        if workflow_approved || workflow_version_approved {
                            true
                        } else if let Some(permission_node_id) = approved_permission_node {
                            task_tools::verify_predecessor_mcp_approval(
                                approval_ledger,
                                &instance.id,
                                permission_node_id,
                                mcp_tool,
                                &arguments,
                                approval_binding.as_ref(),
                            )?
                        } else if reviewed_routine_scope {
                            true
                        } else if routine_authority_can_satisfy_mcp_review(
                            approval_binding.as_ref(),
                        ) {
                            approval_ledger
                                .verify_routine_authority(
                                    &instance.id,
                                    &mcp_tool.tool_name,
                                    &arguments,
                                )
                                .map_err(WorkflowRuntimeError::database)?
                        } else {
                            false
                        }
                    }
                    None => false,
                };
                if ledger_approved {
                    human_approved = true;
                } else {
                    let approval = pause_for_external_action(
                        instance,
                        &mcp_tool.id,
                        &format!(
                            "Approve MCP tool {} / {}",
                            mcp_tool.server_name, mcp_tool.tool_name
                        ),
                        json!({
                            "actionType": "mcp_tool",
                            "serverName": mcp_tool.server_name,
                            "toolName": mcp_tool.tool_name,
                            "arguments": arguments,
                            "capabilityRiskTier": classification.tier.as_str(),
                            "capabilityReason": classification.reason.clone(),
                            "mcpApprovalBinding": approval_binding.clone(),
                            "approvalReuse": workflow_version_approval_material.as_ref().map(|_| json!({
                                "scope": "workflow_version",
                                "workflowVersion": instance.workflow_version,
                            })),
                            "timeoutMs": timeout_ms,
                            "systemTimeoutMs": timeout_ms,
                            "input": input.clone(),
                            "memoryKeys": memory.keys().collect::<Vec<_>>(),
                        }),
                        input,
                        memory,
                        selected_edges,
                        latency_ms,
                    )?;
                    return Ok(ActionNodeStep::Paused(approval));
                }
            }
        }
    }

    let external_tools = (*external_tools).clone();
    let execution_id = instance.id.clone();
    let node_id = mcp_tool.id.clone();
    let label = mcp_tool.label.clone();
    let worker_node_id = node_id.clone();
    let worker_label = label.clone();
    let server_name = mcp_tool.server_name.clone();
    let tool_name = mcp_tool.tool_name.clone();
    let execution_arguments = arguments.clone();
    let result = run_blocking_node_with_timeout(&node_id, &label, timeout_ms, move || {
        external_tools.execute_mcp_tool(
            &execution_id,
            &worker_node_id,
            &worker_label,
            &server_name,
            &tool_name,
            execution_arguments,
            timeout_ms,
            approval_binding,
            human_approved,
        )
    })?;
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(WorkflowRuntimeError::execution(format!(
            "MCP tool {} / {} returned an error result: {}",
            mcp_tool.server_name, mcp_tool.tool_name, result
        )));
    }

    Ok(ActionNodeStep::Completed(NodeOutcome::output(
        mcp_result_envelope(
            &mcp_tool.server_name,
            &mcp_tool.tool_name,
            arguments,
            result,
        ),
        vec!["out".to_string()],
    )))
}

fn mcp_result_envelope(
    server_name: &str,
    tool_name: &str,
    arguments: Value,
    result: Value,
) -> Value {
    let mut metadata = Map::new();
    metadata.insert(
        "serverName".to_string(),
        Value::String(server_name.to_string()),
    );
    metadata.insert("toolName".to_string(), Value::String(tool_name.to_string()));
    metadata.insert("arguments".to_string(), arguments);
    if task_tools::is_registered_task_server(server_name) {
        metadata.insert(
            "operation".to_string(),
            Value::String(tool_name.to_string()),
        );
        metadata.insert("verified".to_string(), Value::Bool(true));
    }
    json!({
        "mediaType": "application/json",
        "data": result,
        "assetPath": null,
        "metadata": Value::Object(metadata),
    })
}

#[cfg(test)]
mod registered_task_result_envelope_tests {
    use super::*;

    #[test]
    fn registered_task_verification_is_transport_metadata_not_domain_data() {
        let envelope = mcp_result_envelope(
            task_tools::SERVER_NAME,
            "analyze_supplier_exceptions",
            json!({"content":"fixture bytes"}),
            json!({"supplierCount":1,"hasException":false}),
        );

        assert_eq!(
            envelope.pointer("/metadata/operation"),
            Some(&json!("analyze_supplier_exceptions"))
        );
        assert_eq!(envelope.pointer("/metadata/verified"), Some(&json!(true)));
        assert!(envelope.pointer("/data/operation").is_none());
        assert!(envelope.pointer("/data/verified").is_none());
    }

    #[test]
    fn ordinary_mcp_results_do_not_claim_registered_task_verification() {
        let envelope = mcp_result_envelope(
            "external_server",
            "read_items",
            json!({}),
            json!({"items":[]}),
        );

        assert!(envelope.pointer("/metadata/operation").is_none());
        assert!(envelope.pointer("/metadata/verified").is_none());
    }
}

fn ensure_mcp_server_ready_once(
    mcp_tool: &McpToolNode,
    external_tools: &impl RuntimeExternalTools,
    ready_mcp_servers: &mut HashSet<String>,
) -> Result<(), WorkflowRuntimeError> {
    if task_tools::is_registered_task_server(&mcp_tool.server_name)
        || is_native_calendar_workflow_tool(&mcp_tool.server_name, &mcp_tool.tool_name)
        || native_notification::is_tool(&mcp_tool.server_name, &mcp_tool.tool_name)
        || is_sync_knowledge_vault_mcp_tool(&mcp_tool.server_name, &mcp_tool.tool_name)
        || ready_mcp_servers.contains(&mcp_tool.server_name)
    {
        return Ok(());
    }
    external_tools.ensure_mcp_server_ready(&mcp_tool.server_name, mcp_tool_timeout_ms(mcp_tool))?;
    ready_mcp_servers.insert(mcp_tool.server_name.clone());
    Ok(())
}

fn normalize_mcp_text_writer_arguments(tool_name: &str, arguments: Value) -> Value {
    if !matches!(tool_name, "write_markdown_report" | "write_file") {
        return arguments;
    }
    let mut arguments = arguments;
    let Value::Object(object) = &mut arguments else {
        return arguments;
    };

    for key in ["content", "report", "markdown", "body", "text"] {
        let Some(content) = object.get(key).and_then(workflow_payload_text) else {
            continue;
        };
        object.insert(key.to_string(), Value::String(content));
        break;
    }
    arguments
}

fn workflow_payload_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Object(object) => {
            if let Some(text) = object.get("data").and_then(workflow_payload_text) {
                return Some(text);
            }
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                return Some(text.to_string());
            }
            if let Some(text) = object
                .get("structuredContent")
                .and_then(|structured| structured.get("content"))
                .and_then(Value::as_str)
            {
                return Some(text.to_string());
            }
            object.get("content").and_then(mcp_text_content)
        }
        _ => None,
    }
}

fn mcp_text_content(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let text = items
                .iter()
                .filter_map(|item| {
                    let object = item.as_object()?;
                    if object.get("type").and_then(Value::as_str) != Some("text") {
                        return None;
                    }
                    object.get("text").and_then(Value::as_str)
                })
                .collect::<Vec<_>>()
                .join("\n");
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

fn execute_sync_knowledge_vault_node(
    node_id: &str,
    label: &str,
    external_tools: &impl RuntimeExternalTools,
    arguments: Value,
    timeout_ms: u64,
    mut metadata: Value,
) -> Result<ActionNodeStep, WorkflowRuntimeError> {
    let worker_tools = (*external_tools).clone();
    let worker_arguments = arguments.clone();
    let node_id = node_id.to_string();
    let label = label.to_string();
    let result = run_blocking_node_with_timeout(&node_id, &label, timeout_ms, move || {
        worker_tools.execute_sync_knowledge_vault(worker_arguments)
    })?;
    if let Value::Object(object) = &mut metadata {
        object.insert("arguments".to_string(), arguments);
    }
    Ok(ActionNodeStep::Completed(NodeOutcome::output(
        json!({
            "mediaType": "application/json",
            "data": result,
            "assetPath": null,
            "metadata": metadata,
        }),
        vec!["out".to_string()],
    )))
}

fn sync_knowledge_vault_arguments_from_system_action(
    args: &[String],
    working_directory: Option<&str>,
) -> Result<Value, WorkflowRuntimeError> {
    let mut path = working_directory
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let mut max_files = None;
    let mut mod_id = None;
    let mut index = 0;

    while index < args.len() {
        let arg = args[index].trim();
        if arg.is_empty() {
            index += 1;
            continue;
        }
        if let Some((key, value)) = arg.split_once('=') {
            apply_sync_knowledge_vault_arg(
                key.trim_start_matches('-'),
                value,
                &mut path,
                &mut max_files,
                &mut mod_id,
            )?;
            index += 1;
            continue;
        }
        let key = arg.trim_start_matches('-');
        let normalized = normalize_runtime_identifier(key);
        if matches!(
            normalized.as_str(),
            "path" | "p" | "max_files" | "maxfiles" | "mod_id" | "modid"
        ) {
            let Some(value) = args.get(index + 1) else {
                return Err(WorkflowRuntimeError::execution(format!(
                    "sync_knowledge_vault argument {arg} requires a value."
                )));
            };
            apply_sync_knowledge_vault_arg(key, value, &mut path, &mut max_files, &mut mod_id)?;
            index += 2;
            continue;
        }
        if path.is_none() {
            path = Some(arg.to_string());
        }
        index += 1;
    }

    let Some(path) = path else {
        return Err(WorkflowRuntimeError::execution(
            "sync_knowledge_vault system action requires a path argument or workingDirectory."
                .to_string(),
        ));
    };
    let mut arguments = Map::new();
    arguments.insert("path".to_string(), Value::String(path));
    if let Some(max_files) = max_files {
        arguments.insert(
            "maxFiles".to_string(),
            Value::Number(serde_json::Number::from(max_files as u64)),
        );
    }
    if let Some(mod_id) = mod_id {
        arguments.insert("modId".to_string(), Value::String(mod_id));
    }
    Ok(Value::Object(arguments))
}

fn apply_sync_knowledge_vault_arg(
    key: &str,
    value: &str,
    path: &mut Option<String>,
    max_files: &mut Option<usize>,
    mod_id: &mut Option<String>,
) -> Result<(), WorkflowRuntimeError> {
    let value = value.trim();
    match normalize_runtime_identifier(key).as_str() {
        "path" | "p" => {
            if !value.is_empty() {
                *path = Some(value.to_string());
            }
        }
        "max_files" | "maxfiles" => {
            *max_files = Some(parse_sync_knowledge_max_files(value)?);
        }
        "mod_id" | "modid" => {
            if !value.is_empty() {
                *mod_id = Some(value.to_string());
            }
        }
        _ => {}
    }
    Ok(())
}

fn parse_sync_knowledge_max_files(value: &str) -> Result<usize, WorkflowRuntimeError> {
    let parsed = value.parse::<usize>().map_err(|_| {
        WorkflowRuntimeError::execution(
            "sync_knowledge_vault maxFiles must be a positive integer.".to_string(),
        )
    })?;
    if parsed == 0 {
        return Err(WorkflowRuntimeError::execution(
            "sync_knowledge_vault maxFiles must be greater than zero.".to_string(),
        ));
    }
    Ok(parsed)
}

fn execute_agent_with_timeout(
    agent: &AgentNode,
    instruction: Option<&CompiledInstruction>,
    model: &impl RuntimeModel,
    memory: &HashMap<String, Value>,
    workspace_root: &Path,
    instance_id: &str,
) -> Result<NodeOutcome, WorkflowRuntimeError> {
    let timeout_ms = agent_timeout_ms(agent);
    let agent = agent.clone();
    let node_id = agent.id.clone();
    let label = agent.label.clone();
    let instruction = instruction.cloned();
    let model = (*model).clone();
    let memory = memory.clone();
    let workspace_root = workspace_root.to_path_buf();
    let instance_id = instance_id.to_string();
    run_blocking_node_with_timeout(&node_id, &label, timeout_ms, move || {
        execute_agent(
            &agent,
            instruction.as_ref(),
            &model,
            &memory,
            &workspace_root,
            &instance_id,
        )
    })
}

fn execute_agent(
    agent: &AgentNode,
    instruction: Option<&CompiledInstruction>,
    model: &impl RuntimeModel,
    memory: &HashMap<String, Value>,
    workspace_root: &Path,
    instance_id: &str,
) -> Result<NodeOutcome, WorkflowRuntimeError> {
    let instruction = instruction.ok_or_else(|| {
        WorkflowRuntimeError::execution(format!(
            "Agent node {} has no compiled instruction.",
            agent.id
        ))
    })?;
    let mut variables = Map::new();
    for (name, template) in &instruction.input_variable_mappings {
        variables.insert(name.clone(), resolve_template(template, memory)?);
    }
    evidence_input::validate_agent_variables(&variables)?;
    let generated = model.execute_agent(
        &format!("workflow-run:{instance_id}:{}", agent.id),
        &instruction.system_prompt,
        &variables,
    )?;
    let envelope =
        materialize_generated_output(&generated.text, workspace_root, instance_id, &agent.id)?;
    Ok(NodeOutcome {
        output: serde_json::to_value(envelope).map_err(WorkflowRuntimeError::serialization)?,
        ports: vec!["out".to_string()],
        prompt_tokens: generated.prompt_tokens,
        completion_tokens: generated.completion_tokens,
        completion_kind: WorkflowCompletionKind::Result,
    })
}

fn execute_router_with_timeout(
    router: &RouterNode,
    model: &impl RuntimeModel,
    input: Option<&Value>,
    memory: &HashMap<String, Value>,
    instance_id: &str,
) -> Result<NodeOutcome, WorkflowRuntimeError> {
    let timeout_ms = router_timeout_ms(router);
    let router = router.clone();
    let node_id = router.id.clone();
    let label = router.label.clone();
    let model = (*model).clone();
    let input = input.cloned();
    let memory = memory.clone();
    let instance_id = instance_id.to_string();
    run_blocking_node_with_timeout(&node_id, &label, timeout_ms, move || {
        execute_router(&router, &model, input.as_ref(), &memory, &instance_id)
    })
}

fn execute_router(
    router: &RouterNode,
    model: &impl RuntimeModel,
    input: Option<&Value>,
    memory: &HashMap<String, Value>,
    instance_id: &str,
) -> Result<NodeOutcome, WorkflowRuntimeError> {
    let input = input.cloned().unwrap_or(Value::Null);
    let (port, prompt_tokens, completion_tokens) = if let Some(result) =
        crate::condition_expression::evaluate_basic_condition(&router.expression, memory, &input)
    {
        (route_for_boolean(router, result)?, 0, 0)
    } else {
        let classified = model.classify_route(
            &format!("workflow-route:{instance_id}:{}", router.id),
            router,
            &input,
        )?;
        let selected = classified.text.trim();
        let port = router
            .routes
            .iter()
            .find(|route| route.port == selected)
            .map(|route| route.port.clone())
            .ok_or_else(|| {
                WorkflowRuntimeError::execution(format!(
                    "Router {} returned unknown port {:?}.",
                    router.id, selected
                ))
            })?;
        (port, classified.prompt_tokens, classified.completion_tokens)
    };
    Ok(NodeOutcome {
        output: json!({
            "mediaType": "application/json",
            "data": { "selectedPort": port, "input": input },
            "assetPath": null,
            "metadata": {}
        }),
        ports: vec![port],
        prompt_tokens,
        completion_tokens,
        completion_kind: WorkflowCompletionKind::Result,
    })
}

fn execute_conditional_with_timeout(
    conditional: &ConditionalNode,
    model: &impl RuntimeModel,
    input: Option<&Value>,
    memory: &HashMap<String, Value>,
    instance_id: &str,
) -> Result<NodeOutcome, WorkflowRuntimeError> {
    let timeout_ms = conditional_timeout_ms(conditional);
    let conditional = conditional.clone();
    let node_id = conditional.id.clone();
    let label = conditional.label.clone();
    let model = (*model).clone();
    let input = input.cloned();
    let memory = memory.clone();
    let instance_id = instance_id.to_string();
    run_blocking_node_with_timeout(&node_id, &label, timeout_ms, move || {
        execute_conditional(&conditional, &model, input.as_ref(), &memory, &instance_id)
    })
}

fn execute_conditional(
    conditional: &ConditionalNode,
    model: &impl RuntimeModel,
    input: Option<&Value>,
    memory: &HashMap<String, Value>,
    instance_id: &str,
) -> Result<NodeOutcome, WorkflowRuntimeError> {
    let input = match conditional.input_mapping.as_deref() {
        Some(template) => resolve_template(template, memory)?,
        None => input.cloned().unwrap_or(Value::Null),
    };
    let (result, prompt_tokens, completion_tokens) = if let Some(result) =
        crate::condition_expression::evaluate_basic_condition(
            &conditional.condition,
            memory,
            &input,
        ) {
        (result, 0, 0)
    } else {
        let judged = model.evaluate_condition(
            &format!("workflow-condition:{instance_id}:{}", conditional.id),
            conditional,
            &input,
        )?;
        let result = parse_model_boolean(&judged.text).ok_or_else(|| {
            WorkflowRuntimeError::execution(format!(
                "Conditional {} returned non-boolean judgment {:?}.",
                conditional.id,
                judged.text.trim()
            ))
        })?;
        (result, judged.prompt_tokens, judged.completion_tokens)
    };
    let port = if result { "true" } else { "false" }.to_string();
    Ok(NodeOutcome {
        output: json!({
            "mediaType": "application/json",
            "data": {
                "result": result,
                "selectedPort": port,
                "input": input,
            },
            "assetPath": null,
            "metadata": {}
        }),
        ports: vec![port],
        prompt_tokens,
        completion_tokens,
        completion_kind: WorkflowCompletionKind::Result,
    })
}

fn execute_output(
    output: &OutputNode,
    binding: Option<&OutputBinding>,
    memory: &HashMap<String, Value>,
    instance_id: &str,
    workflow_ir: &WorkflowIr,
) -> Result<NodeOutcome, WorkflowRuntimeError> {
    if output.completion_kind == WorkflowCompletionKind::EmptyCollection {
        validate_explicit_empty_collection_output(output, workflow_ir)?;
        let payload = resolve_template(output.input_mapping.trim(), memory)?;
        if !payload.as_array().is_some_and(|items| items.is_empty()) {
            return Err(WorkflowRuntimeError::execution(format!(
                "Output node {} declared completionKind empty_collection, but its inputMapping did not resolve to an empty array.",
                output.id
            )));
        }
        return Ok(NodeOutcome::empty_collection());
    }
    let payload = resolve_template(&output.input_mapping, memory)?;
    match binding.unwrap_or(&OutputBinding::Ui) {
        OutputBinding::Ui => {}
        OutputBinding::LocalDirectory {
            directory,
            file_name,
        } => {
            let workspace_root = ensure_workflow_workspace_root()?;
            let directory =
                validate_workflow_output_directory(&workspace_root, Path::new(directory))?;
            let file_name = file_name
                .as_deref()
                .map(sanitize_file_name)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("{instance_id}-{}.json", output.id));
            let path = canonicalize_and_validate_path(&workspace_root, &directory.join(file_name))
                .map_err(|error| WorkflowRuntimeError::permission_rejected(&error))?;
            let bytes =
                serde_json::to_vec_pretty(&payload).map_err(WorkflowRuntimeError::serialization)?;
            fs::write(&path, bytes).map_err(WorkflowRuntimeError::io)?;
        }
    }
    Ok(NodeOutcome::output(payload, Vec::new()))
}

fn validate_explicit_empty_collection_output(
    output: &OutputNode,
    workflow_ir: &WorkflowIr,
) -> Result<(), WorkflowRuntimeError> {
    let reference = exact_template_reference(&output.input_mapping).ok_or_else(|| {
        WorkflowRuntimeError::execution(format!(
            "Output node {} declared completionKind empty_collection, but its inputMapping is not one exact collection reference.",
            output.id
        ))
    })?;
    let (producer_node_id, data_path) = workflow_output_data_reference(&reference).ok_or_else(|| {
        WorkflowRuntimeError::execution(format!(
            "Output node {} declared completionKind empty_collection, but {} is not a canonical producer output data reference.",
            output.id, reference
        ))
    })?;
    let producer = workflow_ir
        .nodes
        .iter()
        .find(|node| node.id() == producer_node_id)
        .and_then(|node| match node {
            WorkflowNode::McpTool(producer) => Some(producer),
            _ => None,
        })
        .ok_or_else(|| {
            WorkflowRuntimeError::execution(format!(
                "Output node {} declared completionKind empty_collection, but producer {} is not an MCP tool with a collection contract.",
                output.id, producer_node_id
            ))
        })?;
    let declared_path = producer
        .output_schema
        .as_ref()
        .and_then(declared_primary_collection_path)
        .ok_or_else(|| {
            WorkflowRuntimeError::execution(format!(
                "Output node {} declared completionKind empty_collection, but producer {} has no schema-declared primary empty-success collection.",
                output.id, producer_node_id
            ))
        })?;
    if data_path != declared_path {
        return Err(WorkflowRuntimeError::execution(format!(
            "Output node {} declared completionKind empty_collection for {}, but producer {} declares data.{} as its primary collection.",
            output.id, reference, producer_node_id, declared_path
        )));
    }
    Ok(())
}

fn exact_template_reference(template: &str) -> Option<String> {
    let template = template.trim();
    let expression = Regex::new(r"\{\{\s*([^{}]+?)\s*\}\}").ok()?;
    let captures = expression.captures(template)?;
    let full = captures.get(0)?;
    if full.as_str() != template {
        return None;
    }
    Some(captures.get(1)?.as_str().trim().to_string())
}

fn workflow_output_data_reference(reference: &str) -> Option<(&str, String)> {
    let reference = reference.strip_prefix("nodes.").unwrap_or(reference);
    let (producer_node_id, data_path) = reference.split_once(".output.data.")?;
    if producer_node_id.is_empty() || data_path.is_empty() {
        return None;
    }
    Some((producer_node_id, data_path.to_string()))
}

fn execute_loop_node(
    loop_node: &LoopNode,
    order: &[String],
    node_by_id: &HashMap<&str, &WorkflowNode>,
    incoming: &HashMap<&str, Vec<&WorkflowEdge>>,
    outgoing: &HashMap<&str, Vec<&WorkflowEdge>>,
    compiled: &CompiledWorkflow,
    model: &impl RuntimeModel,
    external_tools: &impl RuntimeExternalTools,
    workspace_root: &Path,
    instance: &mut ExecutionInstance,
    input: Option<Value>,
    memory: &HashMap<String, Value>,
    selected_edges: &HashSet<String>,
    step_index: usize,
    approval_ledger: Option<&PersistenceEngine>,
    ready_mcp_servers: &mut HashSet<String>,
    progress: &mut impl FnMut(&ExecutionInstance, &str, usize, &str, &str),
) -> Result<LoopExecutionResult, WorkflowRuntimeError> {
    let _timeout_ms = loop_timeout_ms(loop_node);
    let items_value = resolve_template(&loop_node.items_mapping, memory)?;
    let resolved_to_empty_array = resolves_to_empty_loop_array(&items_value);
    let items = coerce_loop_items(items_value).ok_or_else(|| {
        WorkflowRuntimeError::execution(format!(
            "Loop node {} expected an array or newline-delimited string.",
            loop_node.id
        ))
    })?;
    let body_node_ids = loop_body_node_ids(loop_node, outgoing, node_by_id)?;
    if body_node_ids.is_empty() {
        return Err(WorkflowRuntimeError::execution(format!(
            "Loop node {} has no downstream body nodes on its item port.",
            loop_node.id
        )));
    }
    if items.is_empty() {
        if !resolved_to_empty_array {
            return Err(WorkflowRuntimeError::execution(format!(
                "Loop node {} resolved zero items from a value that was not an empty array.",
                loop_node.id
            )));
        }
        let mut next_memory = memory.clone();
        let mut body_outputs = Map::new();
        for body_node_id in &body_node_ids {
            let empty_output = Value::Array(Vec::new());
            body_outputs.insert(body_node_id.clone(), empty_output.clone());
            next_memory.insert(format!("nodes.{body_node_id}.output"), empty_output.clone());
            next_memory.insert(format!("{body_node_id}.output"), empty_output.clone());
            instance.node_payloads.insert(
                body_node_id.clone(),
                NodeExecutionPayload {
                    status: ExecutionStatus::Completed,
                    input: Some(Value::Array(Vec::new())),
                    output: Some(empty_output),
                    error: None,
                    latency_ms: 0,
                    prompt_tokens: 0,
                    completion_tokens: 0,
                },
            );
        }
        return Ok(LoopExecutionResult {
            outcome: NodeOutcome {
                output: json!({
                    "mediaType": "application/json",
                    "data": {
                        "itemCount": 0,
                        "items": [],
                        "bodyOutputs": body_outputs,
                    },
                    "assetPath": null,
                    "metadata": {
                        "itemVariable": loop_node.item_variable,
                    }
                }),
                ports: vec!["done".to_string()],
                prompt_tokens: 0,
                completion_tokens: 0,
                completion_kind: WorkflowCompletionKind::Result,
            },
            body_node_ids,
            execution_order: Vec::new(),
            memory: next_memory,
        });
    }
    let body_order = order
        .iter()
        .filter(|node_id| body_node_ids.contains(node_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let item_edges = outgoing
        .get(loop_node.id.as_str())
        .into_iter()
        .flatten()
        .filter(|edge| edge.source_port == "item")
        .map(|edge| edge.id.clone())
        .collect::<Vec<_>>();
    let mut next_memory = memory.clone();
    let mut execution_order = Vec::new();
    let mut aggregate_inputs = HashMap::<String, Vec<Value>>::new();
    let mut aggregate_outputs = HashMap::<String, Vec<Value>>::new();
    let mut aggregate_latency = HashMap::<String, u64>::new();
    let mut aggregate_prompt_tokens = HashMap::<String, u64>::new();
    let mut aggregate_completion_tokens = HashMap::<String, u64>::new();
    let mut total_prompt_tokens = 0;
    let mut total_completion_tokens = 0;

    for (index, item) in items.iter().cloned().enumerate() {
        let item_output = json!({
            "mediaType": "application/json",
            "data": item,
            "assetPath": null,
            "metadata": {
                "loopNodeId": loop_node.id,
                "index": index,
            }
        });
        let mut iteration_memory = next_memory.clone();
        iteration_memory.insert(loop_node.item_variable.clone(), item.clone());
        iteration_memory.insert("item".to_string(), item.clone());
        iteration_memory.insert(format!("loop.{}.item", loop_node.id), item.clone());
        iteration_memory.insert(format!("loop.{}.index", loop_node.id), json!(index));
        let mut iteration_payloads = instance.node_payloads.clone();
        iteration_payloads.insert(
            loop_node.id.clone(),
            NodeExecutionPayload {
                status: ExecutionStatus::Running,
                input: input.clone(),
                output: Some(item_output.clone()),
                error: None,
                latency_ms: 0,
                prompt_tokens: 0,
                completion_tokens: 0,
            },
        );
        let mut iteration_selected_edges = selected_edges.clone();
        for edge_id in &item_edges {
            iteration_selected_edges.insert(edge_id.clone());
        }

        for body_node_id in &body_order {
            let body_node = node_by_id[&body_node_id.as_str()];
            let reachable = incoming.get(body_node.id()).is_some_and(|edges| {
                edges
                    .iter()
                    .any(|edge| iteration_selected_edges.contains(&edge.id))
            });
            if !reachable {
                continue;
            }
            let started = Instant::now();
            let body_input = incoming_payload(
                body_node.id(),
                incoming,
                &iteration_selected_edges,
                &iteration_payloads,
            );
            progress(
                instance,
                body_node.id(),
                step_index,
                "running",
                "Running loop body node.",
            );
            let node_step = execute_loop_body_node_step(
                body_node,
                compiled,
                model,
                external_tools,
                workspace_root,
                instance,
                body_input.clone(),
                &iteration_memory,
                &iteration_selected_edges,
                approval_ledger,
                ready_mcp_servers,
            )?;
            let outcome = match node_step {
                ActionNodeStep::Completed(outcome) => outcome,
                ActionNodeStep::Paused(_) => {
                    return Err(WorkflowRuntimeError::execution(format!(
                        "Loop body node {} requested approval. Place approval gates outside loop bodies.",
                        body_node.id()
                    )));
                }
            };
            let latency_ms = elapsed_ms(started);
            total_prompt_tokens += outcome.prompt_tokens;
            total_completion_tokens += outcome.completion_tokens;
            aggregate_inputs
                .entry(body_node.id().to_string())
                .or_default()
                .push(body_input.unwrap_or(Value::Null));
            aggregate_outputs
                .entry(body_node.id().to_string())
                .or_default()
                .push(outcome.output.clone());
            *aggregate_latency
                .entry(body_node.id().to_string())
                .or_default() += latency_ms;
            *aggregate_prompt_tokens
                .entry(body_node.id().to_string())
                .or_default() += outcome.prompt_tokens;
            *aggregate_completion_tokens
                .entry(body_node.id().to_string())
                .or_default() += outcome.completion_tokens;
            iteration_payloads.insert(
                body_node.id().to_string(),
                NodeExecutionPayload {
                    status: ExecutionStatus::Completed,
                    input: aggregate_inputs
                        .get(body_node.id())
                        .cloned()
                        .map(Value::Array),
                    output: Some(outcome.output.clone()),
                    error: None,
                    latency_ms,
                    prompt_tokens: outcome.prompt_tokens,
                    completion_tokens: outcome.completion_tokens,
                },
            );
            iteration_memory.insert(
                format!("nodes.{}.output", body_node.id()),
                outcome.output.clone(),
            );
            iteration_memory.insert(format!("{}.output", body_node.id()), outcome.output.clone());
            iteration_memory.insert("workflow.output".to_string(), outcome.output.clone());
            if let WorkflowNode::Agent(agent) = body_node {
                iteration_memory.insert(agent.output_key.clone(), outcome.output.clone());
            }
            for edge in outgoing.get(body_node.id()).into_iter().flatten() {
                if outcome.ports.iter().any(|port| port == &edge.source_port) {
                    iteration_selected_edges.insert(edge.id.clone());
                }
            }
            execution_order.push(body_node.id().to_string());
            progress(
                instance,
                body_node.id(),
                step_index,
                "success",
                "Loop body node completed.",
            );
        }
        next_memory = iteration_memory;
    }

    let mut body_outputs_json = Map::new();
    for body_node_id in &body_node_ids {
        let Some(outputs) = aggregate_outputs.remove(body_node_id) else {
            continue;
        };
        let inputs = aggregate_inputs.remove(body_node_id).ok_or_else(|| {
            WorkflowRuntimeError::execution(format!(
                "Loop body node {body_node_id} produced output without recorded input evidence."
            ))
        })?;
        let latency_ms = aggregate_latency.remove(body_node_id).ok_or_else(|| {
            WorkflowRuntimeError::execution(format!(
                "Loop body node {body_node_id} produced output without latency evidence."
            ))
        })?;
        let prompt_tokens = aggregate_prompt_tokens
            .remove(body_node_id)
            .ok_or_else(|| {
                WorkflowRuntimeError::execution(format!(
                    "Loop body node {body_node_id} produced output without prompt-token evidence."
                ))
            })?;
        let completion_tokens =
            aggregate_completion_tokens
                .remove(body_node_id)
                .ok_or_else(|| {
                    WorkflowRuntimeError::execution(format!(
                        "Loop body node {body_node_id} produced output without completion-token evidence."
                    ))
                })?;
        let output_value = Value::Array(outputs.clone());
        body_outputs_json.insert(body_node_id.clone(), output_value.clone());
        next_memory.insert(format!("nodes.{body_node_id}.output"), output_value.clone());
        next_memory.insert(format!("{body_node_id}.output"), output_value.clone());
        next_memory.insert("workflow.output".to_string(), output_value.clone());
        instance.node_payloads.insert(
            body_node_id.clone(),
            NodeExecutionPayload {
                status: ExecutionStatus::Completed,
                input: Some(Value::Array(inputs)),
                output: Some(output_value),
                error: None,
                latency_ms,
                prompt_tokens,
                completion_tokens,
            },
        );
    }

    Ok(LoopExecutionResult {
        outcome: NodeOutcome {
            output: json!({
                "mediaType": "application/json",
                "data": {
                    "itemCount": items.len(),
                    "items": items,
                    "bodyOutputs": body_outputs_json,
                },
                "assetPath": null,
                "metadata": {
                    "itemVariable": loop_node.item_variable,
                }
            }),
            ports: vec!["done".to_string()],
            prompt_tokens: total_prompt_tokens,
            completion_tokens: total_completion_tokens,
            completion_kind: WorkflowCompletionKind::Result,
        },
        body_node_ids,
        execution_order,
        memory: next_memory,
    })
}

fn execute_loop_body_node_step(
    node: &WorkflowNode,
    compiled: &CompiledWorkflow,
    model: &impl RuntimeModel,
    external_tools: &impl RuntimeExternalTools,
    workspace_root: &Path,
    instance: &mut ExecutionInstance,
    input: Option<Value>,
    memory: &HashMap<String, Value>,
    selected_edges: &HashSet<String>,
    approval_ledger: Option<&PersistenceEngine>,
    ready_mcp_servers: &mut HashSet<String>,
) -> Result<ActionNodeStep, WorkflowRuntimeError> {
    match node {
        WorkflowNode::Agent(agent) => execute_agent_with_timeout(
            agent,
            compiled.instructions.get(&agent.id),
            model,
            memory,
            workspace_root,
            &instance.id,
        )
        .map(ActionNodeStep::Completed),
        WorkflowNode::Router(router) => {
            execute_router_with_timeout(router, model, input.as_ref(), memory, &instance.id)
                .map(ActionNodeStep::Completed)
        }
        WorkflowNode::Conditional(conditional) => execute_conditional_with_timeout(
            conditional,
            model,
            input.as_ref(),
            memory,
            &instance.id,
        )
        .map(ActionNodeStep::Completed),
        WorkflowNode::McpTool(mcp_tool) => execute_mcp_tool_node(
            mcp_tool,
            external_tools,
            instance,
            input,
            memory,
            selected_edges,
            0,
            approval_ledger,
            None,
            None,
            ready_mcp_servers,
        ),
        WorkflowNode::SystemAction(system_action) => execute_system_action_node(
            system_action,
            model,
            external_tools,
            workspace_root,
            instance,
            input,
            memory,
            selected_edges,
            0,
            None,
        ),
        WorkflowNode::Input(_)
        | WorkflowNode::Output(_)
        | WorkflowNode::Permission(_)
        | WorkflowNode::Loop(_) => Err(WorkflowRuntimeError::execution(format!(
            "Node {} cannot execute inside a loop body.",
            node.id()
        ))),
    }
}

fn coerce_loop_items(value: Value) -> Option<Vec<Value>> {
    match value {
        Value::Array(items) => Some(items),
        Value::Object(mut object) => object
            .remove("data")
            .and_then(coerce_loop_items)
            .or_else(|| object.remove("items").and_then(coerce_loop_items)),
        Value::String(text) => Some(
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| Value::String(line.to_string()))
                .collect(),
        ),
        Value::Null => Some(Vec::new()),
        value => Some(vec![value]),
    }
}

fn resolves_to_empty_loop_array(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.is_empty(),
        Value::Object(object) => object
            .get("data")
            .map(resolves_to_empty_loop_array)
            .or_else(|| object.get("items").map(resolves_to_empty_loop_array))
            .unwrap_or(false),
        _ => false,
    }
}

fn loop_body_node_ids(
    loop_node: &LoopNode,
    outgoing: &HashMap<&str, Vec<&WorkflowEdge>>,
    node_by_id: &HashMap<&str, &WorkflowNode>,
) -> Result<HashSet<String>, WorkflowRuntimeError> {
    let mut body = HashSet::new();
    let mut queue = outgoing
        .get(loop_node.id.as_str())
        .into_iter()
        .flatten()
        .filter(|edge| edge.source_port == "item")
        .map(|edge| edge.target_node_id.clone())
        .collect::<VecDeque<_>>();
    while let Some(node_id) = queue.pop_front() {
        let Some(node) = node_by_id.get(node_id.as_str()) else {
            return Err(WorkflowRuntimeError::execution(format!(
                "Loop node {} points to unknown body node {}.",
                loop_node.id, node_id
            )));
        };
        if matches!(node, WorkflowNode::Output(_)) || !body.insert(node_id.clone()) {
            continue;
        }
        for edge in outgoing.get(node_id.as_str()).into_iter().flatten() {
            if edge.source_port != "done" {
                queue.push_back(edge.target_node_id.clone());
            }
        }
    }
    Ok(body)
}

fn pause_for_external_action(
    instance: &mut ExecutionInstance,
    node_id: &str,
    message: &str,
    context: Value,
    input: Option<Value>,
    memory: &HashMap<String, Value>,
    selected_edges: &HashSet<String>,
    latency_ms: u64,
) -> Result<ApprovalRequest, WorkflowRuntimeError> {
    let approval_token = generate_approval_token();
    let token_hash = hash_approval_token(&approval_token);
    let pause_context =
        merge_approval_recovery(context.clone(), token_hash, &approval_token, message);
    instance.status = ExecutionStatus::AwaitingApproval;
    instance.memory = memory.clone();
    instance.selected_edges = selected_edges.clone();
    instance.pause_context = Some(pause_context);
    instance.node_payloads.insert(
        node_id.to_string(),
        NodeExecutionPayload {
            status: ExecutionStatus::AwaitingApproval,
            input,
            output: None,
            error: None,
            latency_ms,
            prompt_tokens: 0,
            completion_tokens: 0,
        },
    );
    finish_timing(instance, false);
    let command = |decision: PermissionDecision| {
        json!({
            "command": "resolve_workflow_permission",
            "request": {
                "instanceId": instance.id,
                "approvalToken": approval_token,
                "decision": decision,
            }
        })
    };
    let approve_command = command(PermissionDecision::Approve);
    let reject_command = command(PermissionDecision::Reject);
    Ok(ApprovalRequest {
        instance_id: instance.id.clone(),
        workflow_id: instance.workflow_id.clone(),
        node_id: node_id.to_string(),
        message: message.to_string(),
        context,
        approval_token,
        approve_command,
        reject_command,
    })
}

fn pause_for_permission(
    instance: &mut ExecutionInstance,
    permission: &crate::workflow_ir::PermissionNode,
    exact_effect: Option<Value>,
    input: Option<Value>,
    memory: &HashMap<String, Value>,
    selected_edges: &HashSet<String>,
    latency_ms: u64,
) -> Result<ApprovalRequest, WorkflowRuntimeError> {
    let approval_token = generate_approval_token();
    let token_hash = hash_approval_token(&approval_token);
    let (context, pause_context) = task_tools::permission_pause_context(
        permission,
        exact_effect,
        input.clone(),
        token_hash,
        &approval_token,
    )?;
    instance.status = ExecutionStatus::AwaitingApproval;
    instance.memory = memory.clone();
    instance.selected_edges = selected_edges.clone();
    instance.pause_context = Some(pause_context);
    instance.node_payloads.insert(
        permission.id.clone(),
        NodeExecutionPayload {
            status: ExecutionStatus::AwaitingApproval,
            input,
            output: None,
            error: None,
            latency_ms,
            prompt_tokens: 0,
            completion_tokens: 0,
        },
    );
    finish_timing(instance, false);
    let command = |decision: PermissionDecision| {
        json!({
            "command": "resolve_workflow_permission",
            "request": {
                "instanceId": instance.id,
                "approvalToken": approval_token,
                "decision": decision,
            }
        })
    };
    let approve_command = command(PermissionDecision::Approve);
    let reject_command = command(PermissionDecision::Reject);
    Ok(ApprovalRequest {
        instance_id: instance.id.clone(),
        workflow_id: instance.workflow_id.clone(),
        node_id: permission.id.clone(),
        message: permission.reason.clone(),
        context,
        approval_token,
        approve_command,
        reject_command,
    })
}

fn merge_approval_recovery(
    mut context: Value,
    token_hash: String,
    approval_token: &str,
    message: &str,
) -> Value {
    let approval_context = context.clone();
    match &mut context {
        Value::Object(object) => {
            object.insert("approvalTokenHash".to_string(), Value::String(token_hash));
            object.insert(
                "approvalToken".to_string(),
                Value::String(approval_token.to_string()),
            );
            object.insert(
                "approvalMessage".to_string(),
                Value::String(message.to_string()),
            );
            object.insert("approvalContext".to_string(), approval_context);
            context
        }
        _ => json!({
            "context": context,
            "approvalTokenHash": token_hash,
            "approvalToken": approval_token,
            "approvalMessage": message,
            "approvalContext": approval_context,
        }),
    }
}

fn approval_request_from_instance(instance: &ExecutionInstance) -> Option<ApprovalRequest> {
    if instance.status != ExecutionStatus::AwaitingApproval {
        return None;
    }
    let pause_context = instance.pause_context.as_ref()?;
    let approval_token = pause_context.get("approvalToken")?.as_str()?.trim();
    let message = pause_context.get("approvalMessage")?.as_str()?.trim();
    let node_id = instance.active_node_id.as_deref()?.trim();
    if approval_token.is_empty() || message.is_empty() || node_id.is_empty() {
        return None;
    }
    let context = pause_context
        .get("approvalContext")
        .cloned()
        .unwrap_or_else(|| {
            let mut context = pause_context.clone();
            if let Some(object) = context.as_object_mut() {
                for key in [
                    "approvalTokenHash",
                    "approvalToken",
                    "approvalMessage",
                    "approvalContext",
                ] {
                    object.remove(key);
                }
            }
            context
        });
    let command = |decision: PermissionDecision| {
        json!({
            "command": "resolve_workflow_permission",
            "request": {
                "instanceId": instance.id,
                "approvalToken": approval_token,
                "decision": decision,
            }
        })
    };
    Some(ApprovalRequest {
        instance_id: instance.id.clone(),
        workflow_id: instance.workflow_id.clone(),
        node_id: node_id.to_string(),
        message: message.to_string(),
        context,
        approval_token: approval_token.to_string(),
        approve_command: command(PermissionDecision::Approve),
        reject_command: command(PermissionDecision::Reject),
    })
}

fn resume_decision_for_node(
    node_id: &str,
    resume_permission: Option<&ResumePermission>,
) -> Option<PermissionDecision> {
    resume_permission
        .filter(|resume| resume.node_id == node_id)
        .map(|resume| resume.decision)
}

fn bounded_node_timeout_ms(timeout_ms: u64) -> u64 {
    timeout_ms.clamp(1, crate::workflow_ir::LONG_TIMEOUT_MS)
}

fn configured_node_timeout_ms(configured: Option<u64>, default_ms: u64) -> u64 {
    bounded_node_timeout_ms(configured.unwrap_or(default_ms))
}

fn agent_timeout_ms(agent: &AgentNode) -> u64 {
    configured_node_timeout_ms(agent.system_timeout_ms, default_agent_timeout_ms(agent))
}

fn router_timeout_ms(router: &RouterNode) -> u64 {
    configured_node_timeout_ms(router.system_timeout_ms, default_router_timeout_ms(router))
}

fn conditional_timeout_ms(conditional: &ConditionalNode) -> u64 {
    configured_node_timeout_ms(
        conditional.system_timeout_ms,
        default_conditional_timeout_ms(conditional),
    )
}

fn loop_timeout_ms(loop_node: &LoopNode) -> u64 {
    configured_node_timeout_ms(
        loop_node.system_timeout_ms,
        default_loop_timeout_ms(loop_node),
    )
}

fn default_agent_timeout_ms(agent: &AgentNode) -> u64 {
    if long_running_operation_hint(&format!("{} {}", agent.label, agent.objective)) {
        crate::workflow_ir::LONG_TIMEOUT_MS
    } else {
        crate::workflow_ir::MEDIUM_TIMEOUT_MS
    }
}

fn default_router_timeout_ms(router: &RouterNode) -> u64 {
    if long_running_operation_hint(&format!("{} {}", router.label, router.expression)) {
        crate::workflow_ir::LONG_TIMEOUT_MS
    } else {
        crate::workflow_ir::SHORT_TIMEOUT_MS
    }
}

fn default_conditional_timeout_ms(conditional: &ConditionalNode) -> u64 {
    if long_running_operation_hint(&format!("{} {}", conditional.label, conditional.condition)) {
        crate::workflow_ir::LONG_TIMEOUT_MS
    } else {
        crate::workflow_ir::SHORT_TIMEOUT_MS
    }
}

fn default_loop_timeout_ms(loop_node: &LoopNode) -> u64 {
    if long_running_operation_hint(&format!("{} {}", loop_node.label, loop_node.items_mapping)) {
        crate::workflow_ir::LONG_TIMEOUT_MS
    } else {
        crate::workflow_ir::MEDIUM_TIMEOUT_MS
    }
}

fn mcp_tool_timeout_ms(mcp_tool: &McpToolNode) -> u64 {
    let configured = configured_node_timeout_ms(
        mcp_tool.system_timeout_ms,
        default_mcp_tool_timeout_ms(mcp_tool),
    );
    configured.max(minimum_mcp_tool_timeout_ms(mcp_tool))
}

fn minimum_mcp_tool_timeout_ms(mcp_tool: &McpToolNode) -> u64 {
    if is_native_calendar_workflow_tool(&mcp_tool.server_name, &mcp_tool.tool_name) {
        SYSTEM_CALENDAR_WORKFLOW_TIMEOUT_MS
    } else if mcp_tool
        .server_name
        .trim()
        .eq_ignore_ascii_case("macos_applescript")
    {
        APPLE_APP_WORKFLOW_TIMEOUT_MS
    } else {
        1
    }
}

fn is_native_calendar_workflow_tool(server_name: &str, tool_name: &str) -> bool {
    server_name.trim().eq_ignore_ascii_case("macos_applescript")
        && tool_name
            .trim()
            .eq_ignore_ascii_case("read_system_calendar")
}

fn default_mcp_tool_timeout_ms(mcp_tool: &McpToolNode) -> u64 {
    if classify_mcp_tool_call(&mcp_tool.server_name, &mcp_tool.tool_name, None)
        .requires_human_approval()
        || long_running_operation_hint(&format!("{} {}", mcp_tool.server_name, mcp_tool.tool_name))
    {
        crate::workflow_ir::LONG_TIMEOUT_MS
    } else {
        crate::workflow_ir::MEDIUM_TIMEOUT_MS
    }
}

fn long_running_operation_hint(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [
        "asset sync",
        "cloud",
        "compute",
        "configure",
        "deep",
        "deploy",
        "docker build",
        "gcloud",
        "multi-turn",
        "provision",
        "reasoning",
        "recursive",
        "remote",
        "research",
        "rsync",
        "scp",
        "ssh",
        "sync_web_assets",
        "terraform",
        "vm",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn run_blocking_node_with_timeout<T>(
    node_id: &str,
    label: &str,
    timeout_ms: u64,
    task: impl FnOnce() -> Result<T, WorkflowRuntimeError> + Send + 'static,
) -> Result<T, WorkflowRuntimeError>
where
    T: Send + 'static,
{
    let timeout_ms = bounded_node_timeout_ms(timeout_ms);
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(task());
    });
    match receiver.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(WorkflowRuntimeError::node_timeout(
            node_id, label, timeout_ms,
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(WorkflowRuntimeError::runtime(format!(
            "Node {node_id} worker exited without returning a result."
        ))),
    }
}

fn command_preview(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    }
}

fn dispatch_workflow_progress(
    app: &tauri::AppHandle,
    instance: &ExecutionInstance,
    node_id: &str,
    step_index: usize,
    status: &str,
    message: &str,
) {
    if let Err(error) = app.emit(
        "vwa://progress",
        json!({
            "plan_id": instance.id,
            "block_id": node_id,
            "step_index": step_index,
            "status": status,
            "message": message,
        }),
    ) {
        eprintln!(
            "WORKFLOW_PROGRESS_NOTIFICATION_FAILED instance_id={} node_id={} error={}",
            instance.id, node_id, error
        );
    }
}

fn validate_approval_request(
    request: &ResolvePermissionRequest,
) -> Result<(), WorkflowRuntimeError> {
    if request.instance_id.trim().is_empty() || request.approval_token.trim().is_empty() {
        return Err(WorkflowRuntimeError::input(
            "instanceId and approvalToken must not be empty.".to_string(),
        ));
    }
    Ok(())
}

fn verify_approval_token(
    instance: &ExecutionInstance,
    approval_token: &str,
) -> Result<(), WorkflowRuntimeError> {
    if instance.status != ExecutionStatus::AwaitingApproval {
        return Err(WorkflowRuntimeError::approval_consumed());
    }
    let expected = instance
        .pause_context
        .as_ref()
        .and_then(|context| context.get("approvalTokenHash"))
        .and_then(Value::as_str)
        .ok_or_else(WorkflowRuntimeError::approval_state_invalid)?;
    let supplied = hash_approval_token(approval_token);
    if !constant_time_eq(expected.as_bytes(), supplied.as_bytes()) {
        return Err(WorkflowRuntimeError::approval_unauthorized());
    }
    Ok(())
}

fn routine_authority_can_satisfy_mcp_review(
    approval_binding: Option<&McpToolApprovalBinding>,
) -> bool {
    !approval_binding.is_some_and(|binding| binding.requires_native_shield)
}

fn generate_approval_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn hash_approval_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"workflow-approval-v1:");
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn resolve_input(binding: &InputBinding) -> Result<Value, WorkflowRuntimeError> {
    let envelope = match binding {
        InputBinding::Manual { value } => PayloadEnvelope {
            media_type: "application/json".to_string(),
            data: Some(value.clone()),
            asset_path: None,
            metadata: Map::new(),
        },
        InputBinding::Environment { name } => {
            let value = resolve_workflow_environment_value(name)?;
            PayloadEnvelope {
                media_type: "text/plain".to_string(),
                data: Some(Value::String(value)),
                asset_path: None,
                metadata: Map::from_iter([(
                    "environmentVariable".to_string(),
                    Value::String(name.clone()),
                )]),
            }
        }
        InputBinding::LocalFile { path } => {
            let workspace_root = ensure_workflow_workspace_root()?;
            let path = canonicalize_and_validate_path(&workspace_root, Path::new(path))
                .map_err(|error| WorkflowRuntimeError::permission_rejected(&error))?;
            let metadata = fs::metadata(&path).map_err(WorkflowRuntimeError::io)?;
            if !metadata.is_file() {
                return Err(WorkflowRuntimeError::input(format!(
                    "Input path {} is not a file.",
                    path.display()
                )));
            }
            if metadata.len() as usize > LARGE_OUTPUT_BYTES {
                PayloadEnvelope {
                    media_type: "application/octet-stream".to_string(),
                    data: None,
                    asset_path: Some(path.to_string_lossy().to_string()),
                    metadata: Map::from_iter([(
                        "sizeBytes".to_string(),
                        Value::from(metadata.len()),
                    )]),
                }
            } else {
                let bytes = fs::read(&path).map_err(WorkflowRuntimeError::io)?;
                let text = String::from_utf8(bytes).map_err(|_| {
                    WorkflowRuntimeError::input(format!(
                        "Small input file {} must contain UTF-8 text.",
                        path.display()
                    ))
                })?;
                PayloadEnvelope {
                    media_type: "text/plain".to_string(),
                    data: Some(Value::String(text)),
                    asset_path: Some(path.to_string_lossy().to_string()),
                    metadata: Map::from_iter([(
                        "sizeBytes".to_string(),
                        Value::from(metadata.len()),
                    )]),
                }
            }
        }
    };
    serde_json::to_value(envelope).map_err(WorkflowRuntimeError::serialization)
}

fn materialize_generated_output(
    text: &str,
    workspace_root: &Path,
    instance_id: &str,
    node_id: &str,
) -> Result<PayloadEnvelope, WorkflowRuntimeError> {
    let code_fences = text.matches("```").count();
    if text.len() >= LARGE_OUTPUT_BYTES || code_fences >= 4 {
        let directory = workspace_root.join(instance_id).join("assets");
        fs::create_dir_all(&directory).map_err(WorkflowRuntimeError::io)?;
        let path = directory.join(format!("{}.md", sanitize_file_name(node_id)));
        fs::write(&path, text).map_err(WorkflowRuntimeError::io)?;
        return Ok(PayloadEnvelope {
            media_type: "text/markdown".to_string(),
            data: None,
            asset_path: Some(path.to_string_lossy().to_string()),
            metadata: Map::from_iter([("sizeBytes".to_string(), Value::from(text.len() as u64))]),
        });
    }
    Ok(PayloadEnvelope {
        media_type: "text/plain".to_string(),
        data: Some(Value::String(text.to_string())),
        asset_path: None,
        metadata: Map::new(),
    })
}

fn resolve_template(
    template: &str,
    memory: &HashMap<String, Value>,
) -> Result<Value, WorkflowRuntimeError> {
    let expression = Regex::new(r"\{\{\s*([^{}]+?)\s*\}\}").map_err(|error| {
        WorkflowRuntimeError::execution(format!("Template parser unavailable: {error}"))
    })?;
    let captures = expression.captures_iter(template).collect::<Vec<_>>();
    if captures.len() == 1
        && captures[0]
            .get(0)
            .is_some_and(|matched| matched.as_str() == template)
    {
        let key = captures[0]
            .get(1)
            .map(|capture| capture.as_str().trim())
            .ok_or_else(|| {
                WorkflowRuntimeError::execution(
                    "Template parser did not return a reference capture.".to_string(),
                )
            })?;
        return resolve_memory_reference(key, memory)
            .map_err(WorkflowRuntimeError::template_resolution);
    }
    let mut rendered = template.to_string();
    for capture in captures {
        let full = capture
            .get(0)
            .map(|matched| matched.as_str())
            .ok_or_else(|| {
                WorkflowRuntimeError::execution(
                    "Template parser did not return the matched expression.".to_string(),
                )
            })?;
        let key = capture
            .get(1)
            .map(|matched| matched.as_str().trim())
            .ok_or_else(|| {
                WorkflowRuntimeError::execution(
                    "Template parser did not return a reference capture.".to_string(),
                )
            })?;
        let value = resolve_memory_reference(key, memory)
            .map_err(WorkflowRuntimeError::template_resolution)?;
        let replacement = match value {
            Value::String(value) => value,
            value => value.to_string(),
        };
        rendered = rendered.replace(full, &replacement);
    }
    Ok(Value::String(rendered))
}

#[derive(Debug, Clone)]
struct TemplateResolutionError {
    reference: String,
    kind: TemplateResolutionErrorKind,
}

#[derive(Debug, Clone)]
enum TemplateResolutionErrorKind {
    UnknownRoot,
    MissingField {
        field: String,
        container_reference: String,
    },
    TypeMismatch {
        segment: String,
        container_reference: String,
        actual: &'static str,
    },
    InvalidArrayIndex {
        segment: String,
        collection_reference: String,
    },
    EmptyArrayIndexed {
        collection_reference: String,
    },
    ArrayIndexOutOfBounds {
        index: usize,
        len: usize,
        collection_reference: String,
    },
}

impl TemplateResolutionError {
    fn new(reference: &str, kind: TemplateResolutionErrorKind) -> Self {
        Self {
            reference: reference.to_string(),
            kind,
        }
    }

    fn message(&self) -> String {
        let detail = match &self.kind {
            TemplateResolutionErrorKind::UnknownRoot => {
                "no matching runtime value exists".to_string()
            }
            TemplateResolutionErrorKind::MissingField {
                field,
                container_reference,
            } => format!("field {field:?} does not exist beneath {container_reference}"),
            TemplateResolutionErrorKind::TypeMismatch {
                segment,
                container_reference,
                actual,
            } => format!(
                "cannot traverse segment {segment:?} beneath {container_reference} because the value is {actual}"
            ),
            TemplateResolutionErrorKind::InvalidArrayIndex {
                segment,
                collection_reference,
            } => format!(
                "segment {segment:?} is not a numeric index for array {collection_reference}"
            ),
            TemplateResolutionErrorKind::EmptyArrayIndexed {
                collection_reference,
            } => format!("array {collection_reference} is empty"),
            TemplateResolutionErrorKind::ArrayIndexOutOfBounds {
                index,
                len,
                collection_reference,
            } => format!(
                "index {index} is out of bounds for array {collection_reference} with length {len}"
            ),
        };
        format!(
            "Template reference {} is unresolved: {detail}.",
            self.reference
        )
    }
}

fn resolve_memory_reference(
    reference: &str,
    memory: &HashMap<String, Value>,
) -> Result<Value, TemplateResolutionError> {
    if let Some(value) = memory.get(reference) {
        return Ok(value.clone());
    }

    let Some((base_key, base_value)) = memory
        .iter()
        .filter(|(key, _)| {
            reference.len() > key.len()
                && reference.starts_with(key.as_str())
                && reference.as_bytes().get(key.len()) == Some(&b'.')
        })
        .max_by_key(|(key, _)| key.len())
    else {
        return Err(TemplateResolutionError::new(
            reference,
            TemplateResolutionErrorKind::UnknownRoot,
        ));
    };
    let path = &reference[base_key.len() + 1..];
    let direct = resolve_value_path(base_value, path, reference, base_key);
    if direct.is_ok() {
        return direct;
    }

    // Legacy workflows may expose fields under output; fall back only when absent.
    let first_segment = path.split('.').next().unwrap_or_default();
    let root_field_is_absent = base_value
        .as_object()
        .is_some_and(|object| !object.contains_key(first_segment));
    if root_field_is_absent {
        if let Some(structured) = base_value
            .get("data")
            .and_then(|data| data.get("structuredContent"))
        {
            let structured_reference = format!("{base_key}.data.structuredContent");
            return resolve_value_path(structured, path, reference, &structured_reference);
        }
    }
    direct
}

fn resolve_value_path(
    value: &Value,
    path: &str,
    reference: &str,
    base_reference: &str,
) -> Result<Value, TemplateResolutionError> {
    let mut current = value;
    let mut current_reference = base_reference.to_string();
    for segment in path.split('.') {
        if segment.is_empty() {
            return Err(TemplateResolutionError::new(
                reference,
                TemplateResolutionErrorKind::MissingField {
                    field: segment.to_string(),
                    container_reference: current_reference,
                },
            ));
        }
        current = match current {
            Value::Object(object) => object.get(segment).ok_or_else(|| {
                TemplateResolutionError::new(
                    reference,
                    TemplateResolutionErrorKind::MissingField {
                        field: segment.to_string(),
                        container_reference: current_reference.clone(),
                    },
                )
            })?,
            Value::Array(items) => {
                let index = segment.parse::<usize>().map_err(|_| {
                    TemplateResolutionError::new(
                        reference,
                        TemplateResolutionErrorKind::InvalidArrayIndex {
                            segment: segment.to_string(),
                            collection_reference: current_reference.clone(),
                        },
                    )
                })?;
                if items.is_empty() {
                    return Err(TemplateResolutionError::new(
                        reference,
                        TemplateResolutionErrorKind::EmptyArrayIndexed {
                            collection_reference: current_reference,
                        },
                    ));
                }
                items.get(index).ok_or_else(|| {
                    TemplateResolutionError::new(
                        reference,
                        TemplateResolutionErrorKind::ArrayIndexOutOfBounds {
                            index,
                            len: items.len(),
                            collection_reference: current_reference.clone(),
                        },
                    )
                })?
            }
            value => {
                return Err(TemplateResolutionError::new(
                    reference,
                    TemplateResolutionErrorKind::TypeMismatch {
                        segment: segment.to_string(),
                        container_reference: current_reference,
                        actual: json_value_kind(value),
                    },
                ));
            }
        };
        current_reference.push('.');
        current_reference.push_str(segment);
    }
    Ok(current.clone())
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn resolve_template_to_string(
    template: &str,
    memory: &HashMap<String, Value>,
) -> Result<String, WorkflowRuntimeError> {
    match resolve_template(template, memory)? {
        Value::String(value) => Ok(value),
        value => Ok(value.to_string()),
    }
}

fn resolve_json_templates(
    value: &Value,
    memory: &HashMap<String, Value>,
) -> Result<Value, WorkflowRuntimeError> {
    match value {
        Value::String(text) => resolve_template(text, memory),
        Value::Array(items) => items
            .iter()
            .map(|item| resolve_json_templates(item, memory))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(object) => object
            .iter()
            .map(|(key, value)| Ok((key.clone(), resolve_json_templates(value, memory)?)))
            .collect::<Result<Map<_, _>, WorkflowRuntimeError>>()
            .map(Value::Object),
        value => Ok(value.clone()),
    }
}

#[derive(Debug)]
struct LegacyEmptyCompletion {
    reference: String,
    collection_reference: String,
    consumer_node_id: String,
}

fn legacy_unguarded_empty_completion(
    compiled: &CompiledWorkflow,
    producer_node_id: &str,
    memory: &HashMap<String, Value>,
    incoming: &HashMap<&str, Vec<&WorkflowEdge>>,
    outgoing: &HashMap<&str, Vec<&WorkflowEdge>>,
    node_by_id: &HashMap<&str, &WorkflowNode>,
) -> Option<LegacyEmptyCompletion> {
    let producer = *node_by_id.get(producer_node_id)?;
    let WorkflowNode::McpTool(producer_tool) = producer else {
        return None;
    };

    let canonical_output = format!("nodes.{producer_node_id}.output");
    let shorthand_output = format!("{producer_node_id}.output");
    let canonical_root = format!("{canonical_output}.");
    let shorthand_root = format!("{shorthand_output}.");
    for consumer in &compiled.workflow_ir.nodes {
        if consumer.id() == producer_node_id
            || !has_linear_unguarded_path(
                producer_node_id,
                consumer.id(),
                incoming,
                outgoing,
                node_by_id,
            )
        {
            continue;
        }
        let templates = runtime_node_templates(compiled, consumer);
        if linear_prefix_has_nonempty_primary_collection(
            producer_node_id,
            &templates,
            memory,
            incoming,
            node_by_id,
        ) {
            continue;
        }
        if path_crosses_unexecuted_referenced_collection(
            producer_node_id,
            consumer.id(),
            &templates,
            memory,
            outgoing,
            node_by_id,
        ) {
            continue;
        }
        for template in templates {
            for reference in template_references(&template) {
                if reference != canonical_output
                    && reference != shorthand_output
                    && !reference.starts_with(&canonical_root)
                    && !reference.starts_with(&shorthand_root)
                {
                    continue;
                }
                let Some(collection_reference) = legacy_empty_collection_reference(
                    producer_tool,
                    producer_node_id,
                    &reference,
                    memory,
                ) else {
                    continue;
                };
                if !empty_collection_matches_producer_contract(
                    producer_tool,
                    producer_node_id,
                    &collection_reference,
                    memory,
                ) {
                    continue;
                }
                return Some(LegacyEmptyCompletion {
                    reference,
                    collection_reference,
                    consumer_node_id: consumer.id().to_string(),
                });
            }
        }
    }
    None
}

fn legacy_empty_collection_reference(
    producer: &McpToolNode,
    producer_node_id: &str,
    reference: &str,
    memory: &HashMap<String, Value>,
) -> Option<String> {
    match resolve_memory_reference(reference, memory) {
        Err(TemplateResolutionError {
            kind:
                TemplateResolutionErrorKind::EmptyArrayIndexed {
                    collection_reference,
                },
            ..
        }) => Some(collection_reference),
        Ok(Value::Array(items)) if items.is_empty() => {
            normalize_legacy_collection_reference(producer_node_id, reference)
        }
        Ok(Value::Object(_)) if is_whole_producer_output_reference(producer_node_id, reference) => {
            empty_primary_collection_reference(producer, producer_node_id, memory)
        }
        Ok(_) | Err(_) => None,
    }
}

fn is_whole_producer_output_reference(producer_node_id: &str, reference: &str) -> bool {
    reference == format!("nodes.{producer_node_id}.output")
        || reference == format!("{producer_node_id}.output")
}

fn empty_primary_collection_reference(
    producer: &McpToolNode,
    producer_node_id: &str,
    memory: &HashMap<String, Value>,
) -> Option<String> {
    if producer.output_schema.is_none() && !is_authoritative_legacy_collection_reader(producer) {
        return None;
    }
    primary_collection_evidence(producer, producer_node_id, memory)
        .filter(|evidence| evidence.is_empty)
        .map(|evidence| evidence.reference)
}

fn is_authoritative_legacy_collection_reader(producer: &McpToolNode) -> bool {
    matches!(
        (producer.server_name.as_str(), producer.tool_name.as_str()),
        ("taskflow_native", "folder_read")
            | ("local_filesystem", "list_directory" | "read_file")
            | (
                "macos_applescript",
                "read_system_calendar"
                    | "read_system_contacts"
                    | "read_system_emails"
                    | "read_system_music"
                    | "read_system_notes"
                    | "read_system_photos"
                    | "read_system_reminders"
            )
    )
}

struct PrimaryCollectionEvidence {
    reference: String,
    is_empty: bool,
}

fn primary_collection_evidence(
    producer: &McpToolNode,
    producer_node_id: &str,
    memory: &HashMap<String, Value>,
) -> Option<PrimaryCollectionEvidence> {
    if let Some(output_schema) = producer.output_schema.as_ref() {
        let path = declared_primary_collection_path(output_schema)?;
        let reference = format!("nodes.{producer_node_id}.output.data.{path}");
        let value = resolve_memory_reference(&reference, memory).ok()?;
        let items = value.as_array()?;
        return Some(PrimaryCollectionEvidence {
            reference,
            is_empty: items.is_empty(),
        });
    }
    let output = memory
        .get(&format!("nodes.{producer_node_id}.output"))
        .or_else(|| memory.get(&format!("{producer_node_id}.output")))?;
    let structured_content = output.pointer("/data/structuredContent")?.as_object()?;
    let mut array_fields = structured_content
        .iter()
        .filter_map(|(field, value)| value.as_array().map(|items| (field, items)));
    let (field, items) = array_fields.next()?;
    if array_fields.next().is_some() {
        return None;
    }
    Some(PrimaryCollectionEvidence {
        reference: format!("nodes.{producer_node_id}.output.data.structuredContent.{field}"),
        is_empty: items.is_empty(),
    })
}

fn consumer_referenced_primary_collection_evidence(
    producer: &McpToolNode,
    producer_node_id: &str,
    consumer_templates: &[String],
    memory: &HashMap<String, Value>,
) -> Option<PrimaryCollectionEvidence> {
    let evidence = primary_collection_evidence(producer, producer_node_id, memory)?;
    if templates_reference_collection(consumer_templates, producer_node_id, &evidence.reference)
        || ((producer.output_schema.is_some()
            || is_authoritative_legacy_collection_reader(producer))
            && templates_reference_whole_producer_output(consumer_templates, producer_node_id))
    {
        return Some(evidence);
    }
    None
}

fn templates_reference_whole_producer_output(templates: &[String], producer_node_id: &str) -> bool {
    let canonical_output = format!("nodes.{producer_node_id}.output");
    let shorthand_output = format!("{producer_node_id}.output");
    templates.iter().any(|template| {
        template_references(template)
            .into_iter()
            .any(|reference| reference == canonical_output || reference == shorthand_output)
    })
}

fn templates_reference_collection(
    templates: &[String],
    producer_node_id: &str,
    collection_reference: &str,
) -> bool {
    let mut aliases = vec![collection_reference.to_string()];
    if let Some(shorthand) = collection_reference.strip_prefix("nodes.") {
        aliases.push(shorthand.to_string());
    }
    let canonical_structured_prefix =
        format!("nodes.{producer_node_id}.output.data.structuredContent.");
    let shorthand_structured_prefix = format!("{producer_node_id}.output.data.structuredContent.");
    if let Some(field) = collection_reference
        .strip_prefix(&canonical_structured_prefix)
        .or_else(|| collection_reference.strip_prefix(&shorthand_structured_prefix))
        .filter(|field| !field.is_empty() && !field.contains('.'))
    {
        aliases.push(format!("nodes.{producer_node_id}.output.{field}"));
        aliases.push(format!("{producer_node_id}.output.{field}"));
    }
    templates.iter().any(|template| {
        template_references(template).into_iter().any(|reference| {
            aliases
                .iter()
                .any(|alias| reference == *alias || reference.starts_with(&format!("{alias}.")))
        })
    })
}

fn linear_prefix_has_nonempty_primary_collection(
    producer_node_id: &str,
    consumer_templates: &[String],
    memory: &HashMap<String, Value>,
    incoming: &HashMap<&str, Vec<&WorkflowEdge>>,
    node_by_id: &HashMap<&str, &WorkflowNode>,
) -> bool {
    let mut current = producer_node_id;
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(current) {
            return true;
        }
        let Some(edges) = incoming.get(current) else {
            return false;
        };
        if edges.len() != 1 {
            return true;
        }
        let previous = edges[0].source_node_id.as_str();
        if let Some(WorkflowNode::McpTool(previous_tool)) = node_by_id.get(previous).copied() {
            if consumer_referenced_primary_collection_evidence(
                previous_tool,
                previous,
                consumer_templates,
                memory,
            )
            .is_some_and(|evidence| !evidence.is_empty)
            {
                return true;
            }
        }
        current = previous;
    }
}

fn path_crosses_unexecuted_referenced_collection(
    producer_node_id: &str,
    consumer_node_id: &str,
    consumer_templates: &[String],
    memory: &HashMap<String, Value>,
    outgoing: &HashMap<&str, Vec<&WorkflowEdge>>,
    node_by_id: &HashMap<&str, &WorkflowNode>,
) -> bool {
    let mut current = producer_node_id;
    let mut visited = HashSet::new();
    while current != consumer_node_id {
        if !visited.insert(current) {
            return true;
        }
        let Some(edges) = outgoing.get(current).filter(|edges| edges.len() == 1) else {
            return true;
        };
        let next = edges[0].target_node_id.as_str();
        if next != consumer_node_id {
            if let Some(WorkflowNode::McpTool(tool)) = node_by_id.get(next).copied() {
                let can_produce_collection = tool.output_schema.as_ref().map_or_else(
                    || {
                        is_authoritative_legacy_collection_reader(tool)
                            || templates_reference_non_whole_producer_output(
                                consumer_templates,
                                next,
                            )
                    },
                    |schema| declared_primary_collection_path(schema).is_some(),
                );
                let has_executed = memory.contains_key(&format!("nodes.{next}.output"))
                    || memory.contains_key(&format!("{next}.output"));
                if can_produce_collection
                    && !has_executed
                    && templates_reference_producer(consumer_templates, next)
                {
                    return true;
                }
            }
        }
        current = next;
    }
    false
}

fn templates_reference_producer(templates: &[String], producer_node_id: &str) -> bool {
    let canonical_output = format!("nodes.{producer_node_id}.output");
    let shorthand_output = format!("{producer_node_id}.output");
    templates.iter().any(|template| {
        template_references(template).into_iter().any(|reference| {
            reference == canonical_output
                || reference == shorthand_output
                || reference.starts_with(&format!("{canonical_output}."))
                || reference.starts_with(&format!("{shorthand_output}."))
        })
    })
}

fn templates_reference_non_whole_producer_output(
    templates: &[String],
    producer_node_id: &str,
) -> bool {
    let canonical_output = format!("nodes.{producer_node_id}.output");
    let shorthand_output = format!("{producer_node_id}.output");
    templates.iter().any(|template| {
        template_references(template).into_iter().any(|reference| {
            reference.starts_with(&format!("{canonical_output}."))
                || reference.starts_with(&format!("{shorthand_output}."))
        })
    })
}

fn normalize_legacy_collection_reference(
    producer_node_id: &str,
    reference: &str,
) -> Option<String> {
    let canonical_root = format!("nodes.{producer_node_id}.output.");
    let shorthand_root = format!("{producer_node_id}.output.");
    let (root, suffix) = reference
        .strip_prefix(&canonical_root)
        .map(|suffix| (canonical_root.as_str(), suffix))
        .or_else(|| {
            reference
                .strip_prefix(&shorthand_root)
                .map(|suffix| (shorthand_root.as_str(), suffix))
        })?;
    if suffix.is_empty() {
        return None;
    }
    if suffix.starts_with("data.") {
        return Some(reference.to_string());
    }
    Some(format!("{root}data.structuredContent.{suffix}"))
}

fn empty_collection_matches_producer_contract(
    producer: &McpToolNode,
    producer_node_id: &str,
    collection_reference: &str,
    memory: &HashMap<String, Value>,
) -> bool {
    match producer.output_schema.as_ref() {
        Some(output_schema) => {
            declared_primary_collection_path(output_schema).is_some_and(|path| {
                collection_reference == format!("nodes.{producer_node_id}.output.data.{path}")
                    || collection_reference == format!("{producer_node_id}.output.data.{path}")
            })
        }
        None => {
            schema_less_top_level_empty_collection(producer_node_id, collection_reference, memory)
        }
    }
}

fn declared_primary_collection_path(output_schema: &Value) -> Option<String> {
    let contract = output_schema.get("x-oomu-result-contract")?;
    if contract.get("kind").and_then(Value::as_str) != Some("collection")
        || contract.get("emptyIsSuccess").and_then(Value::as_bool) != Some(true)
    {
        return None;
    }
    let segments = contract
        .get("path")?
        .as_str()?
        .trim()
        .strip_prefix('/')?
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
        .collect::<Vec<_>>();
    (!segments.is_empty()).then(|| segments.join("."))
}

fn schema_less_top_level_empty_collection(
    producer_node_id: &str,
    collection_reference: &str,
    memory: &HashMap<String, Value>,
) -> bool {
    let canonical_prefix = format!("nodes.{producer_node_id}.output.data.structuredContent.");
    let shorthand_prefix = format!("{producer_node_id}.output.data.structuredContent.");
    let Some(field) = collection_reference
        .strip_prefix(&canonical_prefix)
        .or_else(|| collection_reference.strip_prefix(&shorthand_prefix))
        .filter(|field| !field.is_empty() && !field.contains('.'))
    else {
        return false;
    };
    let canonical_output_key = format!("nodes.{producer_node_id}.output");
    let shorthand_output_key = format!("{producer_node_id}.output");
    let Some(structured_content) = memory
        .get(&canonical_output_key)
        .or_else(|| memory.get(&shorthand_output_key))
        .and_then(|output| output.pointer("/data/structuredContent"))
        .and_then(Value::as_object)
    else {
        return false;
    };
    let mut array_fields = structured_content
        .iter()
        .filter_map(|(name, value)| value.as_array().map(|items| (name.as_str(), items)));
    let Some((array_field, items)) = array_fields.next() else {
        return false;
    };
    array_fields.next().is_none() && array_field == field && items.is_empty()
}

fn has_linear_unguarded_path(
    producer_node_id: &str,
    consumer_node_id: &str,
    incoming: &HashMap<&str, Vec<&WorkflowEdge>>,
    outgoing: &HashMap<&str, Vec<&WorkflowEdge>>,
    node_by_id: &HashMap<&str, &WorkflowNode>,
) -> bool {
    let mut current = producer_node_id;
    let mut visited = HashSet::new();
    while current != consumer_node_id {
        if !visited.insert(current) {
            return false;
        }
        let Some(edges) = outgoing.get(current).filter(|edges| edges.len() == 1) else {
            return false;
        };
        let next = edges[0].target_node_id.as_str();
        if incoming.get(next).map_or(0, Vec::len) != 1 {
            return false;
        }
        let Some(node) = node_by_id.get(next) else {
            return false;
        };
        if matches!(
            node,
            WorkflowNode::Conditional(_) | WorkflowNode::Router(_) | WorkflowNode::Loop(_)
        ) {
            return false;
        }
        current = next;
    }
    true
}

fn runtime_node_templates(compiled: &CompiledWorkflow, node: &WorkflowNode) -> Vec<String> {
    match node {
        WorkflowNode::Agent(agent) => compiled
            .instructions
            .get(&agent.id)
            .map(|instruction| {
                instruction
                    .input_variable_mappings
                    .values()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default(),
        WorkflowNode::Conditional(conditional) => conditional
            .input_mapping
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        WorkflowNode::Loop(loop_node) => vec![loop_node.items_mapping.clone()],
        WorkflowNode::McpTool(mcp_tool) => {
            let mut templates = Vec::new();
            collect_json_template_strings(&mcp_tool.arguments, &mut templates);
            templates
        }
        WorkflowNode::SystemAction(system_action) => {
            let mut templates = vec![system_action.command.clone()];
            templates.extend(system_action.args.iter().cloned());
            templates.extend(system_action.working_directory.iter().cloned());
            templates
        }
        WorkflowNode::Output(output) => vec![output.input_mapping.clone()],
        WorkflowNode::Input(_) | WorkflowNode::Router(_) | WorkflowNode::Permission(_) => {
            Vec::new()
        }
    }
}

fn collect_json_template_strings(value: &Value, templates: &mut Vec<String>) {
    match value {
        Value::String(template) => templates.push(template.clone()),
        Value::Array(items) => {
            for item in items {
                collect_json_template_strings(item, templates);
            }
        }
        Value::Object(object) => {
            for value in object.values() {
                collect_json_template_strings(value, templates);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn template_references(template: &str) -> Vec<String> {
    let Ok(expression) = Regex::new(r"\{\{\s*([^{}]+?)\s*\}\}") else {
        return Vec::new();
    };
    expression
        .captures_iter(template)
        .filter_map(|capture| {
            capture
                .get(1)
                .map(|matched| matched.as_str().trim().to_string())
        })
        .collect()
}

fn reveal_path_in_file_manager(path: &str) -> Result<(), WorkflowRuntimeError> {
    let workspace_root = ensure_workflow_workspace_root()?;
    let path = resolve_workflow_reveal_path(&workspace_root, path)?;

    let status = file_manager_reveal_command(&path)
        .status()
        .map_err(WorkflowRuntimeError::io)?;
    if !status.success() {
        return Err(WorkflowRuntimeError::execution(
            "Could not reveal the selected workflow output.".to_string(),
        ));
    }
    Ok(())
}

fn resolve_workflow_reveal_path(
    workspace_root: &Path,
    requested_path: &str,
) -> Result<PathBuf, WorkflowRuntimeError> {
    let workspace_root = fs::canonicalize(workspace_root).map_err(|_| {
        WorkflowRuntimeError::input("Workflow output storage is unavailable.".to_string())
    })?;
    let requested_path = PathBuf::from(requested_path.trim());
    if requested_path.as_os_str().is_empty() {
        return Err(WorkflowRuntimeError::input(
            "Workflow output selection is required.".to_string(),
        ));
    }
    let candidate = if requested_path.is_absolute() {
        requested_path
    } else {
        workspace_root.join(requested_path)
    };
    let canonical = fs::canonicalize(candidate)
        .map_err(|_| WorkflowRuntimeError::input("Workflow output is unavailable.".to_string()))?;
    if !canonical.starts_with(&workspace_root) {
        return Err(WorkflowRuntimeError::permission_rejected(
            "Workflow output selection is outside app-owned output storage.",
        ));
    }
    if !fs::symlink_metadata(&canonical)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return Err(WorkflowRuntimeError::input(
            "Workflow output selection is not a file.".to_string(),
        ));
    }
    Ok(canonical)
}

#[cfg(target_os = "macos")]
fn file_manager_reveal_command(path: &Path) -> Command {
    let mut process = Command::new("open");
    process.arg("-R").arg(path);
    process
}

#[cfg(target_os = "windows")]
fn file_manager_reveal_command(path: &Path) -> Command {
    let mut process = Command::new("explorer");
    process.arg(format!("/select,{}", path.display()));
    process
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn file_manager_reveal_command(path: &Path) -> Command {
    let mut process = Command::new("xdg-open");
    process.arg(path.parent().unwrap_or(path));
    process
}

fn topological_sort(ir: &WorkflowIr) -> Result<Vec<String>, WorkflowRuntimeError> {
    let original_order = ir
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id(), index))
        .collect::<HashMap<_, _>>();
    let mut indegree = ir
        .nodes
        .iter()
        .map(|node| (node.id().to_string(), 0usize))
        .collect::<HashMap<_, _>>();
    let mut outgoing = HashMap::<&str, Vec<&WorkflowEdge>>::new();
    for edge in &ir.edges {
        *indegree.entry(edge.target_node_id.clone()).or_default() += 1;
        outgoing
            .entry(edge.source_node_id.as_str())
            .or_default()
            .push(edge);
    }
    let mut ready = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    ready.sort_by_key(|id| original_order[id.as_str()]);
    let mut queue = VecDeque::from(ready);
    let mut result = Vec::with_capacity(ir.nodes.len());
    while let Some(id) = queue.pop_front() {
        result.push(id.clone());
        let mut newly_ready = Vec::new();
        for edge in outgoing.get(id.as_str()).into_iter().flatten() {
            let degree = indegree.get_mut(&edge.target_node_id).ok_or_else(|| {
                WorkflowRuntimeError::invalid_ir(vec![format!(
                    "edge target {} is missing from the workflow graph",
                    edge.target_node_id
                )])
            })?;
            *degree -= 1;
            if *degree == 0 {
                newly_ready.push(edge.target_node_id.clone());
            }
        }
        newly_ready.sort_by_key(|node_id| original_order[node_id.as_str()]);
        queue.extend(newly_ready);
    }
    if result.len() != ir.nodes.len() {
        return Err(WorkflowRuntimeError::execution(
            "Workflow graph is not acyclic.".to_string(),
        ));
    }
    Ok(result)
}

fn edges_by_target(edges: &[WorkflowEdge]) -> HashMap<&str, Vec<&WorkflowEdge>> {
    let mut result = HashMap::new();
    for edge in edges {
        result
            .entry(edge.target_node_id.as_str())
            .or_insert_with(Vec::new)
            .push(edge);
    }
    result
}

fn edges_by_source(edges: &[WorkflowEdge]) -> HashMap<&str, Vec<&WorkflowEdge>> {
    let mut result = HashMap::new();
    for edge in edges {
        result
            .entry(edge.source_node_id.as_str())
            .or_insert_with(Vec::new)
            .push(edge);
    }
    result
}

fn incoming_payload(
    node_id: &str,
    incoming: &HashMap<&str, Vec<&WorkflowEdge>>,
    selected_edges: &HashSet<String>,
    payloads: &HashMap<String, NodeExecutionPayload>,
) -> Option<Value> {
    let values = incoming
        .get(node_id)
        .into_iter()
        .flatten()
        .filter(|edge| selected_edges.contains(&edge.id))
        .filter_map(|edge| payloads.get(&edge.source_node_id)?.output.clone())
        .collect::<Vec<_>>();
    match values.len() {
        0 => None,
        1 => values.into_iter().next(),
        _ => Some(Value::Array(values)),
    }
}

fn parse_model_boolean(value: &str) -> Option<bool> {
    let normalized = value
        .trim()
        .trim_matches('"')
        .trim_matches('.')
        .to_ascii_lowercase();
    match normalized.as_str() {
        "true" | "yes" | "matched" | "pass" | "passed" => Some(true),
        "false" | "no" | "not_matched" | "fail" | "failed" => Some(false),
        _ => None,
    }
}

fn route_for_boolean(router: &RouterNode, result: bool) -> Result<String, WorkflowRuntimeError> {
    let preferred = if result {
        ["matched", "true", "yes"]
    } else {
        ["not_matched", "false", "no"]
    };
    router
        .routes
        .iter()
        .find(|route| preferred.contains(&route.port.as_str()))
        .or_else(|| router.routes.get(usize::from(!result)))
        .map(|route| route.port.clone())
        .ok_or_else(|| {
            WorkflowRuntimeError::execution(format!(
                "Router {} has no route for boolean result {result}.",
                router.id
            ))
        })
}

fn new_instance(
    ir: &WorkflowIr,
    request: &RunWorkflowRequest,
) -> Result<ExecutionInstance, WorkflowRuntimeError> {
    let created_at_ms = unix_time_ms();
    let input_payload =
        serde_json::to_value(request).map_err(WorkflowRuntimeError::serialization)?;
    Ok(ExecutionInstance {
        id: instance_id(&ir.workflow_id, ir.workflow_version, created_at_ms),
        workflow_id: ir.workflow_id.clone(),
        workflow_version: ir.workflow_version,
        status: ExecutionStatus::Pending,
        active_node_id: None,
        input_payload,
        output_payload: None,
        node_payloads: HashMap::new(),
        memory: HashMap::new(),
        selected_edges: HashSet::new(),
        pause_context: None,
        error: None,
        execution_latency_ms: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        created_at_ms,
        started_at_ms: None,
        updated_at_ms: created_at_ms,
        completed_at_ms: None,
    })
}

fn validate_request(request: &RunWorkflowRequest) -> Result<(), WorkflowRuntimeError> {
    if request.workflow_id.trim().is_empty() {
        return Err(WorkflowRuntimeError::input(
            "workflowId must not be empty.".to_string(),
        ));
    }
    if request.workflow_version == Some(0) {
        return Err(WorkflowRuntimeError::input(
            "workflowVersion must be greater than zero.".to_string(),
        ));
    }
    Ok(())
}

fn finish_timing(instance: &mut ExecutionInstance, completed: bool) {
    let now = unix_time_ms();
    instance.updated_at_ms = now;
    instance.execution_latency_ms = instance
        .started_at_ms
        .map(|started| now.saturating_sub(started) as u64)
        .unwrap_or_default();
    if completed {
        instance.completed_at_ms = Some(now);
    }
}

fn instance_id(workflow_id: &str, version: u32, created_at_ms: i64) -> String {
    let mut hasher = Sha256::new();
    let sequence = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    hasher.update(format!("{workflow_id}:{version}:{created_at_ms}:{sequence}").as_bytes());
    format!("wfi-{}", hex::encode(hasher.finalize()))
}

pub fn workflow_workspace_root() -> PathBuf {
    crate::settings::app_data_root().join(WORKFLOW_WORKSPACE_DIR)
}

fn ensure_workflow_workspace_root() -> Result<PathBuf, WorkflowRuntimeError> {
    let root = workflow_workspace_root();
    fs::create_dir_all(&root).map_err(WorkflowRuntimeError::io)?;
    fs::canonicalize(&root).map_err(WorkflowRuntimeError::io)
}

pub fn canonicalize_and_validate_path(
    base_root: &Path,
    user_path: &Path,
) -> Result<PathBuf, String> {
    let base_root = fs::canonicalize(base_root).map_err(|error| {
        format!(
            "Failed path canonicalization for sandbox root {}: {error}",
            base_root.display()
        )
    })?;
    if is_sensitive_path(&base_root) {
        return Err(format!(
            "Access Denied: sandbox root {} is a sensitive system directory.",
            base_root.display()
        ));
    }

    let absolute_path = if user_path.is_relative() {
        base_root.join(user_path)
    } else {
        user_path.to_path_buf()
    };
    let canonical = resolve_existing_path_prefix(&absolute_path).map_err(|error| {
        format!(
            "Failed path canonicalization for {}: {error}",
            absolute_path.display()
        )
    })?;

    if is_sensitive_path(&canonical) {
        return Err(format!(
            "Access Denied: sensitive path {} is not available to workflows.",
            canonical.display()
        ));
    }
    if !path_within_root(&canonical, &base_root) {
        return Err(format!(
            "Access Denied: Path escape detected outside secure sandbox root {}.",
            base_root.display()
        ));
    }

    Ok(canonical)
}

fn validate_workflow_output_directory(
    workspace_root: &Path,
    directory: &Path,
) -> Result<PathBuf, WorkflowRuntimeError> {
    let directory = canonicalize_and_validate_path(workspace_root, directory)
        .map_err(|error| WorkflowRuntimeError::permission_rejected(&error))?;
    fs::create_dir_all(&directory).map_err(WorkflowRuntimeError::io)?;
    canonicalize_and_validate_path(workspace_root, &directory)
        .map_err(|error| WorkflowRuntimeError::permission_rejected(&error))
}

pub fn is_workflow_environment_allowlisted(name: &str) -> bool {
    OOMU_ENV_ALLOWLIST.contains(&name)
}

pub fn resolve_workflow_environment_value(name: &str) -> Result<String, WorkflowRuntimeError> {
    if !is_workflow_environment_allowlisted(name) {
        eprintln!("HIGH-ALERT workflow sandbox violation: denied environment binding for {name}");
        return Err(WorkflowRuntimeError::permission_rejected(&format!(
            "Environment variable {name} is not allowlisted for workflow bindings."
        )));
    }
    env::var(name).map_err(|_| {
        WorkflowRuntimeError::input(format!("Environment variable {name} is not available."))
    })
}

fn resolve_existing_path_prefix(candidate: &Path) -> Result<PathBuf, String> {
    if let Ok(real) = fs::canonicalize(candidate) {
        return Ok(real);
    }

    let mut ancestor = candidate;
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or_else(|| {
            format!(
                "Unable to resolve path {} because no parent exists.",
                candidate.display()
            )
        })?;
    }

    let mut resolved = fs::canonicalize(ancestor).map_err(|error| {
        format!(
            "Unable to resolve existing path prefix {}: {error}",
            ancestor.display()
        )
    })?;
    let remainder = candidate
        .strip_prefix(ancestor)
        .unwrap_or_else(|_| Path::new(""));
    for component in remainder.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            std::path::Component::Normal(part) => resolved.push(part),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {}
        }
    }
    Ok(resolved)
}

fn path_within_root(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn is_sensitive_path(path: &Path) -> bool {
    sensitive_roots()
        .iter()
        .any(|root| path == root || path.starts_with(root))
}

fn sensitive_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/etc"),
        PathBuf::from("/private/etc"),
        PathBuf::from("/var/audit"),
        PathBuf::from("/private/var/audit"),
        PathBuf::from("/var/db"),
        PathBuf::from("/private/var/db"),
        PathBuf::from("/var/log"),
        PathBuf::from("/private/var/log"),
        PathBuf::from("/var/root"),
        PathBuf::from("/private/var/root"),
        PathBuf::from("/var/run"),
        PathBuf::from("/private/var/run"),
    ];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".aws"));
        roots.push(home.join(".ssh"));
    }
    roots
}

fn sanitize_file_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

impl WorkflowRuntimeError {
    fn input(message: String) -> Self {
        Self::new("workflow_runtime_input_invalid", message)
    }

    fn invalid_ir(errors: Vec<String>) -> Self {
        Self::new("workflow_runtime_ir_invalid", errors.join("; "))
    }

    fn database(error: rusqlite::Error) -> Self {
        Self::new("workflow_runtime_database_failed", error.to_string())
    }

    fn serialization(error: serde_json::Error) -> Self {
        Self::new("workflow_runtime_serialization_failed", error.to_string())
    }

    fn inference(error: crate::gemma::GemmaError) -> Self {
        Self::new(error.code, error.message)
    }

    fn io(error: std::io::Error) -> Self {
        Self::new("workflow_runtime_io_failed", error.to_string())
    }

    fn execution(message: String) -> Self {
        Self::new("workflow_runtime_execution_failed", message)
    }

    fn template_resolution(error: TemplateResolutionError) -> Self {
        let empty_collection_indexed = matches!(
            &error.kind,
            TemplateResolutionErrorKind::EmptyArrayIndexed { .. }
        );
        let message = error.message();
        if empty_collection_indexed {
            return Self::new("workflow_runtime_empty_collection_indexed", message);
        }
        Self::execution(message)
    }

    fn mcp_server_unreachable(server_name: &str, message: String) -> Self {
        let detail = message.trim().trim_end_matches('.');
        let message = if detail.is_empty() {
            format!(
                "MCP Server '{server_name}' is offline or unreachable. Restart the MCP server and run the workflow again."
            )
        } else {
            format!(
                "MCP Server '{server_name}' is offline or unreachable: {detail}. Restart the MCP server and run the workflow again."
            )
        };
        Self::new("workflow_runtime_mcp_preflight_failed", message)
    }

    fn mcp_sandbox(message: String) -> Self {
        Self::new(
            "workflow_runtime_mcp_sandbox_unavailable",
            format!(
                "MCP sandbox pre-initialization failed: {message}. The workflow was halted before any nodes executed."
            ),
        )
    }

    pub(crate) fn runtime(message: String) -> Self {
        Self::new("workflow_runtime_worker_failed", message)
    }

    fn node_timeout(node_id: &str, label: &str, timeout_ms: u64) -> Self {
        Self::new(
            "workflow_runtime_node_timeout",
            format!("Node Execution Timed Out: node {node_id} ({label}) exceeded {timeout_ms}ms."),
        )
    }

    fn notification_unavailable() -> Self {
        Self::new(
            "workflow_runtime_notification_unavailable",
            "Notifications are off for OOMU. Turn them on in System Settings, then try again."
                .to_string(),
        )
    }

    fn calendar_read(result: &McpToolCallResult) -> Self {
        let structured = result.structured_content.as_ref();
        let native_code = structured
            .and_then(|value| value.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("calendar_read_failed");
        let message = structured
            .and_then(|value| value.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Calendar could not be read.");
        let code = match native_code {
            "calendar_permission_denied"
            | "calendar_permission_restricted"
            | "calendar_permission_write_only"
            | "calendar_permission_unavailable"
            | "calendar_authorization_timeout" => "workflow_runtime_calendar_permission",
            "calendar_read_timeout" => "workflow_runtime_calendar_timeout",
            "calendar_not_found" => "workflow_runtime_calendar_not_found",
            _ => "workflow_runtime_calendar_unavailable",
        };
        Self::new(code, message.to_string())
    }

    fn permission_rejected(reason: &str) -> Self {
        Self::new(
            "workflow_runtime_permission_rejected",
            format!("Permission rejected: {reason}"),
        )
    }

    fn approval_unauthorized() -> Self {
        Self::new(
            "workflow_runtime_approval_unauthorized",
            "The approval token is invalid.".to_string(),
        )
    }

    fn approval_consumed() -> Self {
        Self::new(
            "workflow_runtime_approval_consumed",
            "This approval request is no longer pending.".to_string(),
        )
    }

    fn approval_state_invalid() -> Self {
        Self::new(
            "workflow_runtime_approval_state_invalid",
            "The paused execution is missing approval state.".to_string(),
        )
    }

    fn approval_not_verified(node_id: &str, tool_name: &str) -> Self {
        Self::new(
            "workflow_runtime_approval_not_verified",
            format!(
                "No valid workflow approval ledger entry exists for node {node_id} and tool {tool_name}."
            ),
        )
    }

    fn new(code: &'static str, message: String) -> Self {
        Self {
            code,
            boundary: "workflow_runtime",
            message,
            instance_id: None,
        }
    }
}

#[cfg(test)]
mod tests;
