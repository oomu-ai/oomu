//! Sandboxed workflow system-action execution and bounded self-healing.
use super::{
    command_preview, configured_node_timeout_ms, elapsed_ms, execute_sync_knowledge_vault_node,
    long_running_operation_hint, normalize_runtime_identifier, pause_for_external_action,
    resolve_template_to_string, resume_decision_for_node,
    sync_knowledge_vault_arguments_from_system_action, ActionNodeStep, NodeOutcome,
    PermissionDecision, ResumePermission, RuntimeExternalTools, RuntimeModel, WorkflowRuntimeError,
    SYNC_KNOWLEDGE_VAULT_TOOL,
};
use crate::{
    security::sandbox::{
        build_sandboxed_command, SandboxCommandKind, SandboxCommandRequest,
        SandboxExecutionMetadata, SandboxRoot,
    },
    tool_security::{
        audit_workspace_execution_payload, classify_system_action, CapabilityClassification,
        SandboxPolicy, SystemActionClass,
    },
    workflow_ir::{ExecutionInstance, SystemActionNode, SystemActionType},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const SYSTEM_ACTION_DEFAULT_TIMEOUT_MS: u64 = crate::workflow_ir::SHORT_TIMEOUT_MS;
const SYSTEM_ACTION_MAX_TIMEOUT_MS: u64 = crate::workflow_ir::LONG_TIMEOUT_MS;
const SYSTEM_ACTION_MAX_OUTPUT_BYTES: usize = 50 * 1024;
const SYSTEM_ACTION_POLL_MS: u64 = 10;

#[derive(Debug, Clone)]
pub(super) struct SystemActionFailureContext {
    pub(super) action_type: SystemActionType,
    pub(super) command: String,
    pub(super) args: Vec<String>,
    pub(super) working_directory: String,
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

pub(super) fn execute_system_action_node(
    system_action: &SystemActionNode,
    model: &impl RuntimeModel,
    external_tools: &impl RuntimeExternalTools,
    workspace_root: &Path,
    instance: &mut ExecutionInstance,
    input: Option<Value>,
    memory: &HashMap<String, Value>,
    selected_edges: &HashSet<String>,
    latency_ms: u64,
    resume_permission: Option<&ResumePermission>,
) -> Result<ActionNodeStep, WorkflowRuntimeError> {
    let command = resolve_template_to_string(&system_action.command, memory)?;
    let args = system_action
        .args
        .iter()
        .map(|arg| resolve_template_to_string(arg, memory))
        .collect::<Result<Vec<_>, _>>()?;
    let working_directory = system_action
        .working_directory
        .as_deref()
        .map(|directory| resolve_template_to_string(directory, memory))
        .transpose()?;
    let timeout_ms = system_action_timeout_ms(system_action);
    audit_system_action(system_action, &command, &args, &working_directory)?;
    if normalize_runtime_identifier(&command) == SYNC_KNOWLEDGE_VAULT_TOOL {
        let arguments =
            sync_knowledge_vault_arguments_from_system_action(&args, working_directory.as_deref())?;
        return execute_sync_knowledge_vault_node(
            &system_action.id,
            &system_action.label,
            external_tools,
            arguments,
            timeout_ms,
            json!({
                "actionType": "system_action",
                "command": command,
                "args": args,
                "workingDirectory": working_directory,
            }),
        );
    }
    let classification = classify_system_action(
        system_action_class(&system_action.action_type),
        &command,
        &args,
    );
    let sandbox_policy = classification.sandbox_policy();
    if let Some(paused) = system_action_approval_step(SystemActionApprovalContext {
        system_action,
        command: &command,
        args: &args,
        working_directory: &working_directory,
        classification: &classification,
        sandbox_policy: &sandbox_policy,
        instance,
        input,
        memory,
        selected_edges,
        latency_ms,
        resume_permission,
        timeout_ms,
    })? {
        return Ok(paused);
    }

    let run_workspace_root = workspace_root.join(&instance.id);
    let working_directory =
        resolve_system_working_directory(&run_workspace_root, working_directory)?;
    let max_output_bytes = system_action
        .max_output_bytes
        .clamp(1, SYSTEM_ACTION_MAX_OUTPUT_BYTES);
    let result = run_system_action(
        &system_action.action_type,
        &command,
        &args,
        &run_workspace_root,
        &working_directory,
        bounded_system_timeout_ms(timeout_ms),
        max_output_bytes,
    )?;
    if result.timed_out {
        return Err(WorkflowRuntimeError::node_timeout(
            &system_action.id,
            &system_action.label,
            timeout_ms,
        ));
    }
    let (result, repair_metadata) = if system_action_succeeded(&result) {
        (result, Value::Null)
    } else {
        let original_exit_code = result.exit_code;
        let repaired = attempt_system_action_self_heal(
            system_action,
            model,
            &instance.id,
            &command,
            &args,
            &run_workspace_root,
            &working_directory,
            result,
            timeout_ms,
            max_output_bytes,
        )?;
        (
            repaired.result,
            json!({
                "attempted": true,
                "originalExitCode": original_exit_code,
                "explanation": repaired.explanation,
                "command": repaired.command,
                "args": repaired.args,
            }),
        )
    };

    Ok(ActionNodeStep::Completed(NodeOutcome::output(
        json!({
            "mediaType": "application/json",
            "data": {
                "actionType": system_action.action_type,
                "command": command,
                "args": args,
                "workingDirectory": working_directory.to_string_lossy(),
                "exitCode": result.exit_code,
                "timedOut": result.timed_out,
                "stdout": result.stdout.text,
                "stderr": result.stderr.text,
                "stdoutTruncated": result.stdout.truncated,
                "stderrTruncated": result.stderr.truncated,
                "durationMs": result.duration_ms,
            },
            "assetPath": null,
            "metadata": {
                "maxOutputBytes": max_output_bytes,
                "timeoutMs": bounded_system_timeout_ms(timeout_ms),
                "selfHeal": repair_metadata,
                "sandbox": result.sandbox,
            }
        }),
        vec!["out".to_string()],
    )))
}

fn audit_system_action(
    system_action: &SystemActionNode,
    command: &str,
    args: &[String],
    working_directory: &Option<String>,
) -> Result<(), WorkflowRuntimeError> {
    let payload = json!({
        "actionType": "system_action",
        "mode": &system_action.action_type,
        "command": command,
        "args": args,
        "workingDirectory": working_directory,
    })
    .to_string();
    audit_workspace_execution_payload(&payload)
        .map(|_| ())
        .map_err(|violation| WorkflowRuntimeError::execution(violation.message))
}

struct SystemActionApprovalContext<'a> {
    system_action: &'a SystemActionNode,
    command: &'a str,
    args: &'a [String],
    working_directory: &'a Option<String>,
    classification: &'a CapabilityClassification,
    sandbox_policy: &'a SandboxPolicy,
    instance: &'a mut ExecutionInstance,
    input: Option<Value>,
    memory: &'a HashMap<String, Value>,
    selected_edges: &'a HashSet<String>,
    latency_ms: u64,
    resume_permission: Option<&'a ResumePermission>,
    timeout_ms: u64,
}

fn system_action_approval_step(
    context: SystemActionApprovalContext<'_>,
) -> Result<Option<ActionNodeStep>, WorkflowRuntimeError> {
    if !context.classification.requires_human_approval() {
        return Ok(None);
    }
    match resume_decision_for_node(&context.system_action.id, context.resume_permission) {
        Some(PermissionDecision::Approve) => Ok(None),
        Some(PermissionDecision::Reject) => {
            Err(WorkflowRuntimeError::permission_rejected(&format!(
                "System action {} was rejected.",
                context.system_action.label
            )))
        }
        None => {
            let approval = pause_for_external_action(
                context.instance,
                &context.system_action.id,
                &format!(
                    "Approve system action: {}",
                    command_preview(context.command, context.args)
                ),
                json!({
                    "actionType": "system_action",
                    "mode": context.system_action.action_type,
                    "command": context.command,
                    "args": context.args,
                    "workingDirectory": context.working_directory,
                    "capabilityRiskTier": context.classification.tier.as_str(),
                    "capabilityReason": context.classification.reason.clone(),
                    "sandboxRequired": context.sandbox_policy.required,
                    "sandboxReason": context.sandbox_policy.reason.clone(),
                    "sandboxNetworkEnabled": context.sandbox_policy.network_enabled,
                    "timeoutMs": bounded_system_timeout_ms(context.timeout_ms),
                    "systemTimeoutMs": bounded_system_timeout_ms(context.timeout_ms),
                    "input": context.input.clone(),
                    "memoryKeys": context.memory.keys().collect::<Vec<_>>(),
                }),
                context.input,
                context.memory,
                context.selected_edges,
                context.latency_ms,
            )?;
            Ok(Some(ActionNodeStep::Paused(approval)))
        }
    }
}

#[cfg(test)]
pub(super) fn high_risk_action(
    action_type: &SystemActionType,
    command: &str,
    args: &[String],
) -> bool {
    classify_system_action(system_action_class(action_type), command, args)
        .requires_human_approval()
}

fn system_action_class(action_type: &SystemActionType) -> SystemActionClass {
    match action_type {
        SystemActionType::Shell => SystemActionClass::Shell,
        SystemActionType::Python => SystemActionClass::Python,
        SystemActionType::Binary => SystemActionClass::Binary,
    }
}

fn bounded_system_timeout_ms(timeout_ms: u64) -> u64 {
    let defaulted = if timeout_ms == 0 {
        SYSTEM_ACTION_DEFAULT_TIMEOUT_MS
    } else {
        timeout_ms
    };
    defaulted.clamp(1, SYSTEM_ACTION_MAX_TIMEOUT_MS)
}

fn system_action_timeout_ms(system_action: &SystemActionNode) -> u64 {
    let configured = system_action.system_timeout_ms.or_else(|| {
        (system_action.timeout_ms != SYSTEM_ACTION_DEFAULT_TIMEOUT_MS)
            .then_some(system_action.timeout_ms)
    });
    configured_node_timeout_ms(
        configured,
        default_system_action_node_timeout_ms(system_action),
    )
}

fn default_system_action_node_timeout_ms(system_action: &SystemActionNode) -> u64 {
    let preview = format!(
        "{:?} {} {} {}",
        system_action.action_type,
        system_action.command,
        system_action.args.join(" "),
        system_action
            .working_directory
            .as_deref()
            .unwrap_or_default()
    );
    if long_running_operation_hint(&preview) {
        crate::workflow_ir::LONG_TIMEOUT_MS
    } else {
        crate::workflow_ir::SHORT_TIMEOUT_MS
    }
}

fn resolve_system_working_directory(
    workspace_root: &Path,
    working_directory: Option<String>,
) -> Result<PathBuf, WorkflowRuntimeError> {
    let sandbox =
        SandboxRoot::new(workspace_root.to_path_buf()).map_err(WorkflowRuntimeError::execution)?;
    let directory = working_directory
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let directory = if directory.is_absolute() {
        directory
    } else {
        workspace_root.join(directory)
    };
    let directory = sandbox
        .resolve(directory)
        .map_err(WorkflowRuntimeError::execution)?;
    fs::create_dir_all(&directory).map_err(WorkflowRuntimeError::io)?;
    Ok(directory)
}

pub(super) struct LimitedPipeOutput {
    pub(super) text: String,
    pub(super) truncated: bool,
}

pub(super) struct SystemActionResult {
    pub(super) exit_code: Option<i32>,
    pub(super) timed_out: bool,
    pub(super) stdout: LimitedPipeOutput,
    pub(super) stderr: LimitedPipeOutput,
    pub(super) duration_ms: u64,
    pub(super) sandbox: Option<SandboxExecutionMetadata>,
}

struct RepairedSystemAction {
    result: SystemActionResult,
    command: String,
    args: Vec<String>,
    explanation: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SystemActionRepairPlan {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    explanation: String,
}

pub(super) fn run_system_action(
    action_type: &SystemActionType,
    command: &str,
    args: &[String],
    workspace_root: &Path,
    working_directory: &Path,
    timeout_ms: u64,
    max_output_bytes: usize,
) -> Result<SystemActionResult, WorkflowRuntimeError> {
    let native_executable = match action_type {
        SystemActionType::Python => Some(
            crate::mcp::bootstrap::resolve_system_python3_headless()
                .map(PathBuf::from)
                .map_err(WorkflowRuntimeError::execution)?,
        ),
        _ => None,
    };
    let classification = classify_system_action(system_action_class(action_type), command, args);
    let sandbox_policy = classification.sandbox_policy();
    let (mut process, sandbox) = if sandbox_policy.required {
        let launch = build_sandboxed_command(SandboxCommandRequest {
            kind: sandbox_command_kind(action_type),
            command: command.to_string(),
            args: args.to_vec(),
            native_executable: native_executable.clone(),
            workspace_root: workspace_root.to_path_buf(),
            working_directory: working_directory.to_path_buf(),
            network_enabled: sandbox_policy.network_enabled,
        })
        .map_err(|error| {
            WorkflowRuntimeError::execution(format!(
                "System action requires sandboxing but no sandbox launch could be prepared: {error}"
            ))
        })?;
        (launch.process, Some(launch.metadata))
    } else {
        (
            system_command(action_type, command, args, native_executable.as_deref()),
            None,
        )
    };
    process
        .current_dir(working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = process.spawn().map_err(WorkflowRuntimeError::io)?;
    let stdout = child.stdout.take().ok_or_else(|| {
        WorkflowRuntimeError::execution("System action did not expose stdout.".to_string())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        WorkflowRuntimeError::execution("System action did not expose stderr.".to_string())
    })?;
    let stdout_reader = spawn_limited_reader(stdout, max_output_bytes);
    let stderr_reader = spawn_limited_reader(stderr, max_output_bytes);
    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(WorkflowRuntimeError::io)? {
            break status;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            break child.wait().map_err(WorkflowRuntimeError::io)?;
        }
        thread::sleep(Duration::from_millis(SYSTEM_ACTION_POLL_MS));
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| WorkflowRuntimeError::runtime("stdout reader panicked".to_string()))?
        .map_err(WorkflowRuntimeError::io)?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| WorkflowRuntimeError::runtime("stderr reader panicked".to_string()))?
        .map_err(WorkflowRuntimeError::io)?;
    if let Some(metadata) = sandbox.as_ref() {
        log_sandbox_result(
            command,
            args,
            classification.tier.as_str(),
            metadata,
            &stdout,
            &stderr,
        );
    }
    Ok(SystemActionResult {
        exit_code: status.code(),
        timed_out,
        stdout,
        stderr,
        duration_ms: elapsed_ms(started),
        sandbox,
    })
}

fn system_action_succeeded(result: &SystemActionResult) -> bool {
    !result.timed_out && result.exit_code == Some(0)
}

fn sandbox_command_kind(action_type: &SystemActionType) -> SandboxCommandKind {
    match action_type {
        SystemActionType::Shell => SandboxCommandKind::Shell,
        SystemActionType::Python => SandboxCommandKind::Python,
        SystemActionType::Binary => SandboxCommandKind::Binary,
    }
}

fn log_sandbox_result(
    command: &str,
    args: &[String],
    risk_tier: &str,
    metadata: &SandboxExecutionMetadata,
    stdout: &LimitedPipeOutput,
    stderr: &LimitedPipeOutput,
) {
    let combined = format!("{} {}", stdout.text, stderr.text).to_ascii_lowercase();
    if sandbox_output_indicates_security_block(&combined) {
        eprintln!(
            "OOMU_SANDBOX_SECURITY_BLOCKED engine={} risk_tier={} network_enabled={} command={} stdout={} stderr={}",
            metadata.engine.as_str(),
            risk_tier,
            metadata.network_enabled,
            command_preview(command, args),
            compact_runtime_text(&stdout.text),
            compact_runtime_text(&stderr.text)
        );
    }
}

fn sandbox_output_indicates_security_block(output: &str) -> bool {
    output.contains("operation not permitted")
        || output.contains("permission denied")
        || output.contains("permissionerror")
        || output.contains("not permitted")
        || output.contains("_blocked")
        || output.contains("sandbox")
}

fn compact_runtime_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect()
}

fn attempt_system_action_self_heal(
    system_action: &SystemActionNode,
    model: &impl RuntimeModel,
    instance_id: &str,
    command: &str,
    args: &[String],
    workspace_root: &Path,
    working_directory: &Path,
    failed: SystemActionResult,
    timeout_ms: u64,
    max_output_bytes: usize,
) -> Result<RepairedSystemAction, WorkflowRuntimeError> {
    let failure = SystemActionFailureContext {
        action_type: system_action.action_type.clone(),
        command: command.to_string(),
        args: args.to_vec(),
        working_directory: working_directory.to_string_lossy().to_string(),
        exit_code: failed.exit_code,
        stdout: failed.stdout.text.clone(),
        stderr: failed.stderr.text.clone(),
    };
    let repair = model.repair_system_action(
        &format!("workflow-repair:{instance_id}:{}", system_action.id),
        &failure,
    )?;
    let plan: SystemActionRepairPlan = serde_json::from_str(repair.text.trim()).map_err(|error| {
        WorkflowRuntimeError::execution(format!(
            "System action {} failed with exit code {:?}, and Gemma returned invalid repair JSON: {error}. stderr: {}",
            system_action.id,
            failed.exit_code,
            compact_runtime_text(&failed.stderr.text)
        ))
    })?;
    if plan.command.trim().is_empty() {
        return Err(WorkflowRuntimeError::execution(format!(
            "System action {} failed and Gemma returned an empty repair command.",
            system_action.id
        )));
    }
    let repaired_working_directory = plan
        .working_directory
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| working_directory.to_string_lossy().to_string());
    let repaired_working_directory =
        resolve_system_working_directory(workspace_root, Some(repaired_working_directory))?;
    let classification = classify_system_action(
        system_action_class(&system_action.action_type),
        &plan.command,
        &plan.args,
    );
    if classification.requires_human_approval() {
        return Err(WorkflowRuntimeError::execution(format!(
            "System action {} failed, but the self-heal repair requires human approval: {}",
            system_action.id, classification.reason
        )));
    }
    let result = run_system_action(
        &system_action.action_type,
        &plan.command,
        &plan.args,
        workspace_root,
        &repaired_working_directory,
        bounded_system_timeout_ms(timeout_ms),
        max_output_bytes,
    )?;
    if !system_action_succeeded(&result) {
        return Err(WorkflowRuntimeError::execution(format!(
            "System action {} failed with exit code {:?}; self-heal rerun exited with {:?}. stderr: {}",
            system_action.id,
            failed.exit_code,
            result.exit_code,
            compact_runtime_text(&result.stderr.text)
        )));
    }
    Ok(RepairedSystemAction {
        result,
        command: plan.command,
        args: plan.args,
        explanation: plan.explanation,
    })
}

fn system_command(
    action_type: &SystemActionType,
    command: &str,
    args: &[String],
    native_executable: Option<&Path>,
) -> Command {
    match action_type {
        SystemActionType::Shell => {
            if cfg!(windows) {
                let mut process = Command::new("cmd");
                process.arg("/C").arg(command);
                process
            } else {
                let mut process = Command::new("/bin/bash");
                process.arg("-c").arg(command);
                process
            }
        }
        SystemActionType::Python => {
            let mut process =
                Command::new(native_executable.unwrap_or_else(|| Path::new("python3")));
            process.arg(command).args(args);
            process
        }
        SystemActionType::Binary => {
            let mut process = Command::new(command);
            process.args(args);
            process
        }
    }
}

fn spawn_limited_reader<R>(
    mut reader: R,
    max_output_bytes: usize,
) -> thread::JoinHandle<io::Result<LimitedPipeOutput>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut captured = Vec::new();
        let mut truncated = false;
        let mut buffer = [0u8; 4096];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let remaining = max_output_bytes.saturating_sub(captured.len());
            if remaining > 0 {
                captured.extend_from_slice(&buffer[..read.min(remaining)]);
            }
            if read > remaining {
                truncated = true;
            }
        }
        Ok(LimitedPipeOutput {
            text: String::from_utf8_lossy(&captured).into_owned(),
            truncated,
        })
    })
}
