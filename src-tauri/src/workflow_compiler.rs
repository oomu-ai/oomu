#[cfg(test)]
use crate::workflow_ir::WorkflowEdge;
mod compiler_error;
mod compose_output_status;
mod composition_runtime; // Keep bounded execution isolated from core compiler validation.
mod effect_request_validation;
mod instruction_compiler_runtime;
mod registered_task_capabilities;
mod specialist_composer;
mod specialist_execution;
#[path = "workflow_prompts.rs"]
mod workflow_prompts;
#[path = "workflow_provenance.rs"]
mod workflow_provenance;

use self::{
    workflow_prompts::{
        WORKFLOW_COMPILER_SYSTEM_PROMPT, WORKFLOW_COMPOSE_SYSTEM_PROMPT,
        WORKFLOW_EDIT_SYSTEM_PROMPT,
    },
    workflow_provenance::build_workflow_artifact_provenance,
};
use crate::{
    db::{PersistenceEngine, SavedWorkflowRecord},
    foundation::{clock::unix_time_ms_i64 as unix_time_ms, digest::sha256_hex},
    gemma::{
        format_structured_runtime_prompt, GemmaError, GemmaService, InferRequest,
        PREFERRED_LOCAL_MODEL_ID,
    },
    local_app_intent::has_local_app_intent,
    mcp::{
        client::{McpClientRegistry, McpTool},
        taskflow::native_taskflow_tools,
    },
    sovereign_identity::SovereignIdentity,
    workflow_ir::{
        CompiledInstruction, McpToolNode, WorkflowBlueprint, WorkflowCompletionKind, WorkflowIr,
        WorkflowNode, WorkflowNodeKind, WORKFLOW_COMPILER_MODEL,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    any::Any,
    collections::{HashMap, HashSet},
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Component, Path},
    sync::{atomic::AtomicBool, Arc},
};

use compiler_error::compact_error;
use composition_runtime::{
    compose_disabled_response, compose_infer_request, run_bounded_workflow_compiler,
};
use instruction_compiler_runtime::{compiler_infer_request, deterministic_instruction_free_output};
use specialist_execution::{
    compile_registered_specialist_instructions, specialist_compose_response,
};

const COMPILER_VERSION: &str = "1.0.0";
const MAX_REPAIR_ATTEMPTS: usize = 2;
const COMPOSE_MAX_REPAIR_ATTEMPTS: usize = 1;
const WORKFLOW_COMPILER_RUNTIME_MODEL_ID: &str = PREFERRED_LOCAL_MODEL_ID;
const WORKFLOW_COMPILER_CONTEXT_SIZE: u32 = 8_192;
const WORKFLOW_INSTRUCTION_COMPILER_MAX_NEW_TOKENS: usize = 4_096;
const WORKFLOW_AUTHORING_P0_ENV: &str = "OOMU_WORKFLOW_AUTHORING_P0";
const WORKFLOW_CAPABILITY_CATALOG_VERSION: &str = "2026-06-29.p2";
const LOCAL_MODEL_REPETITION_COLLAPSE_CODE: &str = "local_model_repetition_collapse";
const TASKFLOW_NATIVE_SERVER: &str = "taskflow_native";
const TASKFLOW_DEFAULT_REPORT_PATH: &str = "workspace/report.md";
const WORKFLOW_TOPOLOGY_MISSING_REPORT_WRITER_CODE: &str =
    "workflow_topological_anomaly_missing_report_writer";
const WORKFLOW_TOPOLOGY_INVALID_SANDBOX_PATH_CODE: &str =
    "workflow_topological_anomaly_invalid_sandbox_path";
const WORKFLOW_TOPOLOGY_UNSAFE_COLLECTION_CODE: &str =
    "workflow_topological_anomaly_unsafe_collection_access";
const WORKFLOW_TOPOLOGY_UNSAFE_REFERENCE_CODE: &str =
    "workflow_topological_anomaly_unsafe_reference";
const MCP_FOLDER_PATH_KEYS: &[&str] = &["folderPath", "folder_path", "folder", "path", "directory"];
const MCP_REPORT_PATH_KEYS: &[&str] =
    &["reportPath", "report_path", "path", "filePath", "file_path"];
const MCP_FILE_PATH_KEYS: &[&str] = &["path", "filePath", "file_path"];
const WORKFLOW_ARTIFACT_PROVENANCE_KEY: &str = "oomuArtifactProvenance";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SaveWorkflowRequest {
    #[serde(default)]
    pub project_id: Option<String>,
    pub workflow: SavedWorkflowRecord,
    pub visual_state: Value,
    pub workflow_ir: WorkflowIr,
    pub activate: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveWorkflowResponse {
    pub workflow_id: String,
    pub workflow_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub compilation_status: &'static str,
    pub compiled_node_count: usize,
    pub review_capabilities: crate::workflow_ir::review::WorkflowReviewCapabilities,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GetCompiledInstructionsRequest {
    pub workflow_id: String,
    #[serde(default)]
    pub workflow_version: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UpdateCompiledInstructionRequest {
    pub workflow_id: String,
    pub workflow_version: u32,
    pub node_id: String,
    pub system_prompt: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCatalog {
    pub version: String,
    pub authoring_enabled: bool,
    pub generated_at_ms: i64,
    pub actions: Vec<CapabilityAction>,
    #[serde(default)]
    pub templates: Vec<CapabilityTemplateExample>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityTemplateExample {
    pub id: String,
    pub name: String,
    pub description: String,
    pub seed_prompt: String,
    pub workflow_ir: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityAction {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub outcome: String,
    pub detail: String,
    pub source: String,
    pub available: bool,
    pub availability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_template: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ComposeWorkflowRequest {
    pub prompt: String,
    pub capability_catalog: CapabilityCatalog,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub workflow_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EditWorkflowRequest {
    pub instruction: String,
    pub workflow_ir: WorkflowIr,
    pub capability_catalog: CapabilityCatalog,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeWorkflowResponse {
    pub status: &'static str,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_ir: Option<WorkflowIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_draft: Option<Value>,
    pub missing_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_capability_details: Vec<MissingCapabilityDetail>,
    pub composed_by: &'static str,
    pub attempts: usize,
    pub latency_ms: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingCapabilityDetail {
    pub id: String,
    pub title: String,
    pub outcome: String,
    pub reason: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawComposeOutput {
    status: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    workflow_ir: Option<Value>,
    #[serde(default)]
    partial_draft: Option<Value>,
    #[serde(default)]
    missing_capabilities: Vec<String>,
}

#[derive(Debug, Clone)]
struct ComposeAttemptError {
    message: String,
    partial_draft: Option<Value>,
    missing_capabilities: Vec<String>,
    missing_capability_details: Vec<MissingCapabilityDetail>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowCompilerError {
    pub code: &'static str,
    pub boundary: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CompilerOutput {
    compiler_version: String,
    instructions: Vec<CompilerInstruction>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CompilerInstruction {
    node_id: String,
    system_prompt: String,
    input_variable_mappings: Vec<VariableMapping>,
    evaluation_protocol: EvaluationProtocol,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct VariableMapping {
    name: String,
    template: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EvaluationProtocol {
    success_criteria: Vec<String>,
    failure_action: FailureAction,
    max_retries: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum FailureAction {
    Fail,
    Retry,
    Route,
}

trait InstructionCompiler {
    fn compile(&self, workflow_ir: &WorkflowIr) -> Result<CompilerOutput, WorkflowCompilerError>;
}

struct GemmaInstructionCompiler {
    gemma: GemmaService,
}

impl InstructionCompiler for GemmaInstructionCompiler {
    fn compile(&self, workflow_ir: &WorkflowIr) -> Result<CompilerOutput, WorkflowCompilerError> {
        if specialist_composer::is_registered_specialist_workflow(workflow_ir) {
            return compile_registered_specialist_instructions(workflow_ir);
        }
        if let Some(output) = deterministic_instruction_free_output(workflow_ir) {
            return Ok(output);
        }
        let compiler_ir = sanitize_workflow_ir_for_compiler(workflow_ir);
        let workflow_json =
            serde_json::to_string(&compiler_ir).map_err(WorkflowCompilerError::serialization)?;
        let mut prompt = format_structured_runtime_prompt(
            WORKFLOW_COMPILER_SYSTEM_PROMPT,
            &format!("Compile this Workflow IR:\n{workflow_json}"),
        );
        prompt.push_str("<|channel>text\n<channel|>");
        let session_id = format!(
            "workflow-compiler:{}:{}",
            workflow_ir.workflow_id, workflow_ir.workflow_version
        );
        let mut response = self
            .gemma
            .infer_model_sync(
                WORKFLOW_COMPILER_RUNTIME_MODEL_ID,
                compiler_infer_request(prompt, &session_id),
            )
            .map_err(WorkflowCompilerError::inference)?;

        for attempt in 0..=MAX_REPAIR_ATTEMPTS {
            match parse_compiler_output(&response.text, workflow_ir) {
                Ok(output) => return Ok(output),
                Err(error) if attempt < MAX_REPAIR_ATTEMPTS => {
                    let repair_prompt = format!(
                        "<|turn>user\nYour prior compiler JSON failed validation: {} Return only a corrected compact JSON object matching the required schema and Workflow IR.<turn|>\n<|turn>model\n",
                        compact_error(&error.message)
                    );
                    response = self
                        .gemma
                        .infer_model_sync(
                            WORKFLOW_COMPILER_RUNTIME_MODEL_ID,
                            compiler_infer_request(repair_prompt, &session_id),
                        )
                        .map_err(WorkflowCompilerError::inference)?;
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("bounded compiler repair loop must return")
    }
}

fn sanitize_workflow_ir_for_compiler(workflow_ir: &WorkflowIr) -> WorkflowIr {
    let mut compiler_ir = workflow_ir.clone();
    prune_workflow_ir_metadata(&mut compiler_ir);
    compiler_ir
}

fn prune_workflow_ir_metadata(ir: &mut WorkflowIr) {
    for node in &mut ir.nodes {
        match node {
            WorkflowNode::Input(input) => {
                input.input_schema = Value::Null;
            }
            WorkflowNode::McpTool(tool) => {
                tool.input_schema = None;
                tool.output_schema = None;
            }
            WorkflowNode::Output(output) => {
                output.output_schema = Value::Null;
            }
            WorkflowNode::Agent(_)
            | WorkflowNode::Router(_)
            | WorkflowNode::Conditional(_)
            | WorkflowNode::Loop(_)
            | WorkflowNode::Permission(_)
            | WorkflowNode::SystemAction(_) => {}
        }
    }
    ir.metadata = None;
}

#[tauri::command]
pub async fn save_workflow(
    request: SaveWorkflowRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    gemma: tauri::State<'_, GemmaService>,
    identity: tauri::State<'_, SovereignIdentity>,
    mcp_registry: tauri::State<'_, McpClientRegistry>,
) -> Result<SaveWorkflowResponse, WorkflowCompilerError> {
    let capability_catalog = build_capability_catalog(&mcp_registry).await?;
    let persistence = persistence.inner().clone();
    let identity = identity.inner().clone();
    let compiler = GemmaInstructionCompiler {
        gemma: gemma.inner().clone(),
    };
    tauri::async_runtime::spawn_blocking(move || {
        run_workflow_compiler_guard("save_workflow", || {
            compile_and_save_workflow(
                request,
                &capability_catalog,
                &persistence,
                &compiler,
                &identity,
            )
        })
    })
    .await
    .map_err(|error| WorkflowCompilerError::runtime(error.to_string()))?
}

#[tauri::command]
pub async fn get_compiled_instructions(
    request: GetCompiledInstructionsRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<CompiledInstruction>, WorkflowCompilerError> {
    if request.workflow_id.trim().is_empty() {
        return Err(WorkflowCompilerError::invalid_request(
            "workflowId must not be empty.",
        ));
    }
    let persistence = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let compiled = persistence
            .load_compiled_workflow(&request.workflow_id, request.workflow_version)
            .map_err(WorkflowCompilerError::database)?;
        let mut instructions = compiled.instructions.into_values().collect::<Vec<_>>();
        instructions.sort_by(|left, right| left.node_id.cmp(&right.node_id));
        Ok(instructions)
    })
    .await
    .map_err(|error| WorkflowCompilerError::runtime(error.to_string()))?
}

#[tauri::command]
pub async fn update_compiled_instruction(
    request: UpdateCompiledInstructionRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<CompiledInstruction, WorkflowCompilerError> {
    if request.workflow_id.trim().is_empty()
        || request.node_id.trim().is_empty()
        || request.system_prompt.trim().is_empty()
        || request.workflow_version == 0
    {
        return Err(WorkflowCompilerError::invalid_request(
            "workflowId, workflowVersion, nodeId, and systemPrompt are required.",
        ));
    }
    let persistence = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        persistence
            .update_compiled_instruction(
                &request.workflow_id,
                request.workflow_version,
                &request.node_id,
                request.system_prompt.trim(),
            )
            .map_err(WorkflowCompilerError::database)
    })
    .await
    .map_err(|error| WorkflowCompilerError::runtime(error.to_string()))?
}

#[tauri::command]
pub async fn get_workflow_irs(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<WorkflowBlueprint>, WorkflowCompilerError> {
    let persistence = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        persistence
            .select_latest_workflow_ir_blueprints()
            .map_err(WorkflowCompilerError::database)
    })
    .await
    .map_err(|error| WorkflowCompilerError::runtime(error.to_string()))?
}

#[tauri::command]
pub async fn get_workflow_capability_catalog(
    mcp_registry: tauri::State<'_, McpClientRegistry>,
) -> Result<CapabilityCatalog, WorkflowCompilerError> {
    build_capability_catalog(&mcp_registry).await
}

#[tauri::command]
pub async fn compose_workflow(
    mut request: ComposeWorkflowRequest,
    gemma: tauri::State<'_, GemmaService>,
    mcp_registry: tauri::State<'_, McpClientRegistry>,
) -> Result<ComposeWorkflowResponse, WorkflowCompilerError> {
    if let Some(project_id) = request.project_id.as_deref() {
        crate::p0_contracts::ProjectId::parse(project_id)
            .map_err(|error| WorkflowCompilerError::invalid_request(&error.to_string()))?;
    }
    if !workflow_authoring_p0_enabled() {
        return Ok(compose_disabled_response());
    }
    let live_catalog = build_capability_catalog(&mcp_registry).await?;
    request.capability_catalog = merge_compose_catalogs(live_catalog, request.capability_catalog);
    let gemma = gemma.inner().clone();
    run_bounded_workflow_compiler("compose", move |cancellation| {
        compose_workflow_sync(request, &gemma, &cancellation)
    })
    .await
}

#[tauri::command]
pub async fn edit_workflow(
    mut request: EditWorkflowRequest,
    gemma: tauri::State<'_, GemmaService>,
    mcp_registry: tauri::State<'_, McpClientRegistry>,
) -> Result<ComposeWorkflowResponse, WorkflowCompilerError> {
    if !workflow_authoring_p0_enabled() {
        return Ok(ComposeWorkflowResponse {
            reason: "Workflow natural-language editing is disabled by the workflow authoring feature flag."
                .to_string(),
            ..compose_disabled_response()
        });
    }
    request
        .workflow_ir
        .validate()
        .map_err(WorkflowCompilerError::invalid_ir)?;
    let live_catalog = build_capability_catalog(&mcp_registry).await?;
    request.capability_catalog = merge_compose_catalogs(live_catalog, request.capability_catalog);
    let gemma = gemma.inner().clone();
    run_bounded_workflow_compiler("edit", move |cancellation| {
        edit_workflow_sync(request, &gemma, &cancellation)
    })
    .await
}

fn compile_and_save_workflow(
    mut request: SaveWorkflowRequest,
    capability_catalog: &CapabilityCatalog,
    persistence: &PersistenceEngine,
    compiler: &impl InstructionCompiler,
    identity: &SovereignIdentity,
) -> Result<SaveWorkflowResponse, WorkflowCompilerError> {
    validate_save_request(&request)?;
    // A saved version is a newly compiled artifact. Preserve historical IR in
    // storage, but bind every newly emitted version to the model that actually
    // compiled it instead of carrying a legacy authoring label forward.
    request.workflow_ir.compiler.model = WORKFLOW_COMPILER_MODEL.to_string();
    normalize_native_tool_arguments(&mut request.workflow_ir)?;
    hydrate_mcp_output_schemas(&mut request.workflow_ir, capability_catalog);
    request
        .workflow_ir
        .validate()
        .map_err(WorkflowCompilerError::invalid_ir)?;
    validate_workflow_ir_topology(&request.workflow_ir)?;
    let (version, project_id) = persistence
        .reserve_workflow_blueprint_for_project(
            &request.workflow,
            &request.visual_state,
            &mut request.workflow_ir,
            request.project_id.as_deref(),
        )
        .map_err(WorkflowCompilerError::database)?;
    let review_capabilities =
        crate::workflow_ir::review::workflow_review_capabilities(&request.workflow_ir);
    canonicalize_compiled_workflow_projection(
        &mut request,
        project_id.as_deref(),
        &review_capabilities,
    )?;
    stamp_workflow_artifacts(&mut request, identity)?;

    let result: Result<SaveWorkflowResponse, WorkflowCompilerError> =
        catch_unwind(AssertUnwindSafe(|| {
            request
                .workflow_ir
                .validate()
                .map_err(WorkflowCompilerError::invalid_ir)?;
            let output = compiler.compile(&request.workflow_ir)?;
            let instructions = materialize_instructions(output, &request.workflow_ir)?;
            persistence
                .publish_compiled_workflow_for_project(
                    &request.workflow,
                    &request.visual_state,
                    &request.workflow_ir,
                    &instructions,
                    request.activate,
                    project_id.as_deref(),
                )
                .map_err(WorkflowCompilerError::database)?;
            crate::workflow_scheduler::sync_workflow_schedule_from_visual_state(
                persistence,
                &request.workflow.id,
                request.workflow_ir.workflow_version,
                &request.workflow.name,
                &request.visual_state,
                request.activate,
            )
            .map_err(WorkflowCompilerError::runtime)?;
            Ok(SaveWorkflowResponse {
                workflow_id: request.workflow.id.clone(),
                workflow_version: version,
                project_id: project_id.clone(),
                compilation_status: "Compiled",
                compiled_node_count: instructions.len(),
                review_capabilities: review_capabilities.clone(),
            })
        }))
        .unwrap_or_else(|payload| {
            let error = workflow_compiler_panic_error("compile_and_save_workflow", payload);
            eprintln!(
                "OOMU_WORKFLOW_COMPILER_PANIC workflow_id={} version={} message={}",
                request.workflow.id, version, error.message
            );
            Err(error)
        });

    if let Err(error) = &result {
        if let Err(mark_error) = persistence.mark_workflow_compilation_failed(
            &request.workflow.id,
            version,
            &error.message,
        ) {
            eprintln!(
                "WORKFLOW_COMPILATION_FAILURE_STATE_WRITE_FAILED workflow_id={} version={} error={}",
                request.workflow.id, version, mark_error
            );
        }
    }
    result
}

fn canonicalize_compiled_workflow_projection(
    request: &mut SaveWorkflowRequest,
    project_id: Option<&str>,
    review_capabilities: &crate::workflow_ir::review::WorkflowReviewCapabilities,
) -> Result<(), WorkflowCompilerError> {
    let object = request.visual_state.as_object_mut().ok_or_else(|| {
        WorkflowCompilerError::invalid_request("visualState must be a JSON object.")
    })?;
    object.insert(
        "workflowIr".to_string(),
        serde_json::to_value(&request.workflow_ir).map_err(WorkflowCompilerError::serialization)?,
    );
    object.insert(
        "workflowVersion".to_string(),
        json!(request.workflow_ir.workflow_version),
    );
    object.insert("compilationStatus".to_string(), json!("Compiled"));
    object.insert(
        "reviewCapabilities".to_string(),
        serde_json::to_value(review_capabilities).map_err(WorkflowCompilerError::serialization)?,
    );
    match project_id {
        Some(project_id) => {
            object.insert("projectId".to_string(), json!(project_id));
        }
        None => {
            object.remove("projectId");
        }
    }
    request.workflow.steps = serde_json::to_string(&request.visual_state)
        .map_err(WorkflowCompilerError::serialization)?;
    Ok(())
}

fn validate_save_request(request: &SaveWorkflowRequest) -> Result<(), WorkflowCompilerError> {
    if request.workflow.id.trim().is_empty() || request.workflow.name.trim().is_empty() {
        return Err(WorkflowCompilerError::invalid_request(
            "Workflow id and name must not be empty.",
        ));
    }
    if request.workflow.id != request.workflow_ir.workflow_id {
        return Err(WorkflowCompilerError::invalid_request(
            "Saved workflow id must match Workflow IR workflowId.",
        ));
    }
    if request.workflow.name != request.workflow_ir.name {
        return Err(WorkflowCompilerError::invalid_request(
            "Saved workflow name must match Workflow IR name.",
        ));
    }
    if !request.visual_state.is_object() {
        return Err(WorkflowCompilerError::invalid_request(
            "visualState must be a JSON object.",
        ));
    }
    Ok(())
}

fn stamp_workflow_artifacts(
    request: &mut SaveWorkflowRequest,
    identity: &SovereignIdentity,
) -> Result<(), WorkflowCompilerError> {
    let visual_metadata = build_workflow_artifact_provenance(
        "workflow_json_configuration",
        &request.workflow.id,
        &request.visual_state,
        identity,
    )
    .map_err(WorkflowCompilerError::metadata)?;
    request
        .visual_state
        .as_object_mut()
        .ok_or_else(|| {
            WorkflowCompilerError::metadata(
                "Workflow visual state must be a JSON object before provenance signing."
                    .to_string(),
            )
        })?
        .insert(
            WORKFLOW_ARTIFACT_PROVENANCE_KEY.to_string(),
            serde_json::to_value(visual_metadata).map_err(WorkflowCompilerError::serialization)?,
        );
    request.workflow.steps = serde_json::to_string(&request.visual_state)
        .map_err(WorkflowCompilerError::serialization)?;

    let workflow_ir_value =
        serde_json::to_value(&request.workflow_ir).map_err(WorkflowCompilerError::serialization)?;
    let metadata = build_workflow_artifact_provenance(
        "workflow_intermediate_representation",
        &format!(
            "{}:{}",
            request.workflow_ir.workflow_id, request.workflow_ir.workflow_version
        ),
        &workflow_ir_value,
        identity,
    )
    .map_err(WorkflowCompilerError::metadata)?;
    let provenance =
        serde_json::to_value(metadata).map_err(WorkflowCompilerError::serialization)?;
    let mut workflow_metadata = request
        .workflow_ir
        .metadata
        .take()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    workflow_metadata.insert(WORKFLOW_ARTIFACT_PROVENANCE_KEY.to_string(), provenance);
    request.workflow_ir.metadata = Some(Value::Object(workflow_metadata));
    Ok(())
}

fn parse_compiler_output(
    text: &str,
    workflow_ir: &WorkflowIr,
) -> Result<CompilerOutput, WorkflowCompilerError> {
    let output: CompilerOutput =
        serde_json::from_str(text.trim()).map_err(WorkflowCompilerError::invalid_output)?;
    validate_compiler_output(&output, workflow_ir)?;
    Ok(output)
}

fn validate_compiler_output(
    output: &CompilerOutput,
    workflow_ir: &WorkflowIr,
) -> Result<(), WorkflowCompilerError> {
    if output.compiler_version != COMPILER_VERSION {
        return Err(WorkflowCompilerError::contract(format!(
            "compilerVersion must be {COMPILER_VERSION}."
        )));
    }

    let expected_agents = workflow_ir
        .nodes
        .iter()
        .filter_map(|node| match node {
            WorkflowNode::Agent(agent) => Some((agent.id.as_str(), &agent.input_mappings)),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    for instruction in &output.instructions {
        let Some(expected_mappings) = expected_agents.get(instruction.node_id.as_str()) else {
            return Err(WorkflowCompilerError::contract(format!(
                "instruction {} does not identify an Agent node.",
                instruction.node_id
            )));
        };
        if !seen.insert(instruction.node_id.as_str()) {
            return Err(WorkflowCompilerError::contract(format!(
                "duplicate instruction for Agent node {}.",
                instruction.node_id
            )));
        }
        if instruction.system_prompt.trim().is_empty() {
            return Err(WorkflowCompilerError::contract(format!(
                "Agent node {} has an empty systemPrompt.",
                instruction.node_id
            )));
        }
        let mut mapping_names = HashSet::new();
        for mapping in &instruction.input_variable_mappings {
            if mapping.name.trim().is_empty()
                || mapping.template.trim().is_empty()
                || !mapping_names.insert(mapping.name.as_str())
            {
                return Err(WorkflowCompilerError::contract(format!(
                    "Agent node {} has invalid inputVariableMappings.",
                    instruction.node_id
                )));
            }
            if !mapping.template.contains("{{") || !mapping.template.contains("}}") {
                return Err(WorkflowCompilerError::contract(format!(
                    "Agent node {} mapping {} must use a deterministic double-brace template.",
                    instruction.node_id, mapping.name
                )));
            }
        }
        let expected_names = expected_mappings
            .keys()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if mapping_names != expected_names {
            return Err(WorkflowCompilerError::contract(format!(
                "Agent node {} inputVariableMappings must exactly match its declared inputMappings.",
                instruction.node_id
            )));
        }
        for mapping in &instruction.input_variable_mappings {
            if expected_mappings.get(&mapping.name) != Some(&mapping.template) {
                return Err(WorkflowCompilerError::contract(format!(
                    "Agent node {} mapping {} must exactly match its declared input mapping.",
                    instruction.node_id, mapping.name
                )));
            }
        }
        if instruction.evaluation_protocol.success_criteria.is_empty()
            || instruction
                .evaluation_protocol
                .success_criteria
                .iter()
                .any(|criterion| criterion.trim().is_empty())
        {
            return Err(WorkflowCompilerError::contract(format!(
                "Agent node {} requires non-empty successCriteria.",
                instruction.node_id
            )));
        }
        if matches!(
            instruction.evaluation_protocol.failure_action,
            FailureAction::Fail
        ) && instruction.evaluation_protocol.max_retries != 0
        {
            return Err(WorkflowCompilerError::contract(format!(
                "Agent node {} cannot declare retries when failureAction is fail.",
                instruction.node_id
            )));
        }
    }
    if seen.len() != expected_agents.len() {
        return Err(WorkflowCompilerError::contract(
            "Compiler output must contain exactly one instruction per Agent node.".to_string(),
        ));
    }
    Ok(())
}

fn materialize_instructions(
    output: CompilerOutput,
    workflow_ir: &WorkflowIr,
) -> Result<Vec<CompiledInstruction>, WorkflowCompilerError> {
    validate_compiler_output(&output, workflow_ir)?;
    let created_at_ms = unix_time_ms();
    output
        .instructions
        .into_iter()
        .map(|instruction| {
            let input_variable_mappings = instruction
                .input_variable_mappings
                .into_iter()
                .map(|mapping| (mapping.name, mapping.template))
                .collect();
            let evaluation_protocol = serde_json::to_value(instruction.evaluation_protocol)
                .map_err(WorkflowCompilerError::serialization)?;
            Ok(CompiledInstruction {
                id: instruction_id(
                    &workflow_ir.workflow_id,
                    workflow_ir.workflow_version,
                    &instruction.node_id,
                ),
                workflow_id: workflow_ir.workflow_id.clone(),
                workflow_version: workflow_ir.workflow_version,
                node_id: instruction.node_id,
                node_kind: WorkflowNodeKind::Agent,
                system_prompt: instruction.system_prompt,
                input_variable_mappings,
                evaluation_protocol,
                compiler_model: WORKFLOW_COMPILER_MODEL.to_string(),
                compiler_version: output.compiler_version.clone(),
                created_at_ms,
            })
        })
        .collect()
}

async fn build_capability_catalog(
    registry: &McpClientRegistry,
) -> Result<CapabilityCatalog, WorkflowCompilerError> {
    let tool_catalog = registry.cached_tool_catalog().await;
    let mut actions = Vec::new();
    actions.extend(graph_authoring_capabilities());
    actions.extend(known_mcp_capabilities(&tool_catalog));
    actions.extend(taskflow_native_capabilities()?);
    actions.extend(registered_task_capabilities::catalog_actions()?);

    for (server_name, tools) in &tool_catalog {
        for tool in tools {
            if !workflow_runtime_supports_mcp_tool(server_name, &tool.name) {
                continue;
            }
            actions.push(mcp_capability_from_tool(server_name, tool));
        }
    }

    Ok(CapabilityCatalog {
        version: WORKFLOW_CAPABILITY_CATALOG_VERSION.to_string(),
        authoring_enabled: workflow_authoring_p0_enabled(),
        generated_at_ms: unix_time_ms(),
        actions: dedupe_capability_actions(actions),
        templates: Vec::new(),
    })
}

fn workflow_runtime_supports_mcp_tool(server_name: &str, tool_name: &str) -> bool {
    // Music is executed by the native chat bridge, not by the MCP session used
    // by Workflow Runtime. Keep it out of workflow authoring until that runtime
    // has an equivalent native executor instead of advertising a dead step.
    !(server_name == "macos_applescript"
        && matches!(tool_name, "read_system_music" | "send_system_email"))
}

fn merge_compose_catalogs(
    live_catalog: CapabilityCatalog,
    client_catalog: CapabilityCatalog,
) -> CapabilityCatalog {
    let mut actions = live_catalog.actions;
    let live_ids = actions
        .iter()
        .map(|action| action.id.clone())
        .collect::<HashSet<_>>();

    for mut action in client_catalog.actions {
        if live_ids.contains(&action.id) {
            continue;
        }
        if action.kind == "mcp_tool" {
            action.available = false;
            action.availability = "requires_connection".to_string();
            action.unavailable_reason = Some(connect_reason_for_action(&action));
        }
        actions.push(action);
    }

    let mut templates = live_catalog.templates;
    let live_template_ids = templates
        .iter()
        .map(|template| template.id.clone())
        .collect::<HashSet<_>>();
    for template in client_catalog.templates {
        if !live_template_ids.contains(&template.id) {
            templates.push(template);
        }
    }
    templates.sort_by(|left, right| left.name.cmp(&right.name));

    CapabilityCatalog {
        version: WORKFLOW_CAPABILITY_CATALOG_VERSION.to_string(),
        authoring_enabled: workflow_authoring_p0_enabled(),
        generated_at_ms: unix_time_ms(),
        actions: dedupe_capability_actions(actions),
        templates,
    }
}

fn workflow_authoring_p0_enabled() -> bool {
    std::env::var(WORKFLOW_AUTHORING_P0_ENV)
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "False"))
        .unwrap_or(true)
}

fn compose_workflow_sync(
    request: ComposeWorkflowRequest,
    gemma: &GemmaService,
    cancellation: &Arc<AtomicBool>,
) -> Result<ComposeWorkflowResponse, WorkflowCompilerError> {
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return Err(WorkflowCompilerError::invalid_request(
            "compose prompt must not be empty.",
        ));
    }
    if request.capability_catalog.actions.is_empty() {
        return Ok(ComposeWorkflowResponse {
            status: "failed",
            reason: "No workflow capabilities are available for grounding.".to_string(),
            workflow_ir: None,
            partial_draft: None,
            missing_capabilities: Vec::new(),
            missing_capability_details: Vec::new(),
            composed_by: "not_run",
            attempts: 0,
            latency_ms: 0,
        });
    }

    let started_at = unix_time_ms();
    if let Some(workflow_ir) = specialist_composer::compose_supported_workflow(&request)? {
        return specialist_compose_response(workflow_ir, &request, started_at);
    }
    let session_id = compose_session_id(prompt);
    let mut response = match gemma.infer_model_sync(
        WORKFLOW_COMPILER_RUNTIME_MODEL_ID,
        compose_infer_request(compose_prompt(&request)?, &session_id, cancellation),
    ) {
        Ok(response) => response,
        Err(error) => {
            return recover_compose_inference_error(error, &request, started_at, 1);
        }
    };
    let mut last_error: Option<ComposeAttemptError> = None;

    for attempt in 0..=COMPOSE_MAX_REPAIR_ATTEMPTS {
        match parse_compose_output(&response.text, &request, attempt, started_at) {
            Ok(output) => return Ok(output),
            Err(error) if attempt < COMPOSE_MAX_REPAIR_ATTEMPTS => {
                let repair_prompt = format!(
                    "<|turn>user\nYour prior workflow draft failed validation: {} {} Return only corrected compact JSON in the required compose response shape. Never return invalid Workflow IR, placeholder capability names, or tools absent from the catalog.<turn|>\n<|turn>model\n",
                    compact_error(&error.message),
                    compose_repair_hint(&request)
                );
                last_error = Some(error);
                response = match gemma.infer_model_sync(
                    WORKFLOW_COMPILER_RUNTIME_MODEL_ID,
                    compose_infer_request(repair_prompt, &session_id, cancellation),
                ) {
                    Ok(response) => response,
                    Err(error) => {
                        return recover_compose_inference_error(
                            error,
                            &request,
                            started_at,
                            attempt + 2,
                        );
                    }
                };
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
    }

    let error = last_error.unwrap_or_else(|| ComposeAttemptError {
        message: "Gemma did not produce a workflow draft.".to_string(),
        partial_draft: None,
        missing_capabilities: Vec::new(),
        missing_capability_details: Vec::new(),
    });
    Ok(ComposeWorkflowResponse {
        status: if error.missing_capabilities.is_empty() {
            "failed"
        } else {
            "needs_connection"
        },
        reason: if error.missing_capabilities.is_empty() {
            compose_failed_reason()
        } else {
            error.message
        },
        workflow_ir: None,
        partial_draft: error.partial_draft,
        missing_capabilities: error.missing_capabilities,
        missing_capability_details: error.missing_capability_details,
        composed_by: "gemma",
        attempts: COMPOSE_MAX_REPAIR_ATTEMPTS + 1,
        latency_ms: unix_time_ms().saturating_sub(started_at),
    })
}

fn edit_workflow_sync(
    request: EditWorkflowRequest,
    gemma: &GemmaService,
    cancellation: &Arc<AtomicBool>,
) -> Result<ComposeWorkflowResponse, WorkflowCompilerError> {
    let instruction = request.instruction.trim();
    if instruction.is_empty() {
        return Err(WorkflowCompilerError::invalid_request(
            "edit instruction must not be empty.",
        ));
    }
    request
        .workflow_ir
        .validate()
        .map_err(WorkflowCompilerError::invalid_ir)?;
    if request.capability_catalog.actions.is_empty() {
        return Ok(ComposeWorkflowResponse {
            status: "failed",
            reason: "No workflow capabilities are available for grounding.".to_string(),
            workflow_ir: None,
            partial_draft: None,
            missing_capabilities: Vec::new(),
            missing_capability_details: Vec::new(),
            composed_by: "not_run",
            attempts: 0,
            latency_ms: 0,
        });
    }

    let started_at = unix_time_ms();
    if let Some(workflow_ir) = specialist_composer::edit_supported_workflow(&request)? {
        let specialist_request = ComposeWorkflowRequest {
            prompt: request.instruction.clone(),
            capability_catalog: request.capability_catalog.clone(),
            project_id: None,
            workflow_id: Some(request.workflow_ir.workflow_id.clone()),
            name: Some(request.workflow_ir.name.clone()),
        };
        return specialist_compose_response(workflow_ir, &specialist_request, started_at);
    }
    let parse_request = ComposeWorkflowRequest {
        prompt: request.instruction.clone(),
        capability_catalog: request.capability_catalog.clone(),
        project_id: None,
        workflow_id: Some(request.workflow_ir.workflow_id.clone()),
        name: Some(request.workflow_ir.name.clone()),
    };
    let session_id = edit_session_id(&request.workflow_ir, instruction);
    let mut response = match gemma.infer_model_sync(
        WORKFLOW_COMPILER_RUNTIME_MODEL_ID,
        compose_infer_request(edit_prompt(&request)?, &session_id, cancellation),
    ) {
        Ok(response) => response,
        Err(error) => {
            return recover_edit_inference_error(error, started_at, 1);
        }
    };
    let mut last_error: Option<ComposeAttemptError> = None;

    for attempt in 0..=COMPOSE_MAX_REPAIR_ATTEMPTS {
        match parse_compose_output(&response.text, &parse_request, attempt, started_at) {
            Ok(output) => return Ok(output),
            Err(error) if attempt < COMPOSE_MAX_REPAIR_ATTEMPTS => {
                let repair_prompt = format!(
                    "<|turn>user\nYour prior edited workflow failed validation: {} {} Return only corrected compact JSON in the required compose response shape. Never return invalid Workflow IR, placeholder capability names, or tools absent from the catalog.<turn|>\n<|turn>model\n",
                    compact_error(&error.message),
                    compose_repair_hint(&parse_request)
                );
                last_error = Some(error);
                response = match gemma.infer_model_sync(
                    WORKFLOW_COMPILER_RUNTIME_MODEL_ID,
                    compose_infer_request(repair_prompt, &session_id, cancellation),
                ) {
                    Ok(response) => response,
                    Err(error) => {
                        return recover_edit_inference_error(error, started_at, attempt + 2);
                    }
                };
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
    }

    let error = last_error.unwrap_or_else(|| ComposeAttemptError {
        message: "Gemma did not produce an edited workflow draft.".to_string(),
        partial_draft: None,
        missing_capabilities: Vec::new(),
        missing_capability_details: Vec::new(),
    });
    Ok(ComposeWorkflowResponse {
        status: if error.missing_capabilities.is_empty() {
            "failed"
        } else {
            "needs_connection"
        },
        reason: if error.missing_capabilities.is_empty() {
            compose_failed_reason()
        } else {
            error.message
        },
        workflow_ir: None,
        partial_draft: error.partial_draft,
        missing_capabilities: error.missing_capabilities,
        missing_capability_details: error.missing_capability_details,
        composed_by: "gemma",
        attempts: COMPOSE_MAX_REPAIR_ATTEMPTS + 1,
        latency_ms: unix_time_ms().saturating_sub(started_at),
    })
}

fn recover_compose_inference_error(
    error: GemmaError,
    _request: &ComposeWorkflowRequest,
    started_at: i64,
    attempts: usize,
) -> Result<ComposeWorkflowResponse, WorkflowCompilerError> {
    if !is_recoverable_compose_inference_error(&error) {
        return Err(WorkflowCompilerError::inference(error));
    }

    Ok(ComposeWorkflowResponse {
        status: "failed",
        reason: compose_failed_reason(),
        workflow_ir: None,
        partial_draft: None,
        missing_capabilities: Vec::new(),
        missing_capability_details: Vec::new(),
        composed_by: "gemma",
        attempts,
        latency_ms: unix_time_ms().saturating_sub(started_at),
    })
}

fn recover_edit_inference_error(
    error: GemmaError,
    started_at: i64,
    attempts: usize,
) -> Result<ComposeWorkflowResponse, WorkflowCompilerError> {
    if !is_recoverable_compose_inference_error(&error) {
        return Err(WorkflowCompilerError::inference(error));
    }

    Ok(ComposeWorkflowResponse {
        status: "failed",
        reason: compose_failed_reason(),
        workflow_ir: None,
        partial_draft: None,
        missing_capabilities: Vec::new(),
        missing_capability_details: Vec::new(),
        composed_by: "gemma",
        attempts,
        latency_ms: unix_time_ms().saturating_sub(started_at),
    })
}

fn is_recoverable_compose_inference_error(error: &GemmaError) -> bool {
    error.code == LOCAL_MODEL_REPETITION_COLLAPSE_CODE
}

fn compose_prompt(request: &ComposeWorkflowRequest) -> Result<String, WorkflowCompilerError> {
    let catalog =
        serde_json::to_string(&compose_catalog_prompt_payload(&request.capability_catalog))
            .map_err(WorkflowCompilerError::serialization)?;
    let workflow_id = request
        .workflow_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("wf-composed-draft");
    let name = request
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Composed workflow");
    let user_prompt = format!(
        "User request:\n{}\n\nWorkflow id: {}\nWorkflow name: {}\n\nAvailable capability catalog:\n{}",
        request.prompt.trim(),
        workflow_id,
        name,
        catalog
    );
    let mut prompt = format_structured_runtime_prompt(WORKFLOW_COMPOSE_SYSTEM_PROMPT, &user_prompt);
    prompt.push_str("<|channel>text\n<channel|>");
    Ok(prompt)
}

fn edit_prompt(request: &EditWorkflowRequest) -> Result<String, WorkflowCompilerError> {
    let catalog =
        serde_json::to_string(&compose_catalog_prompt_payload(&request.capability_catalog))
            .map_err(WorkflowCompilerError::serialization)?;
    let current_ir =
        serde_json::to_string(&sanitize_workflow_ir_for_compiler(&request.workflow_ir))
            .map_err(WorkflowCompilerError::serialization)?;
    let user_prompt = format!(
        "Existing Workflow IR:\n{}\n\nChange request:\n{}\n\nAvailable capability catalog:\n{}",
        current_ir,
        request.instruction.trim(),
        catalog
    );
    let mut prompt = format_structured_runtime_prompt(WORKFLOW_EDIT_SYSTEM_PROMPT, &user_prompt);
    prompt.push_str("<|channel>text\n<channel|>");
    Ok(prompt)
}

fn parse_compose_output(
    text: &str,
    request: &ComposeWorkflowRequest,
    attempt: usize,
    started_at: i64,
) -> Result<ComposeWorkflowResponse, ComposeAttemptError> {
    let json_text = extract_json_object(text).ok_or_else(|| ComposeAttemptError {
        message: "Gemma did not return a JSON object.".to_string(),
        partial_draft: None,
        missing_capabilities: Vec::new(),
        missing_capability_details: Vec::new(),
    })?;
    let raw: RawComposeOutput =
        serde_json::from_str(json_text).map_err(|error| ComposeAttemptError {
            message: format!("Gemma returned invalid compose JSON: {error}"),
            partial_draft: None,
            missing_capabilities: Vec::new(),
            missing_capability_details: Vec::new(),
        })?;

    if raw.status != "composed" {
        return compose_output_status::resolve_non_composed_output(
            raw, request, attempt, started_at,
        );
    }

    let workflow_value = raw.workflow_ir.ok_or_else(|| ComposeAttemptError {
        message: "Composed response omitted workflowIr.".to_string(),
        partial_draft: raw.partial_draft,
        missing_capabilities: Vec::new(),
        missing_capability_details: Vec::new(),
    })?;
    let mut workflow_ir: WorkflowIr =
        serde_json::from_value(workflow_value.clone()).map_err(|error| ComposeAttemptError {
            message: format!("workflowIr is not a valid Workflow IR object: {error}"),
            partial_draft: Some(workflow_value.clone()),
            missing_capabilities: Vec::new(),
            missing_capability_details: Vec::new(),
        })?;

    // The compiler identity is native-observed provenance, not model-authored
    // metadata. This also upgrades edits of readable legacy E2B IR without
    // rewriting the historical record they were derived from.
    workflow_ir.compiler.model = WORKFLOW_COMPILER_MODEL.to_string();

    if let Some(workflow_id) = request
        .workflow_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        workflow_ir.workflow_id = workflow_id.to_string();
    }
    if let Some(name) = request
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        workflow_ir.name = name.to_string();
    }
    normalize_native_tool_arguments(&mut workflow_ir).map_err(|error| ComposeAttemptError {
        message: error.message,
        partial_draft: Some(workflow_value.clone()),
        missing_capabilities: Vec::new(),
        missing_capability_details: Vec::new(),
    })?;
    hydrate_mcp_output_schemas(&mut workflow_ir, &request.capability_catalog);

    workflow_ir
        .validate()
        .map_err(|errors| ComposeAttemptError {
            message: format!("Workflow IR validation failed: {}", errors.join("; ")),
            partial_draft: Some(workflow_value.clone()),
            missing_capabilities: Vec::new(),
            missing_capability_details: Vec::new(),
        })?;
    validate_workflow_ir_topology(&workflow_ir).map_err(|error| ComposeAttemptError {
        message: error.message,
        partial_draft: Some(workflow_value.clone()),
        missing_capabilities: Vec::new(),
        missing_capability_details: Vec::new(),
    })?;
    registered_task_capabilities::validate_objective_bindings(&request.prompt, &workflow_ir)
        .map_err(|error| ComposeAttemptError {
            message: error.message,
            partial_draft: Some(workflow_value.clone()),
            missing_capabilities: Vec::new(),
            missing_capability_details: Vec::new(),
        })?;

    let missing_capabilities =
        missing_grounded_capabilities(&workflow_ir, &request.capability_catalog)?;
    if !missing_capabilities.is_empty() {
        let missing_capability_titles = missing_capability_titles(&missing_capabilities);
        return Ok(ComposeWorkflowResponse {
            status: "needs_connection",
            reason: missing_capability_reason(&missing_capabilities),
            workflow_ir: None,
            partial_draft: Some(workflow_value),
            missing_capabilities: missing_capability_titles,
            missing_capability_details: missing_capabilities,
            composed_by: "gemma",
            attempts: attempt + 1,
            latency_ms: unix_time_ms().saturating_sub(started_at),
        });
    }

    Ok(ComposeWorkflowResponse {
        status: "composed",
        reason: non_empty_or(raw.reason, "Workflow composed successfully."),
        workflow_ir: Some(workflow_ir),
        partial_draft: None,
        missing_capabilities: Vec::new(),
        missing_capability_details: Vec::new(),
        composed_by: "gemma",
        attempts: attempt + 1,
        latency_ms: unix_time_ms().saturating_sub(started_at),
    })
}

fn hydrate_mcp_output_schemas(ir: &mut WorkflowIr, catalog: &CapabilityCatalog) {
    for node in &mut ir.nodes {
        let WorkflowNode::McpTool(tool) = node else {
            continue;
        };
        // Output contracts are runtime authority, not model-authored metadata.
        // Prefer the live tool schema; if a packaged helper is still connecting,
        // use only OOMU's exact built-in contract. Unknown/custom tools keep no
        // client-authored schema and therefore cannot invent collection semantics.
        tool.output_schema = catalog
            .actions
            .iter()
            .find(|action| {
                action.kind == "mcp_tool"
                    && action.server_name.as_deref() == Some(tool.server_name.as_str())
                    && action.tool_name.as_deref() == Some(tool.tool_name.as_str())
            })
            .and_then(|action| action.output_schema.clone())
            .or_else(|| trusted_builtin_mcp_output_schema(&tool.server_name, &tool.tool_name));
    }
}

fn trusted_builtin_mcp_output_schema(server_name: &str, tool_name: &str) -> Option<Value> {
    let collection_name = match (server_name, tool_name) {
        ("local_filesystem", "list_directory") => "files",
        ("local_search", "search_web") => "results",
        ("macos_applescript", "read_system_calendar") => "events",
        ("macos_applescript", "read_system_emails") => "emails",
        ("macos_applescript", "read_system_notes") => "notes",
        ("macos_applescript", "read_system_contacts") => "contacts",
        ("macos_applescript", "read_system_reminders") => "reminders",
        ("macos_applescript", "read_apple_app_ui") => "uiText",
        _ => return None,
    };
    Some(structured_collection_output_schema(collection_name))
}

fn structured_collection_output_schema(collection_name: &str) -> Value {
    let collection_properties = serde_json::Map::from_iter([(
        collection_name.to_string(),
        json!({ "type": "array", "items": {} }),
    )]);
    json!({
        "type": "object",
        "x-oomu-result-contract": {
            "kind": "collection",
            "path": format!("/structuredContent/{collection_name}"),
            "emptyIsSuccess": true
        },
        "properties": {
            "structuredContent": {
                "type": "object",
                "properties": collection_properties,
                "required": [collection_name],
                "additionalProperties": true
            }
        },
        "required": ["structuredContent"],
        "additionalProperties": true
    })
}

fn missing_grounded_capabilities(
    ir: &WorkflowIr,
    catalog: &CapabilityCatalog,
) -> Result<Vec<MissingCapabilityDetail>, ComposeAttemptError> {
    let available_mcp = catalog
        .actions
        .iter()
        .filter(|action| action.kind == "mcp_tool" && action.available)
        .filter_map(|action| {
            Some((
                action.server_name.as_ref()?.as_str(),
                action.tool_name.as_ref()?.as_str(),
            ))
        })
        .collect::<HashSet<_>>();
    let known_mcp = catalog
        .actions
        .iter()
        .filter(|action| action.kind == "mcp_tool")
        .filter_map(|action| {
            Some((
                action.server_name.as_ref()?.as_str(),
                action.tool_name.as_ref()?.as_str(),
                action,
            ))
        })
        .collect::<Vec<_>>();

    let mut missing = Vec::<MissingCapabilityDetail>::new();
    for node in &ir.nodes {
        if let WorkflowNode::McpTool(tool) = node {
            let key = (tool.server_name.as_str(), tool.tool_name.as_str());
            if available_mcp.contains(&key) {
                continue;
            }
            let detail = known_mcp
                .iter()
                .find(|(server, name, _)| *server == key.0 && *name == key.1)
                .map(|(_, _, action)| missing_capability_detail_from_action(action))
                .ok_or_else(|| ComposeAttemptError {
                    message: format!(
                        "Workflow referenced absent catalog tool {} / {}.",
                        tool.server_name, tool.tool_name
                    ),
                    partial_draft: None,
                    missing_capabilities: Vec::new(),
                    missing_capability_details: Vec::new(),
                });
            let detail = detail?;
            if !missing.iter().any(|existing| existing.id == detail.id) {
                missing.push(detail);
            }
        }
    }
    Ok(missing)
}

fn validate_workflow_ir_topology(ir: &WorkflowIr) -> Result<(), WorkflowCompilerError> {
    for node in &ir.nodes {
        let WorkflowNode::McpTool(tool) = node else {
            continue;
        };

        if is_report_preview_tool(tool) && !upstream_contains_report_writer(ir, &tool.id) {
            return Err(WorkflowCompilerError::topological_anomaly(
                WORKFLOW_TOPOLOGY_MISSING_REPORT_WRITER_CODE,
                "This workflow wants to open a report for review, but it needs to save the report to disk first. Insert a Write a project report step before opening the report."
                    .to_string(),
            ));
        }

        if let Some((field_label, keys)) = sandbox_path_argument_keys(tool) {
            validate_mcp_tool_sandbox_path(tool, field_label, keys)?;
        }
    }

    validate_template_reference_dominance(ir)?;
    validate_declared_collection_consumers(ir)?;
    validate_collection_empty_routes(ir)?;
    validate_empty_completion_outputs(ir)?;
    validate_indexed_collection_guards(ir)?;
    registered_task_capabilities::validate_static_evidence_synthesis(ir)?;
    Ok(())
}

fn validate_declared_collection_consumers(ir: &WorkflowIr) -> Result<(), WorkflowCompilerError> {
    let collection_references =
        ir.nodes
            .iter()
            .filter_map(|node| match node {
                WorkflowNode::McpTool(tool) => declared_collection_reference(tool)
                    .map(|reference| (tool.id.as_str(), reference)),
                _ => None,
            })
            .collect::<HashMap<_, _>>();
    if collection_references.is_empty() {
        return Ok(());
    }
    for node in &ir.nodes {
        if matches!(node, WorkflowNode::Conditional(_) | WorkflowNode::Output(_)) {
            continue;
        }
        let references = node_template_references(node);
        let used_collections = collection_references
            .iter()
            .filter(|(producer_id, _)| {
                let producer_prefix = format!("nodes.{producer_id}.output");
                references.iter().any(|reference| {
                    canonical_node_output_reference(reference).is_some_and(|reference| {
                        reference == producer_prefix.as_str()
                            || reference.starts_with(&format!("{producer_prefix}."))
                    })
                })
            })
            .map(|(_, reference)| reference.as_str())
            .collect::<HashSet<_>>();
        if used_collections.is_empty() {
            continue;
        }
        let guard_ids = ir
            .nodes
            .iter()
            .filter_map(|candidate| {
                let WorkflowNode::Conditional(guard) = candidate else {
                    return None;
                };
                let mapped = guard
                    .input_mapping
                    .as_deref()
                    .and_then(exact_template_reference)
                    .and_then(|reference| canonical_node_output_reference(&reference))?;
                (used_collections.contains(mapped.as_str())
                    && is_nonempty_collection_condition(&guard.condition))
                .then_some(guard.id.as_str())
            })
            .collect::<HashSet<_>>();
        if guard_ids.is_empty() || path_can_reach_without_nonempty_guard(ir, node.id(), &guard_ids)
        {
            return Err(WorkflowCompilerError::topological_anomaly(
                WORKFLOW_TOPOLOGY_UNSAFE_COLLECTION_CODE,
                format!(
                    "The {} step can receive an empty collection. Add a simple check before this step and finish successfully when every relevant collection is empty.",
                    compact_error(node_label(node))
                ),
            ));
        }
    }
    Ok(())
}

fn validate_collection_empty_routes(ir: &WorkflowIr) -> Result<(), WorkflowCompilerError> {
    let guards = declared_collection_guards(ir);
    if collection_empty_routes_are_safe(ir, &guards) {
        return Ok(());
    }
    Err(WorkflowCompilerError::topological_anomaly(
        WORKFLOW_TOPOLOGY_UNSAFE_COLLECTION_CODE,
        "When a collection is empty, the workflow must finish with Nothing found before running a model, approval, or action."
            .to_string(),
    ))
}

fn declared_collection_guards(ir: &WorkflowIr) -> HashMap<&str, String> {
    let declared_collections = ir
        .nodes
        .iter()
        .filter_map(|node| match node {
            WorkflowNode::McpTool(tool) => declared_collection_reference(tool),
            _ => None,
        })
        .collect::<HashSet<_>>();
    ir.nodes
        .iter()
        .filter_map(|candidate| {
            let WorkflowNode::Conditional(guard) = candidate else {
                return None;
            };
            let mapped = guard
                .input_mapping
                .as_deref()
                .and_then(exact_template_reference)
                .and_then(|reference| canonical_node_output_reference(&reference))?;
            (declared_collections.contains(&mapped)
                && is_nonempty_collection_condition(&guard.condition))
            .then_some((guard.id.as_str(), mapped))
        })
        .collect()
}

fn collection_empty_routes_are_safe(ir: &WorkflowIr, guards: &HashMap<&str, String>) -> bool {
    guards.iter().all(|(guard_id, collection)| {
        ir.edges
            .iter()
            .find(|edge| edge.source_node_id == *guard_id && edge.source_port == "false")
            .is_some_and(|edge| {
                let proven_empty = HashSet::from([collection.clone()]);
                collection_empty_route_is_safe(
                    ir,
                    edge.target_node_id.as_str(),
                    guards,
                    &proven_empty,
                    &mut HashSet::new(),
                )
            })
    })
}

fn collection_empty_route_is_safe<'a>(
    ir: &'a WorkflowIr,
    node_id: &'a str,
    guards: &HashMap<&str, String>,
    proven_empty: &HashSet<String>,
    path: &mut HashSet<&'a str>,
) -> bool {
    if !path.insert(node_id) {
        return false;
    }
    let Some(node) = ir.nodes.iter().find(|node| node.id() == node_id) else {
        return false;
    };
    let edges = ir
        .edges
        .iter()
        .filter(|edge| edge.source_node_id == node_id)
        .collect::<Vec<_>>();
    let safe = match node {
        WorkflowNode::Output(output) => {
            let mapped = exact_template_reference(&output.input_mapping)
                .and_then(|reference| canonical_node_output_reference(&reference));
            output.completion_kind == WorkflowCompletionKind::EmptyCollection
                && edges.is_empty()
                && mapped.is_some_and(|reference| proven_empty.contains(&reference))
        }
        WorkflowNode::Conditional(_) if !edges.is_empty() => edges.iter().all(|edge| {
            if guards.contains_key(node_id) && edge.source_port == "true" {
                true
            } else {
                let mut next_proven_empty = proven_empty.clone();
                if edge.source_port == "false" {
                    if let Some(collection) = guards.get(node_id) {
                        next_proven_empty.insert(collection.clone());
                    }
                }
                collection_empty_route_is_safe(
                    ir,
                    edge.target_node_id.as_str(),
                    guards,
                    &next_proven_empty,
                    path,
                )
            }
        }),
        _ => false,
    };
    path.remove(node_id);
    safe
}

fn validate_empty_completion_outputs(ir: &WorkflowIr) -> Result<(), WorkflowCompilerError> {
    let declared_collections = ir
        .nodes
        .iter()
        .filter_map(|node| match node {
            WorkflowNode::McpTool(tool) => declared_collection_reference(tool),
            _ => None,
        })
        .collect::<HashSet<_>>();
    for node in &ir.nodes {
        let WorkflowNode::Output(output) = node else {
            continue;
        };
        if output.completion_kind != WorkflowCompletionKind::EmptyCollection {
            continue;
        }
        let mapped = exact_template_reference(&output.input_mapping)
            .and_then(|reference| canonical_node_output_reference(&reference));
        if mapped
            .as_ref()
            .map_or(true, |reference| !declared_collections.contains(reference))
        {
            return Err(WorkflowCompilerError::topological_anomaly(
                WORKFLOW_TOPOLOGY_UNSAFE_COLLECTION_CODE,
                format!(
                    "The {} step must use the exact collection declared by its read step before it can report that nothing was found.",
                    compact_error(&output.label)
                ),
            ));
        }
    }
    Ok(())
}

fn declared_collection_reference(tool: &McpToolNode) -> Option<String> {
    let contract = tool.output_schema.as_ref()?.get("x-oomu-result-contract")?;
    if contract.get("kind").and_then(Value::as_str) != Some("collection")
        || contract.get("emptyIsSuccess").and_then(Value::as_bool) != Some(true)
    {
        return None;
    }
    let pointer = contract.get("path")?.as_str()?.trim();
    let segments = pointer
        .strip_prefix('/')?
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
        .collect::<Vec<_>>();
    (!segments.is_empty()).then(|| format!("nodes.{}.output.data.{}", tool.id, segments.join(".")))
}

fn path_can_reach_without_nonempty_guard(
    ir: &WorkflowIr,
    consumer_id: &str,
    guard_ids: &HashSet<&str>,
) -> bool {
    let roots = ir
        .nodes
        .iter()
        .filter(|node| !ir.edges.iter().any(|edge| edge.target_node_id == node.id()))
        .map(WorkflowNode::id)
        .collect::<Vec<_>>();
    let mut stack = roots
        .into_iter()
        .map(|node_id| (node_id, false))
        .collect::<Vec<_>>();
    let mut seen = HashSet::<(&str, bool)>::new();
    while let Some((node_id, guarded)) = stack.pop() {
        if !seen.insert((node_id, guarded)) {
            continue;
        }
        if node_id == consumer_id && !guarded {
            return true;
        }
        for edge in ir
            .edges
            .iter()
            .filter(|edge| edge.source_node_id == node_id)
        {
            let next_guarded =
                guarded || (guard_ids.contains(node_id) && edge.source_port == "true");
            stack.push((edge.target_node_id.as_str(), next_guarded));
        }
    }
    false
}

fn validate_template_reference_dominance(ir: &WorkflowIr) -> Result<(), WorkflowCompilerError> {
    let dominators = workflow_dominators(ir);
    for node in &ir.nodes {
        for reference in node_template_references(node) {
            let Some(producer_id) = referenced_node_id(&reference) else {
                continue;
            };
            let producer_dominates = producer_id == node.id()
                || dominators
                    .get(node.id())
                    .is_some_and(|nodes| nodes.contains(producer_id));
            if !producer_dominates {
                return Err(WorkflowCompilerError::topological_anomaly(
                    WORKFLOW_TOPOLOGY_UNSAFE_REFERENCE_CODE,
                    format!(
                        "The {} step can run without the earlier result it uses. Connect every path through {} before this step.",
                        compact_error(node_label(node)),
                        compact_error(producer_id)
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn validate_indexed_collection_guards(ir: &WorkflowIr) -> Result<(), WorkflowCompilerError> {
    let dominators = workflow_dominators(ir);
    for node in &ir.nodes {
        for reference in node_template_references(node) {
            let Some(collection_reference) = indexed_collection_reference(&reference) else {
                continue;
            };
            let guarded = ir.nodes.iter().any(|candidate| {
                let WorkflowNode::Conditional(guard) = candidate else {
                    return false;
                };
                guard.input_mapping.as_deref().is_some_and(|mapping| {
                    exact_template_reference(mapping)
                        .and_then(|reference| canonical_node_output_reference(&reference))
                        .as_deref()
                        == canonical_node_output_reference(&collection_reference).as_deref()
                }) && is_nonempty_collection_condition(&guard.condition)
                    && dominators
                        .get(node.id())
                        .is_some_and(|nodes| nodes.contains(guard.id.as_str()))
                    && guarded_true_path_only(ir, &guard.id, node.id())
                    && false_path_completes_empty(ir, &guard.id)
            });
            if !guarded {
                return Err(WorkflowCompilerError::topological_anomaly(
                    WORKFLOW_TOPOLOGY_UNSAFE_COLLECTION_CODE,
                    format!(
                        "The {} step uses the first item in a collection without checking that an item exists. Add a check before this step and finish successfully when the collection is empty.",
                        compact_error(node_label(node))
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn workflow_dominators(ir: &WorkflowIr) -> HashMap<&str, HashSet<&str>> {
    let node_ids = ir
        .nodes
        .iter()
        .map(|node| node.id())
        .collect::<HashSet<_>>();
    let mut predecessors = HashMap::<&str, Vec<&str>>::new();
    for edge in &ir.edges {
        predecessors
            .entry(edge.target_node_id.as_str())
            .or_default()
            .push(edge.source_node_id.as_str());
    }
    let mut dominators = ir
        .nodes
        .iter()
        .map(|node| {
            let values = if predecessors.get(node.id()).map_or(true, Vec::is_empty) {
                HashSet::from([node.id()])
            } else {
                node_ids.clone()
            };
            (node.id(), values)
        })
        .collect::<HashMap<_, _>>();

    loop {
        let mut changed = false;
        for node in &ir.nodes {
            let Some(parents) = predecessors
                .get(node.id())
                .filter(|items| !items.is_empty())
            else {
                continue;
            };
            let mut intersection = node_ids.clone();
            for parent in parents {
                if let Some(parent_dominators) = dominators.get(parent) {
                    intersection.retain(|candidate| parent_dominators.contains(candidate));
                }
            }
            intersection.insert(node.id());
            if dominators.get(node.id()) != Some(&intersection) {
                dominators.insert(node.id(), intersection);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    dominators
}

fn node_template_references(node: &WorkflowNode) -> Vec<String> {
    let mut templates = Vec::<&str>::new();
    match node {
        WorkflowNode::Agent(agent) => {
            templates.extend(agent.input_mappings.values().map(String::as_str))
        }
        WorkflowNode::Router(router) => templates.push(&router.expression),
        WorkflowNode::Conditional(conditional) => {
            if let Some(mapping) = conditional.input_mapping.as_deref() {
                templates.push(mapping);
            }
        }
        WorkflowNode::Loop(loop_node) => templates.push(&loop_node.items_mapping),
        WorkflowNode::McpTool(tool) => {
            collect_json_template_strings(&tool.arguments, &mut templates)
        }
        WorkflowNode::SystemAction(action) => {
            templates.push(&action.command);
            templates.extend(action.args.iter().map(String::as_str));
            if let Some(directory) = action.working_directory.as_deref() {
                templates.push(directory);
            }
        }
        WorkflowNode::Output(output) => templates.push(&output.input_mapping),
        WorkflowNode::Input(_) | WorkflowNode::Permission(_) => {}
    }
    templates
        .into_iter()
        .flat_map(template_references)
        .collect()
}

fn collect_json_template_strings<'a>(value: &'a Value, target: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => target.push(text),
        Value::Array(values) => {
            for value in values {
                collect_json_template_strings(value, target);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_json_template_strings(value, target);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn template_references(template: &str) -> Vec<String> {
    let mut references = Vec::new();
    let mut remainder = template;
    while let Some(start) = remainder.find("{{") {
        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find("}}") else {
            break;
        };
        let reference = after_start[..end].trim();
        if !reference.is_empty() {
            references.push(reference.to_string());
        }
        remainder = &after_start[end + 2..];
    }
    references
}

fn exact_template_reference(template: &str) -> Option<String> {
    let trimmed = template.trim();
    let inner = trimmed.strip_prefix("{{")?.strip_suffix("}}")?.trim();
    (!inner.is_empty() && !inner.contains("{{") && !inner.contains("}}")).then(|| inner.to_string())
}

fn referenced_node_id(reference: &str) -> Option<&str> {
    let tail = reference.strip_prefix("nodes.").unwrap_or(reference);
    let (node_id, suffix) = tail.split_once('.')?;
    (suffix == "output" || suffix.starts_with("output.")).then_some(node_id)
}

fn canonical_node_output_reference(reference: &str) -> Option<String> {
    referenced_node_id(reference)?;
    if reference.starts_with("nodes.") {
        Some(reference.to_string())
    } else {
        Some(format!("nodes.{reference}"))
    }
}

fn indexed_collection_reference(reference: &str) -> Option<String> {
    let segments = reference.split('.').collect::<Vec<_>>();
    let numeric_index = segments
        .iter()
        .position(|segment| segment.parse::<usize>().is_ok())?;
    (numeric_index > 0).then(|| segments[..numeric_index].join("."))
}

fn is_nonempty_collection_condition(condition: &str) -> bool {
    matches!(
        condition.split_whitespace().collect::<Vec<_>>().as_slice(),
        ["$", "!=", "[]"] | ["input", "!=", "[]"]
    )
}

fn guarded_true_path_only(ir: &WorkflowIr, guard_id: &str, consumer_id: &str) -> bool {
    let true_target = ir
        .edges
        .iter()
        .find(|edge| edge.source_node_id == guard_id && edge.source_port == "true")
        .map(|edge| edge.target_node_id.as_str());
    let false_target = ir
        .edges
        .iter()
        .find(|edge| edge.source_node_id == guard_id && edge.source_port == "false")
        .map(|edge| edge.target_node_id.as_str());
    true_target.is_some_and(|target| node_reaches(ir, target, consumer_id))
        && false_target.is_some_and(|target| !node_reaches(ir, target, consumer_id))
}

fn false_path_completes_empty(ir: &WorkflowIr, guard_id: &str) -> bool {
    let false_target = ir
        .edges
        .iter()
        .find(|edge| edge.source_node_id == guard_id && edge.source_port == "false")
        .map(|edge| edge.target_node_id.as_str());
    let Some(false_target) = false_target else {
        return false;
    };
    let guards = declared_collection_guards(ir);
    let Some(collection) = guards.get(guard_id).cloned() else {
        return false;
    };
    let proven_empty = HashSet::from([collection]);
    collection_empty_route_is_safe(
        ir,
        false_target,
        &guards,
        &proven_empty,
        &mut HashSet::new(),
    )
}

fn node_reaches(ir: &WorkflowIr, start: &str, target: &str) -> bool {
    if start == target {
        return true;
    }
    let mut stack = vec![start];
    let mut seen = HashSet::new();
    while let Some(node_id) = stack.pop() {
        if !seen.insert(node_id) {
            continue;
        }
        for edge in ir
            .edges
            .iter()
            .filter(|edge| edge.source_node_id == node_id)
        {
            if edge.target_node_id == target {
                return true;
            }
            stack.push(edge.target_node_id.as_str());
        }
    }
    false
}

fn node_label(node: &WorkflowNode) -> &str {
    match node {
        WorkflowNode::Input(node) => &node.label,
        WorkflowNode::Agent(node) => &node.label,
        WorkflowNode::Router(node) => &node.label,
        WorkflowNode::Conditional(node) => &node.label,
        WorkflowNode::Loop(node) => &node.label,
        WorkflowNode::Permission(node) => &node.label,
        WorkflowNode::McpTool(node) => &node.label,
        WorkflowNode::SystemAction(node) => &node.label,
        WorkflowNode::Output(node) => &node.label,
    }
}

fn upstream_contains_report_writer(ir: &WorkflowIr, target_node_id: &str) -> bool {
    let node_by_id = ir
        .nodes
        .iter()
        .map(|node| (node.id(), node))
        .collect::<HashMap<_, _>>();
    let mut reverse_edges = HashMap::<&str, Vec<&str>>::new();
    for edge in &ir.edges {
        reverse_edges
            .entry(edge.target_node_id.as_str())
            .or_default()
            .push(edge.source_node_id.as_str());
    }

    let mut seen = HashSet::<&str>::new();
    let mut stack = reverse_edges
        .get(target_node_id)
        .cloned()
        .unwrap_or_default();
    while let Some(node_id) = stack.pop() {
        if !seen.insert(node_id) {
            continue;
        }
        if node_by_id
            .get(node_id)
            .is_some_and(|node| is_report_writer_node(node))
        {
            return true;
        }
        if let Some(parents) = reverse_edges.get(node_id) {
            stack.extend(parents.iter().copied());
        }
    }

    false
}

fn is_report_preview_tool(tool: &McpToolNode) -> bool {
    tool.tool_name == "preview_report"
}

fn is_report_writer_node(node: &WorkflowNode) -> bool {
    matches!(
        node,
        WorkflowNode::McpTool(tool)
            if matches!(tool.tool_name.as_str(), "write_markdown_report" | "write_file" | "create_file")
    )
}

fn sandbox_path_argument_keys(
    tool: &McpToolNode,
) -> Option<(&'static str, &'static [&'static str])> {
    match tool.tool_name.as_str() {
        "folder_read" => Some(("folderPath", MCP_FOLDER_PATH_KEYS)),
        "list_directory" | "read_file" | "write_file" => Some(("path", MCP_FILE_PATH_KEYS)),
        "write_markdown_report" | "preview_report" => Some(("reportPath", MCP_REPORT_PATH_KEYS)),
        _ => None,
    }
}

fn validate_mcp_tool_sandbox_path(
    tool: &McpToolNode,
    field_label: &str,
    keys: &[&str],
) -> Result<(), WorkflowCompilerError> {
    let Some(arguments) = tool.arguments.as_object() else {
        return Err(sandbox_path_anomaly(
            tool,
            field_label,
            "Tool arguments must be an object.",
        ));
    };
    let Some(raw_path) = keys
        .iter()
        .find_map(|key| arguments.get(*key).map(|value| (*key, value)))
    else {
        return Err(sandbox_path_anomaly(
            tool,
            field_label,
            "The path is missing.",
        ));
    };
    let (path_key, path_value) = raw_path;
    let Some(path) = path_value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(sandbox_path_anomaly(
            tool,
            field_label,
            &format!("{path_key} must be a non-empty string."),
        ));
    };

    validate_static_sandbox_path(path).map_err(|reason| {
        sandbox_path_anomaly(
            tool,
            field_label,
            &format!("{path_key} must stay inside the approved workflow workspace. {reason}"),
        )
    })
}

fn validate_static_sandbox_path(path: &str) -> Result<(), String> {
    if path.contains("{{") || path.contains("}}") {
        return Err("Use a fixed sandbox-relative path instead of a runtime template.".to_string());
    }
    if path.starts_with('~') {
        return Err("Home-directory shortcuts are outside the workflow sandbox.".to_string());
    }

    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return Err("Absolute paths are outside the workflow sandbox.".to_string());
    }

    let mut saw_normal_component = false;
    for component in candidate.components() {
        match component {
            Component::Normal(_) => saw_normal_component = true,
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("Parent-directory traversal is not allowed.".to_string());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("Rooted paths are outside the workflow sandbox.".to_string());
            }
        }
    }

    if !saw_normal_component && path != "." {
        return Err("The path must name a folder or file inside the sandbox.".to_string());
    }

    Ok(())
}

fn sandbox_path_anomaly(
    tool: &McpToolNode,
    field_label: &str,
    reason: &str,
) -> WorkflowCompilerError {
    WorkflowCompilerError::topological_anomaly(
        WORKFLOW_TOPOLOGY_INVALID_SANDBOX_PATH_CODE,
        format!(
            "The {} step needs a safe {field_label} before it can run. {reason}",
            compact_error(&tool.label)
        ),
    )
}

fn resolve_missing_capability_details(
    entries: &[String],
    catalog: &CapabilityCatalog,
    partial_draft: Option<Value>,
) -> Result<Vec<MissingCapabilityDetail>, ComposeAttemptError> {
    if entries.is_empty() {
        return Err(ComposeAttemptError {
            message: "Gemma requested a connection but did not name a catalog capability."
                .to_string(),
            partial_draft,
            missing_capabilities: Vec::new(),
            missing_capability_details: Vec::new(),
        });
    }

    let mut details = Vec::<MissingCapabilityDetail>::new();
    for entry in entries {
        if looks_like_placeholder_capability(entry) {
            return Err(ComposeAttemptError {
                message: "Gemma returned a placeholder missing capability instead of a catalog title or id."
                    .to_string(),
                partial_draft,
                missing_capabilities: Vec::new(),
                missing_capability_details: Vec::new(),
            });
        }

        let normalized = normalize_catalog_match(entry);
        let action = catalog
            .actions
            .iter()
            .find(|action| {
                normalize_catalog_match(&action.id) == normalized
                    || normalize_catalog_match(&action.title) == normalized
            })
            .ok_or_else(|| ComposeAttemptError {
                message: format!(
                    "Gemma named missing capability '{}' but it does not appear in the catalog.",
                    compact_error(entry)
                ),
                partial_draft: partial_draft.clone(),
                missing_capabilities: Vec::new(),
                missing_capability_details: Vec::new(),
            })?;

        if action.available {
            return Err(ComposeAttemptError {
                message: format!(
                    "Gemma requested a connection for '{}' even though that capability is available.",
                    action.title
                ),
                partial_draft,
                missing_capabilities: Vec::new(),
                missing_capability_details: Vec::new(),
            });
        }

        let detail = missing_capability_detail_from_action(action);
        if !details.iter().any(|existing| existing.id == detail.id) {
            details.push(detail);
        }
    }

    Ok(details)
}

fn missing_capability_detail_from_action(action: &CapabilityAction) -> MissingCapabilityDetail {
    MissingCapabilityDetail {
        id: action.id.clone(),
        title: action.title.clone(),
        outcome: action.outcome.clone(),
        reason: action
            .unavailable_reason
            .clone()
            .unwrap_or_else(|| connect_reason_for_action(action)),
        source: action.source.clone(),
        server_name: action.server_name.clone(),
        tool_name: action.tool_name.clone(),
    }
}

fn missing_capability_titles(details: &[MissingCapabilityDetail]) -> Vec<String> {
    details.iter().map(|detail| detail.title.clone()).collect()
}

fn missing_capability_reason(details: &[MissingCapabilityDetail]) -> String {
    if details.is_empty() {
        return "Connect the missing capability before composing this workflow.".to_string();
    }
    details
        .iter()
        .map(|detail| detail.reason.clone())
        .collect::<Vec<_>>()
        .join(" ")
}

fn compose_failed_reason() -> String {
    "OOMU could not turn that into a runnable workflow. Try a simpler request, rephrase the outcome, or start from a template."
        .to_string()
}

fn compose_repair_hint(request: &ComposeWorkflowRequest) -> String {
    let matched = matched_grounding_actions(&request.prompt, &request.capability_catalog);
    if matched.is_empty() {
        return "Use only capabilities that appear in the supplied catalog.".to_string();
    }

    let capabilities = matched
        .iter()
        .take(8)
        .map(|action| match (&action.server_name, &action.tool_name) {
            (Some(server), Some(tool)) => format!("{} ({server}/{tool})", action.title),
            _ => action.title.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "The catalog contains available actions that may ground this request: {capabilities}. Compose a real workflowIr using matching available actions."
    )
}

fn matched_grounding_actions<'a>(
    prompt: &str,
    catalog: &'a CapabilityCatalog,
) -> Vec<&'a CapabilityAction> {
    let normalized_prompt = prompt.to_lowercase();
    let mut seen = HashSet::<String>::new();
    catalog
        .actions
        .iter()
        .filter(|action| {
            action.kind == "mcp_tool"
                && action.available
                && action.server_name.is_some()
                && action.tool_name.is_some()
                && action_matches_prompt(action, &normalized_prompt)
        })
        .filter(|action| seen.insert(action.id.clone()))
        .collect()
}

fn action_matches_prompt(action: &CapabilityAction, prompt: &str) -> bool {
    let tool_name = action.tool_name.as_deref().unwrap_or_default();
    if action.server_name.as_deref() == Some(registered_task_capabilities::REGISTERED_TASK_SERVER) {
        return registered_task_capabilities::matches_prompt(tool_name, prompt);
    }
    match tool_name {
        "read_system_emails" => has_local_app_intent(prompt, "mail"),
        "draft_system_email" => {
            has_local_app_intent(prompt, "mail") && has_mail_draft_intent(prompt)
        }
        "read_system_calendar" => has_local_app_intent(prompt, "calendar"),
        "read_system_reminders" => has_local_app_intent(prompt, "reminders"),
        "read_system_notes" => has_local_app_intent(prompt, "notes"),
        "read_system_contacts" => has_local_app_intent(prompt, "contacts"),
        "trigger_system_notification" => {
            prompt_has_any(prompt, &["notify", "notification", "alert"])
        }
        "read_file" => prompt_has_any(
            prompt,
            &[
                "file", "folder", "note", "notes", "sandbox", "local", "read", "scan",
            ],
        ),
        "list_directory" => prompt_has_any(prompt, &["folder", "directory", "list", "scan"]),
        "write_file" => prompt_has_any(
            prompt,
            &[
                "write", "save", "report", "markdown", "disk", "file", "summary",
            ],
        ),
        "folder_read" => prompt_has_any(prompt, &["folder", "project", "notes", "scan"]),
        "write_markdown_report" => has_report_write_intent(prompt),
        "preview_report" => has_report_preview_intent(prompt),
        _ => action_text_matches_prompt(action, prompt),
    }
}

fn action_text_matches_prompt(action: &CapabilityAction, prompt: &str) -> bool {
    let action_text = format!(
        "{} {} {} {} {}",
        action.id,
        action.title,
        action.outcome,
        action.detail,
        action.tool_name.as_deref().unwrap_or_default()
    )
    .to_lowercase();
    prompt
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|term| term.len() >= 4 && !is_prompt_stopword(term))
        .any(|term| action_text.contains(term))
}

fn prompt_has_any(prompt: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| prompt.contains(term))
}

fn has_mail_draft_intent(prompt: &str) -> bool {
    prompt_has_any(
        prompt,
        &["draft", "drafting", "reply", "replies", "open", "send"],
    ) && prompt_has_any(
        prompt,
        &[
            "mail", "email", "inbox", "message", "messages", "reply", "replies",
        ],
    )
}

fn has_report_document_intent(prompt: &str) -> bool {
    prompt_has_any(prompt, &["report", "markdown"])
        || prompt.contains("markdown summary")
        || prompt.contains("project summary")
        || prompt.contains("save project summary")
}

fn has_report_write_intent(prompt: &str) -> bool {
    has_report_document_intent(prompt)
        && (prompt_has_any(
            prompt,
            &[
                "write", "writing", "written", "save", "saved", "generate", "create",
            ],
        ) || prompt.contains("markdown summary")
            || prompt.contains("save project summary"))
}

fn has_report_preview_intent(prompt: &str) -> bool {
    has_report_document_intent(prompt)
        && prompt_has_any(prompt, &["preview", "open", "opening", "review", "inspect"])
}

fn is_prompt_stopword(term: &str) -> bool {
    matches!(
        term,
        "this" | "that" | "from" | "with" | "into" | "before" | "after" | "create" | "make"
    )
}

fn normalize_native_tool_arguments(ir: &mut WorkflowIr) -> Result<(), WorkflowCompilerError> {
    let incoming_references = incoming_node_references(ir);
    let native_schemas = native_taskflow_input_schemas()?;
    for node in &mut ir.nodes {
        let WorkflowNode::McpTool(tool) = node else {
            continue;
        };
        if tool.server_name != TASKFLOW_NATIVE_SERVER {
            continue;
        }
        if tool.input_schema.is_none() {
            tool.input_schema = native_schemas.get(&tool.tool_name).cloned();
        }
        let upstream_reference = incoming_references
            .get(&tool.id)
            .cloned()
            .unwrap_or_else(|| "{{workflow.input}}".to_string());
        merge_missing_native_tool_arguments(tool, &upstream_reference);
    }
    Ok(())
}

fn incoming_node_references(ir: &WorkflowIr) -> HashMap<String, String> {
    ir.edges
        .iter()
        .filter(|edge| !edge.target_node_id.trim().is_empty())
        .map(|edge| {
            (
                edge.target_node_id.clone(),
                workflow_reference(&edge.source_node_id),
            )
        })
        .collect()
}

fn merge_missing_native_tool_arguments(tool: &mut McpToolNode, upstream_reference: &str) {
    let defaults = match tool.tool_name.as_str() {
        "write_markdown_report" => json!({
            "reportPath": TASKFLOW_DEFAULT_REPORT_PATH,
            "content": upstream_reference
        }),
        "preview_report" => json!({
            "reportPath": TASKFLOW_DEFAULT_REPORT_PATH
        }),
        _ => return,
    };

    if !tool.arguments.is_object() {
        tool.arguments = json!({});
    }
    let Some(arguments) = tool.arguments.as_object_mut() else {
        return;
    };
    let Some(defaults) = defaults.as_object() else {
        return;
    };

    for (key, value) in defaults {
        if arguments.get(key).map_or(true, |existing| {
            should_apply_argument_default(existing, value)
        }) {
            arguments.insert(key.clone(), value.clone());
        }
    }
}

fn should_apply_argument_default(existing: &Value, fallback: &Value) -> bool {
    match fallback {
        Value::String(_) => existing.as_str().map(str::trim).map_or(true, str::is_empty),
        _ => existing.is_null(),
    }
}

#[cfg(test)]
fn workflow_edge(source_node_id: &str, source_port: &str, target_node_id: &str) -> WorkflowEdge {
    WorkflowEdge {
        id: format!("edge-{source_node_id}-{target_node_id}"),
        source_node_id: source_node_id.to_string(),
        source_port: source_port.to_string(),
        target_node_id: target_node_id.to_string(),
        target_port: None,
    }
}

fn workflow_reference(node_id: &str) -> String {
    if node_id == "input" {
        "{{workflow.input}}".to_string()
    } else {
        format!("{{{{nodes.{node_id}.output}}}}")
    }
}

fn looks_like_placeholder_reason(reason: &str) -> bool {
    let normalized = reason.trim().to_lowercase();
    if normalized.is_empty() {
        return false;
    }
    normalized == "connect y to do x."
        || normalized == "connect y to do x"
        || normalized.contains("<capability_name>")
        || normalized.contains("connect y")
        || normalized.contains("do x")
        || contains_placeholder_word(&normalized, false)
}

fn looks_like_placeholder_capability(capability: &str) -> bool {
    let normalized = capability.trim().to_lowercase();
    normalized.is_empty()
        || matches!(
            normalized.as_str(),
            "x" | "y" | "kind" | "capability" | "capability_name" | "<capability_name>"
        )
        || normalized.contains("<capability_name>")
}

fn contains_placeholder_word(value: &str, include_kind: bool) -> bool {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|word| matches!(word, "x" | "y") || (include_kind && word == "kind"))
}

fn normalize_catalog_match(value: &str) -> String {
    value.trim().to_lowercase()
}

fn compose_catalog_prompt_payload(catalog: &CapabilityCatalog) -> Value {
    json!({
        "actions": catalog
            .actions
            .iter()
            .take(120)
            .map(|action| {
                json!({
                    "id": action.id,
                    "kind": action.kind,
                    "title": action.title,
                    "outcome": action.outcome,
                    "selectionHint": capability_selection_hint(action),
                    "available": action.available,
                    "serverName": action.server_name,
                    "toolName": action.tool_name,
                    "inputSchema": action.input_schema,
                    "outputSchema": action.output_schema,
                    "unavailableReason": action.unavailable_reason,
                })
            })
            .collect::<Vec<_>>(),
        "templates": catalog
            .templates
            .iter()
            .take(8)
            .map(|template| {
                json!({
                    "id": template.id,
                    "name": template.name,
                    "description": template.description,
                    "seedPrompt": template.seed_prompt,
                    "workflowIr": template.workflow_ir,
                })
            })
            .collect::<Vec<_>>(),
    })
}

fn capability_selection_hint(action: &CapabilityAction) -> Option<&'static str> {
    registered_task_capabilities::selection_hint(action)
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end >= start).then_some(&text[start..=end])
}

fn compose_session_id(prompt: &str) -> String {
    format!("workflow-compose:{}", short_hash(prompt))
}

fn edit_session_id(workflow_ir: &WorkflowIr, instruction: &str) -> String {
    format!(
        "workflow-edit:{}:{}",
        workflow_ir.workflow_id,
        short_hash(instruction)
    )
}

fn short_hash(value: &str) -> String {
    sha256_hex(value.as_bytes())
        .chars()
        .take(16)
        .collect::<String>()
}

fn non_empty_or(value: String, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn graph_authoring_capabilities() -> Vec<CapabilityAction> {
    vec![
        library_capability(
            "library:draft:create-draft",
            "agent",
            "Draft a reply or summary",
            "Write a reply, summary, or brief from what the workflow just read.",
            "agent",
        ),
        library_capability(
            "library:system_action:bounded-local-command",
            "system_action",
            "Use a local action",
            "Open or update something on this Mac using a saved workflow step.",
            "system_action",
        ),
    ]
}

fn known_mcp_capabilities(tool_catalog: &HashMap<String, Vec<McpTool>>) -> Vec<CapabilityAction> {
    vec![
        known_mcp_capability(
            tool_catalog,
            "local_filesystem",
            "list_directory",
            "List files in the workflow folder",
            "See what files are available in the local workflow folder.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ),
        known_mcp_capability(
            tool_catalog,
            "local_filesystem",
            "read_file",
            "Read a file from the workflow folder",
            "Read a local file that the workflow is allowed to use.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ),
        known_mcp_capability(
            tool_catalog,
            "local_filesystem",
            "write_file",
            "Write a file to the workflow folder",
            "Save generated text into the local workflow folder.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        ),
        known_mcp_capability(
            tool_catalog,
            "macos_applescript",
            "read_system_calendar",
            "Read your Calendar",
            "Read upcoming events from Calendar on this Mac.",
            json!({
                "type": "object",
                "properties": {
                    "calendar_name": { "type": "string" },
                    "hours_ahead": { "type": "number", "minimum": 0.25, "maximum": 720 },
                    "start_date": { "type": "string" },
                    "end_date": { "type": "string" }
                },
                "additionalProperties": false
            }),
        ),
        known_mcp_capability(
            tool_catalog,
            "macos_applescript",
            "trigger_system_notification",
            "Show a Mac notification",
            "Display a native notification on this Mac.",
            json!({
                "type": "object",
                "properties": {
                    "title_text": { "type": "string" },
                    "subtitle_text": { "type": "string" },
                    "body_text": { "type": "string" }
                },
                "required": ["body_text"],
                "additionalProperties": false
            }),
        ),
        known_mcp_capability(
            tool_catalog,
            "macos_applescript",
            "draft_system_email",
            "Open a Mail draft for review",
            "Prepare a visible Apple Mail draft that you can review before sending.",
            json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string" },
                    "subject": { "type": "string" },
                    "body": { "type": "string" },
                    "cc": { "type": "string" },
                    "bcc": { "type": "string" }
                },
                "required": ["subject", "body"],
                "additionalProperties": false
            }),
        ),
        known_mcp_capability(
            tool_catalog,
            "macos_applescript",
            "read_system_emails",
            "Read your Mail",
            "Read recent messages from Mail on this Mac.",
            json!({
                "type": "object",
                "properties": {
                    "max_messages": { "type": "number", "minimum": 1, "maximum": 50 },
                    "unread_only": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
        ),
        known_mcp_capability(
            tool_catalog,
            "macos_applescript",
            "read_system_reminders",
            "Read Reminders",
            "Read tasks from the local Reminders app.",
            json!({
                "type": "object",
                "properties": {
                    "list_name": { "type": "string" },
                    "completed_only": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
        ),
    ]
}

fn native_taskflow_input_schemas() -> Result<HashMap<String, Value>, WorkflowCompilerError> {
    native_taskflow_tools()
        .map_err(WorkflowCompilerError::metadata)
        .map(|tools| {
            tools
                .into_iter()
                .map(|tool| (tool.name, tool.input_schema))
                .collect()
        })
}

fn native_taskflow_output_schemas() -> Result<HashMap<String, Value>, WorkflowCompilerError> {
    native_taskflow_tools()
        .map_err(WorkflowCompilerError::metadata)
        .map(|tools| {
            tools
                .into_iter()
                .filter_map(|tool| tool.output_schema.map(|schema| (tool.name, schema)))
                .collect()
        })
}

fn native_schema_for_tool(
    schemas: &HashMap<String, Value>,
    tool_name: &str,
) -> Result<Value, WorkflowCompilerError> {
    schemas.get(tool_name).cloned().ok_or_else(|| {
        WorkflowCompilerError::metadata(format!(
            "Native taskflow tool schema is missing for {tool_name}."
        ))
    })
}

fn taskflow_native_capabilities() -> Result<Vec<CapabilityAction>, WorkflowCompilerError> {
    let schemas = native_taskflow_input_schemas()?;
    let output_schemas = native_taskflow_output_schemas()?;
    Ok(vec![
        native_capability(
            "native:taskflow:folder-read",
            "Read an approved project folder",
            "Scan text files from a folder you have approved for the workflow.",
            TASKFLOW_NATIVE_SERVER,
            "folder_read",
            native_schema_for_tool(&schemas, "folder_read")?,
            output_schemas.get("folder_read").cloned(),
        ),
        native_capability(
            "native:taskflow:write-markdown-report",
            "Write a project report",
            "Write a Markdown report into the approved project folder.",
            TASKFLOW_NATIVE_SERVER,
            "write_markdown_report",
            native_schema_for_tool(&schemas, "write_markdown_report")?,
            output_schemas.get("write_markdown_report").cloned(),
        ),
        native_capability(
            "native:taskflow:preview-report",
            "Open the report for review",
            "Open the generated report on this Mac so you can inspect it.",
            TASKFLOW_NATIVE_SERVER,
            "preview_report",
            native_schema_for_tool(&schemas, "preview_report")?,
            output_schemas.get("preview_report").cloned(),
        ),
    ])
}

fn library_capability(
    id: &str,
    kind: &str,
    title: &str,
    outcome: &str,
    node_kind: &str,
) -> CapabilityAction {
    CapabilityAction {
        id: id.to_string(),
        kind: kind.to_string(),
        title: title.to_string(),
        outcome: outcome.to_string(),
        detail: outcome.to_string(),
        source: "library".to_string(),
        available: true,
        availability: "available".to_string(),
        unavailable_reason: None,
        server_name: None,
        tool_name: None,
        input_schema: None,
        output_schema: None,
        node_kind: Some(node_kind.to_string()),
        node_template: None,
    }
}

fn native_capability(
    id: &str,
    title: &str,
    outcome: &str,
    server_name: &str,
    tool_name: &str,
    input_schema: Value,
    output_schema: Option<Value>,
) -> CapabilityAction {
    CapabilityAction {
        id: id.to_string(),
        kind: "mcp_tool".to_string(),
        title: title.to_string(),
        outcome: outcome.to_string(),
        detail: outcome.to_string(),
        source: "native".to_string(),
        available: true,
        availability: "available".to_string(),
        unavailable_reason: None,
        server_name: Some(server_name.to_string()),
        tool_name: Some(tool_name.to_string()),
        input_schema: Some(input_schema),
        output_schema,
        node_kind: Some("mcp".to_string()),
        node_template: None,
    }
}

fn known_mcp_capability(
    tool_catalog: &HashMap<String, Vec<McpTool>>,
    server_name: &str,
    tool_name: &str,
    title: &str,
    outcome: &str,
    fallback_schema: Value,
) -> CapabilityAction {
    let tool = tool_catalog
        .get(server_name)
        .and_then(|tools| tools.iter().find(|tool| tool.name == tool_name));
    let available = tool.is_some();
    let mut action = CapabilityAction {
        id: mcp_capability_id(server_name, tool_name),
        kind: "mcp_tool".to_string(),
        title: title.to_string(),
        outcome: outcome.to_string(),
        detail: tool
            .and_then(|tool| {
                (!tool.description.trim().is_empty()).then(|| tool.description.clone())
            })
            .unwrap_or_else(|| outcome.to_string()),
        source: "mcp".to_string(),
        available,
        availability: if available {
            "available".to_string()
        } else {
            "requires_connection".to_string()
        },
        unavailable_reason: None,
        server_name: Some(server_name.to_string()),
        tool_name: Some(tool_name.to_string()),
        input_schema: Some(
            tool.map(|tool| tool.input_schema.clone())
                .unwrap_or(fallback_schema),
        ),
        output_schema: tool.and_then(|tool| tool.output_schema.clone()),
        node_kind: Some("mcp".to_string()),
        node_template: None,
    };
    if !available {
        action.unavailable_reason = Some(connect_reason_for_action(&action));
    }
    action
}

fn mcp_capability_from_tool(server_name: &str, tool: &McpTool) -> CapabilityAction {
    CapabilityAction {
        id: mcp_capability_id(server_name, &tool.name),
        kind: "mcp_tool".to_string(),
        title: humanize_identifier(&tool.name),
        outcome: if tool.description.trim().is_empty() {
            format!(
                "Use {} from {}.",
                humanize_identifier(&tool.name),
                humanize_identifier(server_name)
            )
        } else {
            tool.description.clone()
        },
        detail: tool.description.clone(),
        source: "mcp".to_string(),
        available: true,
        availability: "available".to_string(),
        unavailable_reason: None,
        server_name: Some(server_name.to_string()),
        tool_name: Some(tool.name.clone()),
        input_schema: Some(tool.input_schema.clone()),
        output_schema: tool.output_schema.clone(),
        node_kind: Some("mcp".to_string()),
        node_template: None,
    }
}

fn dedupe_capability_actions(actions: Vec<CapabilityAction>) -> Vec<CapabilityAction> {
    let mut by_id = HashMap::<String, CapabilityAction>::new();
    for action in actions {
        match by_id.get(&action.id) {
            Some(existing) if existing.available && !action.available => {}
            _ => {
                by_id.insert(action.id.clone(), action);
            }
        }
    }
    let mut actions = by_id.into_values().collect::<Vec<_>>();
    actions.sort_by(|left, right| left.title.cmp(&right.title));
    actions
}

fn connect_reason_for_action(action: &CapabilityAction) -> String {
    match (&action.server_name, &action.tool_name) {
        (Some(server), Some(tool)) => {
            format!("Connect {server} to use {tool} for {}.", action.outcome)
        }
        _ => format!("Connect the required integration for {}.", action.outcome),
    }
}

fn mcp_capability_id(server_name: &str, tool_name: &str) -> String {
    format!("mcp:{server_name}:{tool_name}")
}

fn humanize_identifier(value: &str) -> String {
    value
        .split(['_', '-'])
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

fn compiler_output_grammar() -> &'static str {
    r#"
root ::= ws "{" ws "\"compilerVersion\"" ws ":" ws "\"1.0.0\"" ws "," ws "\"instructions\"" ws ":" ws instructions ws "}" ws
instructions ::= "[" ws (instruction (ws "," ws instruction)*)? ws "]"
instruction ::= "{" ws "\"nodeId\"" ws ":" ws string ws "," ws "\"systemPrompt\"" ws ":" ws string ws "," ws "\"inputVariableMappings\"" ws ":" ws mappings ws "," ws "\"evaluationProtocol\"" ws ":" ws protocol ws "}"
mappings ::= "[" ws (mapping (ws "," ws mapping)*)? ws "]"
mapping ::= "{" ws "\"name\"" ws ":" ws string ws "," ws "\"template\"" ws ":" ws string ws "}"
protocol ::= "{" ws "\"successCriteria\"" ws ":" ws strings ws "," ws "\"failureAction\"" ws ":" ws action ws "," ws "\"maxRetries\"" ws ":" ws integer ws "}"
strings ::= "[" ws (string (ws "," ws string)*)? ws "]"
action ::= "\"fail\"" | "\"retry\"" | "\"route\""
integer ::= [0-9]+
string ::= "\"" chars "\""
chars ::= ([^"\\] | "\\" ["\\/bfnrt] | "\\u" hex hex hex hex)*
hex ::= [0-9a-fA-F]
ws ::= [ \t\n\r]*
	"#
}

fn compose_output_grammar() -> &'static str {
    r#"
root ::= ws object ws
value ::= object | array | string | number | "true" | "false" | "null"
object ::= "{" ws (member (ws "," ws member)*)? ws "}"
member ::= string ws ":" ws value
array ::= "[" ws (value (ws "," ws value)*)? ws "]"
number ::= "-"? ([0-9] | [1-9] [0-9]*) ("." [0-9]+)? ([eE] [+-]? [0-9]+)?
string ::= "\"" chars "\""
chars ::= ([^"\\] | "\\" ["\\/bfnrt] | "\\u" hex hex hex hex)*
hex ::= [0-9a-fA-F]
ws ::= [ \t\n\r]*
	"#
}

fn instruction_id(workflow_id: &str, version: u32, node_id: &str) -> String {
    format!(
        "wci-{}",
        sha256_hex(format!("{workflow_id}:{version}:{node_id}").as_bytes())
    )
}

fn run_workflow_compiler_guard<T>(
    operation: &'static str,
    work: impl FnOnce() -> Result<T, WorkflowCompilerError>,
) -> Result<T, WorkflowCompilerError> {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|payload| {
        let error = workflow_compiler_panic_error(operation, payload);
        eprintln!(
            "OOMU_WORKFLOW_COMPILER_PANIC operation={} message={}",
            operation, error.message
        );
        Err(error)
    })
}

fn workflow_compiler_panic_error(
    operation: &'static str,
    payload: Box<dyn Any + Send>,
) -> WorkflowCompilerError {
    let message = panic_payload_message(payload);
    WorkflowCompilerError::runtime(format!(
        "Workflow compiler worker panicked during {operation}: {message}"
    ))
}

fn panic_payload_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

#[cfg(test)]
mod tests;
