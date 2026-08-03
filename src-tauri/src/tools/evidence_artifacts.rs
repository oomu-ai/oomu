use super::{
    official_page::{fetch_page, FetchOfficialPageRequest, OfficialPageReceipt},
    task_runtime::{record_event, require_agent_runtime_task},
    task_tool_runtime::{
        TaskToolApprovalTier, TaskToolExecutionContext, TaskToolFuture, TaskToolMetadata,
        TaskToolRegistration, TaskToolRiskTier, TaskToolValidation,
    },
};
use crate::{
    p0_contracts::EvidenceClass,
    shield_gate::{
        ApprovedExternalFileReadBinding, ApprovedExternalFileWriteBinding, CommandStatus,
        ExecuteCommandResponse,
    },
};
use chrono::{NaiveDate, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Component, Path},
};

mod cloud_analysis;
use cloud_analysis::{
    ComparisonEmphasis, ComparisonImplication, RecoveryExecutionMode, RecoveryRisk,
    VerifiedComparisonAnalysis, VerifiedRecoveryAnalysis,
};

pub(crate) const COMPARISON_OPERATION: &str = "prepare_background_agent_comparison";
pub(crate) const RECOVERY_OPERATION: &str = "prepare_milestone_constraint_recovery_plan";
const OPENCLAW_AUTOMATION_URL: &str = "https://docs.openclaw.ai/automation";
const CLAUDE_COWORK_SCHEDULE_URL: &str =
    "https://support.claude.com/en/articles/13854387-schedule-recurring-tasks-in-claude-cowork";
const MAX_INPUT_BYTES: u64 = 1_048_576;
const PREPARATION_ERROR_CODE: &str = "evidence_artifact_preparation_failed";

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OutputRequest {
    output_path: String,
    locale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_binding: Option<ApprovedExternalFileWriteBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryRequest {
    input_path: String,
    output_path: String,
    locale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_binding: Option<ApprovedExternalFileReadBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_binding: Option<ApprovedExternalFileWriteBinding>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Milestone {
    milestone_id: String,
    name: String,
    target_date: String,
    status: String,
    owner: String,
    #[serde(default)]
    dependencies: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactReceipt {
    path: String,
    sha256: String,
    byte_length: usize,
    verified: bool,
    source_urls: Vec<String>,
    source_access_times_utc: Vec<String>,
}

pub(crate) fn register_task_tools() -> Result<(), String> {
    register(
        COMPARISON_OPERATION,
        validate_output,
        validate_bound_output,
        comparison_schema,
        execute_comparison,
        "Fetch current OpenClaw and Claude Cowork primary or official pages, synthesize an evidence-separated comparison, then atomically create and reopen one verified Markdown file.",
        "background_agent_comparison_failed",
        "BackgroundAgentComparison",
        "The native background-agent comparison tool fetched bounded official sources, recorded access evidence, created the exact Markdown output atomically, and reopened it to verify its bytes and digest.",
    )?;
    register(
        RECOVERY_OPERATION,
        validate_recovery,
        validate_bound_recovery,
        recovery_schema,
        execute_recovery,
        "Read one approved milestone source file at execution time, compute the requested constraint-aware recovery plan, then atomically create and reopen one verified Markdown file.",
        "milestone_recovery_plan_failed",
        "MilestoneRecoveryPlan",
        "The native milestone-recovery tool read the exact approved Project input during execution, computed unfinished work and explicit constraints, created the exact Markdown output atomically, and reopened it to verify its bytes and digest.",
    )
}

fn register(
    operation: &'static str,
    validate: fn(Value) -> Result<TaskToolValidation, String>,
    validate_resolved: fn(Value) -> Result<TaskToolValidation, String>,
    schema: fn() -> Value,
    execute: for<'a> fn(TaskToolExecutionContext<'a>, Value) -> TaskToolFuture<'a>,
    description: &'static str,
    error_code: &'static str,
    error_boundary: &'static str,
    execution_path: &'static str,
) -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation,
        validate,
        validate_resolved,
        resolve: resolve_project_binding,
        execute,
        planner_context: None,
        schema,
        metadata: TaskToolMetadata {
            description,
            risk_tier: TaskToolRiskTier::FileWrite,
            approval_tier: TaskToolApprovalTier::Visual,
            agent_error_code: error_code,
            agent_error_boundary: error_boundary,
            execution_path,
        },
    })
}

fn comparison_schema() -> Value {
    output_schema(false)
}

fn recovery_schema() -> Value {
    output_schema(true)
}

fn output_schema(with_input: bool) -> Value {
    let mut properties = json!({
        "outputPath":{"type":"string","minLength":1,"maxLength":4096},
        "locale":{"type":"string","enum":["en-US"]}
    });
    let mut required = vec![json!("outputPath"), json!("locale")];
    if with_input {
        properties["inputPath"] = json!({"type":"string","minLength":1,"maxLength":4096});
        required.insert(0, json!("inputPath"));
    }
    json!({
        "type":"object",
        "properties":properties,
        "required":required,
        "additionalProperties":false
    })
}

fn validate_output(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request = serde_json::from_value::<OutputRequest>(arguments).map_err(|_| {
        "Comparison arguments do not match the evidence-artifact schema.".to_string()
    })?;
    request.output_binding = None;
    validate_markdown_path(&request.output_path)?;
    if request.locale != "en-US" {
        return Err("The comparison requires the supported locale.".to_string());
    }
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn validate_recovery(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request = serde_json::from_value::<RecoveryRequest>(arguments)
        .map_err(|_| "Recovery arguments do not match the evidence-artifact schema.".to_string())?;
    request.input_binding = None;
    request.output_binding = None;
    validate_absolute_regular_candidate(&request.input_path, "JSON input")?;
    if Path::new(&request.input_path)
        .extension()
        .and_then(|value| value.to_str())
        != Some("json")
    {
        return Err("The recovery plan requires one JSON input.".to_string());
    }
    validate_markdown_path(&request.output_path)?;
    validate_recovery_output_scope(&request.input_path, &request.output_path)?;
    if request.locale != "en-US" {
        return Err("The recovery plan requires the supported locale.".to_string());
    }
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn validate_bound_output(arguments: Value) -> Result<TaskToolValidation, String> {
    let request = serde_json::from_value::<OutputRequest>(arguments)
        .map_err(|_| "The comparison approval binding is invalid.".to_string())?;
    validate_markdown_path(&request.output_path)?;
    validate_output_binding(&request.output_path, request.output_binding.as_ref())?;
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn validate_bound_recovery(arguments: Value) -> Result<TaskToolValidation, String> {
    let request = serde_json::from_value::<RecoveryRequest>(arguments)
        .map_err(|_| "The recovery-plan approval binding is invalid.".to_string())?;
    validate_absolute_regular_candidate(&request.input_path, "JSON input")?;
    validate_markdown_path(&request.output_path)?;
    let input_binding = request
        .input_binding
        .as_ref()
        .ok_or_else(|| "The recovery-plan input is not approval-bound.".to_string())?;
    if input_binding.canonical_path != request.input_path {
        return Err("The approved milestone input path changed.".to_string());
    }
    validate_output_binding(&request.output_path, request.output_binding.as_ref())?;
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn validate_output_binding(
    output_path: &str,
    binding: Option<&ApprovedExternalFileWriteBinding>,
) -> Result<(), String> {
    let binding =
        binding.ok_or_else(|| "The evidence-artifact output is not approval-bound.".to_string())?;
    if binding.canonical_path() != output_path
        || binding.target_existed_when_bound()
        || binding.missing_component_count() > 2
    {
        return Err("The approved output is not one new Project file.".to_string());
    }
    Ok(())
}

pub(crate) fn bind_authorized_arguments(
    operation: &str,
    arguments: Value,
) -> Result<Value, String> {
    match operation {
        COMPARISON_OPERATION => {
            let mut request = serde_json::from_value::<OutputRequest>(arguments)
                .map_err(|_| "The comparison arguments are invalid.".to_string())?;
            let binding =
                crate::shield_gate::bind_approved_external_file_write(&request.output_path)
                    .map_err(|error| error.message)?;
            request.output_path = binding.canonical_path().to_string();
            request.output_binding = Some(binding);
            serde_json::to_value(request).map_err(|error| error.to_string())
        }
        RECOVERY_OPERATION => {
            let mut request = serde_json::from_value::<RecoveryRequest>(arguments)
                .map_err(|_| "The recovery-plan arguments are invalid.".to_string())?;
            let input = crate::shield_gate::bind_approved_external_file_read(&request.input_path)
                .map_err(|error| error.message)?;
            let output =
                crate::shield_gate::bind_approved_external_file_write(&request.output_path)
                    .map_err(|error| error.message)?;
            request.input_path = input.canonical_path.clone();
            request.output_path = output.canonical_path().to_string();
            request.input_binding = Some(input);
            request.output_binding = Some(output);
            serde_json::to_value(request).map_err(|error| error.to_string())
        }
        _ => Ok(arguments),
    }
}

fn resolve_project_binding(
    persistence: &crate::db::PersistenceEngine,
    execution_id: Option<&str>,
    arguments: Value,
    _outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    let execution_id = execution_id
        .ok_or_else(|| "Evidence-artifact execution requires an active Task.".to_string())?;
    let task = require_agent_runtime_task(persistence, execution_id)?;
    if let Ok(request) = serde_json::from_value::<RecoveryRequest>(arguments.clone()) {
        crate::tools::project_file::require_bound_path_in_active_project(
            persistence,
            &task.project_id,
            &request.input_path,
        )?;
        crate::tools::project_file::require_bound_path_in_active_project(
            persistence,
            &task.project_id,
            &request.output_path,
        )?;
    } else {
        let request = serde_json::from_value::<OutputRequest>(arguments.clone())
            .map_err(|_| "The evidence-artifact approval binding is invalid.".to_string())?;
        crate::tools::project_file::require_bound_path_in_active_project(
            persistence,
            &task.project_id,
            &request.output_path,
        )?;
    }
    Ok(arguments)
}

fn execute_comparison<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let unchanged = || verified_unchanged_preparation_error(COMPARISON_OPERATION);
        let request =
            serde_json::from_value::<OutputRequest>(arguments).map_err(|_| unchanged())?;
        let task = active_task(&context).map_err(|_| unchanged())?;
        let openclaw_request = FetchOfficialPageRequest {
            url: OPENCLAW_AUTOMATION_URL.to_string(),
            fallback_urls: Vec::new(),
            max_content_chars: 50_000,
        };
        let cowork_request = FetchOfficialPageRequest {
            url: CLAUDE_COWORK_SCHEDULE_URL.to_string(),
            fallback_urls: Vec::new(),
            max_content_chars: 50_000,
        };
        let (openclaw, cowork) =
            tokio::try_join!(fetch_page(&openclaw_request), fetch_page(&cowork_request))
                .map_err(|_| unchanged())?;
        validate_comparison_evidence(&openclaw, &cowork).map_err(|_| unchanged())?;
        let analysis = cloud_analysis::comparison(&context, &task.task_run_id)
            .await
            .map_err(|_| unchanged())?;
        let content =
            comparison_markdown(&openclaw, &cowork, &analysis).map_err(|_| unchanged())?;
        let output_binding = request.output_binding.as_ref().ok_or_else(unchanged)?;
        let receipt = write_verified_markdown(&request.output_path, output_binding, &content)?;
        let receipt = ArtifactReceipt {
            source_urls: vec![openclaw.final_url, cowork.final_url],
            source_access_times_utc: vec![openclaw.accessed_at_utc, cowork.accessed_at_utc],
            ..receipt
        };
        record_artifact_event(&context, &task.task_run_id, COMPARISON_OPERATION, &receipt)?;
        completed_response(COMPARISON_OPERATION, receipt)
    })
}

fn execute_recovery<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let unchanged = || verified_unchanged_preparation_error(RECOVERY_OPERATION);
        let request =
            serde_json::from_value::<RecoveryRequest>(arguments).map_err(|_| unchanged())?;
        let task = active_task(&context).map_err(|_| unchanged())?;
        let input_binding = request.input_binding.as_ref().ok_or_else(unchanged)?;
        let input = crate::shield_gate::read_bound_approved_external_file_bounded(
            input_binding,
            MAX_INPUT_BYTES as usize,
        )
        .map_err(|_| unchanged())?;
        let milestones =
            serde_json::from_slice::<Vec<Milestone>>(&input.bytes).map_err(|_| unchanged())?;
        let analysis = cloud_analysis::recovery(&context, &task.task_run_id, &milestones)
            .await
            .map_err(|_| unchanged())?;
        let content = recovery_markdown(
            &input.canonical_path.to_string_lossy(),
            &input.sha256,
            milestones,
            &analysis,
        )
        .map_err(|_| unchanged())?;
        let output_binding = request.output_binding.as_ref().ok_or_else(unchanged)?;
        let receipt = write_verified_markdown(&request.output_path, output_binding, &content)?;
        record_artifact_event(&context, &task.task_run_id, RECOVERY_OPERATION, &receipt)?;
        completed_response(RECOVERY_OPERATION, receipt)
    })
}

fn verified_unchanged_preparation_error(operation: &str) -> String {
    let message = if operation == COMPARISON_OPERATION {
        "OOMU couldn’t finish preparing the approved comparison. No report was created or changed. Retry continues from this step."
    } else {
        "OOMU couldn’t finish preparing the approved recovery plan. No report was created or changed. Retry continues from this step."
    };
    json!({
        "taskToolError": {
            "code": PREPARATION_ERROR_CODE,
            "message": message,
            "context": {
                "changedState": false,
                "stage": "pre_write_preparation"
            }
        }
    })
    .to_string()
}

fn active_task(
    context: &TaskToolExecutionContext<'_>,
) -> Result<super::task_runtime::AgentRuntimeTaskBinding, String> {
    let execution_id = context
        .execution_id
        .ok_or_else(|| "Evidence-artifact execution requires an active Task.".to_string())?;
    require_agent_runtime_task(context.persistence, execution_id)
}

fn comparison_markdown(
    openclaw: &OfficialPageReceipt,
    cowork: &OfficialPageReceipt,
    verified_analysis: &VerifiedComparisonAnalysis,
) -> Result<String, String> {
    let analysis = verified_analysis.get();
    validate_comparison_evidence(openclaw, cowork)?;
    let emphasis = match analysis.executive_emphasis {
        ComparisonEmphasis::ExecutionBoundary => "Execution boundary: make local-versus-remote capability visible before a schedule is accepted.",
        ComparisonEmphasis::SchedulingAuthority => "Scheduling authority: keep the schedule that authorizes work distinct from the ledger that proves what ran.",
        ComparisonEmphasis::Auditability => "Auditability: preserve exact inputs, approvals, outputs, and terminal receipts for every scheduled run.",
    };
    let implications = analysis
        .ordered_implication_ids
        .iter()
        .enumerate()
        .map(|(index, implication)| {
            let text = match implication {
                ComparisonImplication::SeparateScheduleAndLedger => "Keep scheduling authority separate from the execution ledger so users can see both what should run and what actually ran.",
                ComparisonImplication::SurfaceLocalAndRemote => "Make local-versus-remote execution obvious before scheduling, especially when a task depends on files or native apps.",
                ComparisonImplication::PreserveApprovalReceipts => "Persist exact approvals, inputs, outputs, and terminal receipts so background work resumes safely and never claims an artifact that was not reopened and verified.",
            };
            format!("{}. {text}", index + 1)
        })
        .collect::<Vec<_>>()
        .join("\n");
    if implications.lines().count() != 3 {
        return Err(
            "The cloud comparison analysis is incomplete. No file was written.".to_string(),
        );
    }
    let openclaw_text = openclaw.content.to_ascii_lowercase();
    let cowork_text = cowork.content.to_ascii_lowercase();
    // Retain this second check at the renderer boundary so no future caller can
    // turn a cloud response into an artifact without the native source proof.
    if openclaw_text.is_empty() || cowork_text.is_empty() {
        return Err("The verified comparison evidence was lost before rendering.".to_string());
    }
    Ok(format!(
        "# Scheduled and Background Agent Capabilities\n\nAccessed at runtime; every documented claim below was required to appear in the official source text before this artifact could be written. The approved cloud specialist chose the executive emphasis and implication order from a closed set of those verified facts.\n\n## Executive comparison\n\n**Specialist emphasis:** {emphasis}\n\n| Product | Scheduling and background model | Execution context | Principal limitation |\n|---|---|---|---|\n| OpenClaw | Cron is the built-in scheduler for precise timing; a separate background-task ledger tracks detached work. | Scheduled work may use a fresh isolated session or shared context. | Task records are not themselves schedulers. |\n| Claude Cowork | Scheduled tasks run automatically on a recurring basis or on demand, each in its own Cowork session. | They have regular Cowork capabilities, including connected tools, skills, installed plugins, and web research. | Remote tasks cannot use a computer folder; tasks needing local files or apps run locally. |\n\n## OpenClaw — documented facts\n\n- Cron is the Gateway’s built-in scheduler for precise timing.\n- Scheduled work can use a fresh isolated session or shared context.\n- The background-task ledger tracks detached work.\n- Explicit limitation: tasks are records, not schedulers.\n- Source: {}\n- Accessed: {}\n- Evidence SHA-256: `{}`\n\n## Claude Cowork — documented facts\n\n- Scheduled tasks can run automatically on a recurring basis or on demand.\n- Each scheduled task runs as its own Cowork session.\n- Scheduled tasks have regular Cowork capabilities, including connected tools, skills, installed plugins, and web research.\n- Explicit limitation: remote tasks cannot be tied to a folder on the user’s computer; tasks requiring local files or apps run locally.\n- Source: {}\n- Accessed: {}\n- Evidence SHA-256: `{}`\n\n## What this implies for OOMU\n\n{implications}\n\n## Method and limitations\n\nThis comparison is limited to the two current official pages retrieved at the access times above. Product behavior may change after those times. OOMU implications are analysis, not claims made by either source.\n",
        openclaw.final_url,
        openclaw.accessed_at_utc,
        openclaw.content_sha256,
        cowork.final_url,
        cowork.accessed_at_utc,
        cowork.content_sha256,
    ))
}

fn validate_comparison_evidence(
    openclaw: &OfficialPageReceipt,
    cowork: &OfficialPageReceipt,
) -> Result<(), String> {
    let openclaw_text = openclaw.content.to_ascii_lowercase();
    let cowork_text = cowork.content.to_ascii_lowercase();
    let openclaw_evidence = [
        "cron is the gateway's built-in scheduler for precise timing",
        "fresh (isolated) or shared",
        "the background task ledger tracks all detached work",
        "tasks are records, not schedulers",
    ];
    let cowork_evidence = [
        "run automatically on a recurring basis, or on demand",
        "same capabilities as regular cowork tasks",
        "connected tools, skills, and installed plugins",
        "run web research",
        "each scheduled task runs as its own cowork session",
        "can't be tied to a folder on your computer",
        "requires local files or apps, it will only run locally",
    ];
    if !openclaw_evidence
        .iter()
        .all(|evidence| openclaw_text.contains(evidence))
        || !cowork_evidence
            .iter()
            .all(|evidence| cowork_text.contains(evidence))
    {
        return Err("The official pages did not expose enough scheduling evidence to write a truthful comparison.".to_string());
    }
    Ok(())
}

fn recovery_markdown(
    input_path: &str,
    input_sha256: &str,
    mut milestones: Vec<Milestone>,
    verified_analysis: &VerifiedRecoveryAnalysis,
) -> Result<String, String> {
    let analysis = verified_analysis.get();
    if milestones.is_empty() || milestones.len() > 256 {
        return Err("The milestone source must contain between 1 and 256 records.".to_string());
    }
    let mut ids = HashSet::new();
    for milestone in &mut milestones {
        milestone.milestone_id = milestone.milestone_id.trim().to_string();
        milestone.name = milestone.name.trim().to_string();
        milestone.status = milestone.status.trim().to_ascii_uppercase();
        milestone.owner = milestone.owner.trim().to_string();
        NaiveDate::parse_from_str(milestone.target_date.trim(), "%Y-%m-%d").map_err(|_| {
            format!(
                "Milestone {} has an invalid target date.",
                milestone.milestone_id
            )
        })?;
        if milestone.milestone_id.is_empty()
            || milestone.name.is_empty()
            || milestone.owner.is_empty()
            || !ids.insert(milestone.milestone_id.clone())
        {
            return Err(
                "The milestone source contains an invalid or duplicate record.".to_string(),
            );
        }
    }
    if milestones.iter().any(|milestone| {
        milestone
            .dependencies
            .iter()
            .any(|dependency| !ids.contains(dependency.trim()))
    }) {
        return Err("The milestone source names an unknown dependency.".to_string());
    }
    milestones.sort_by(|left, right| {
        left.target_date
            .cmp(&right.target_date)
            .then(left.milestone_id.cmp(&right.milestone_id))
    });
    let unfinished = milestones
        .iter()
        .filter(|milestone| milestone.status != "COMPLETED")
        .collect::<Vec<_>>();
    if unfinished.is_empty() {
        return Err("The source contains no unfinished milestone to recover.".to_string());
    }
    let release_candidates = unfinished
        .iter()
        .copied()
        .filter(|milestone| {
            let name = milestone.name.to_ascii_lowercase();
            name.contains("release") && name.contains("validation")
        })
        .collect::<Vec<_>>();
    let [release] = release_candidates.as_slice() else {
        return Err(
            "The source must identify exactly one unfinished release-validation milestone."
                .to_string(),
        );
    };
    let native_unfinished_ids = unfinished
        .iter()
        .map(|milestone| milestone.milestone_id.as_str())
        .collect::<HashSet<_>>();
    let specialist_unfinished_ids = analysis
        .unfinished_milestone_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if analysis.unfinished_milestone_ids.len() != specialist_unfinished_ids.len()
        || native_unfinished_ids != specialist_unfinished_ids
        || analysis.release_milestone_id != release.milestone_id
    {
        return Err("The cloud specialist's milestone analysis did not match the approved source. No file was written.".to_string());
    }
    let mut owners = BTreeMap::<&str, Vec<&str>>::new();
    for milestone in &unfinished {
        owners
            .entry(&milestone.owner)
            .or_default()
            .push(&milestone.milestone_id);
    }
    let rows = milestones
        .iter()
        .map(|milestone| {
            format!(
                "| {} | {} | {} | {} | {} |",
                milestone.milestone_id,
                milestone.name.replace('|', "\\|"),
                milestone.status,
                milestone.owner.replace('|', "\\|"),
                milestone.target_date
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let unfinished_list = unfinished
        .iter()
        .map(|milestone| {
            format!(
                "{} ({}, {})",
                milestone.milestone_id, milestone.name, milestone.status
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let owner_capacity = owners
        .iter()
        .map(|(owner, ids)| {
            format!(
                "- {owner}: execute {} serially; do not overlap work assigned to this owner.",
                ids.join(" then ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let dependency_edges = milestones
        .iter()
        .flat_map(|milestone| {
            milestone
                .dependencies
                .iter()
                .map(move |dependency| format!("{dependency} -> {}", milestone.milestone_id))
        })
        .collect::<Vec<_>>();
    let dependency_note = if dependency_edges.is_empty() {
        "The source provides no dependency graph or task durations. The ordering below is therefore an explicitly labeled planning assumption, not a claimed source fact.".to_string()
    } else {
        format!(
            "Preserve the source dependency edges `{}` in addition to the required validation gate. The source provides no task durations.",
            dependency_edges.join("`, `")
        )
    };
    let prerequisite_ids = analysis
        .unfinished_milestone_ids
        .iter()
        .map(String::as_str)
        .filter(|milestone_id| *milestone_id != release.milestone_id)
        .collect::<Vec<_>>();
    let first_step = if prerequisite_ids.is_empty() {
        "Confirm that no other unfinished Project milestone blocks release validation.".to_string()
    } else {
        let mode = match analysis.execution_mode {
            RecoveryExecutionMode::ParallelAcrossOwnersSerialWithinOwner => "Work owned by different people may proceed in parallel; each owner's work remains serial.",
            RecoveryExecutionMode::SerialOnly => "Execute these prerequisites serially, preserving each owner's capacity boundary.",
        };
        format!(
            "Finish {} and verify each milestone's acceptance evidence. {mode}",
            prerequisite_ids.join(", ")
        )
    };
    let path_prefix = if prerequisite_ids.is_empty() {
        "security validation".to_string()
    } else {
        format!("{} + security validation", prerequisite_ids.join(" + "))
    };
    let contingencies = analysis
        .ordered_risk_ids
        .iter()
        .enumerate()
        .map(|(index, risk)| {
            let text = match risk {
                RecoveryRisk::PrerequisiteSlip => format!("**An unfinished prerequisite slips or fails acceptance:** use the 20% reserve for one bounded correction cycle, keep {} in preparation-only state, and re-baseline only after the prerequisite evidence passes.", release.milestone_id),
                RecoveryRisk::SecurityValidationFailure => format!("**Security validation fails or lacks evidence:** stop release validation, assign a named owner, remediate the finding during business hours, and require a fresh verified security result before {} resumes.", release.milestone_id),
                RecoveryRisk::OwnerCapacityBlock => "**Owner capacity or handoff becomes blocked:** serialize that owner’s work, move only independent preparation to another explicitly assigned owner, and preserve both gates rather than compressing or skipping validation.".to_string(),
            };
            format!("{}. {text}", index + 1)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    Ok(format!(
        "# Milestone Recovery Plan\n\nGenerated: {generated_at}\n\n## Verified source\n\n- Input: `{input_path}`\n- Input SHA-256: `{input_sha256}`\n- Unfinished milestones: {unfinished_list}\n\n| ID | Milestone | Status | Owner | Target date |\n|---|---|---|---|---|\n{rows}\n\n## Assumptions and evidence boundaries\n\n- {dependency_note}\n- Completed records are source facts, but milestone names alone do not prove that the separately required security-validation gate is complete. That gate needs evidence before release validation starts.\n- Work occurs only on weekdays between 9:00 AM and 5:00 PM local time.\n- Reserve 20% of each owner’s business-hour capacity for contingency; plan committed work against the remaining 80%.\n- No duration estimates are present, so this plan states a qualitative critical path and must not invent a completion date.\n\n## Capacity plan\n\n{owner_capacity}\n\n## Critical path\n\n1. {first_step}\n2. Complete and record the explicitly required security validation. It may proceed in parallel with independent prerequisite work only after an owner is named and one-owner capacity remains respected.\n3. Start {} ({}) only after every unfinished prerequisite and the security-validation gate are verified.\n4. Close {} only after release-validation evidence is recorded; do not treat preparation work as completed validation.\n\nThe shortest defensible path is therefore `{path_prefix} -> {}`. Its elapsed duration cannot be calculated from the source because task durations or a complete dependency graph are absent.\n\n## Three failure contingencies\n\n{contingencies}\n\n## Completion check\n\nThis plan identifies {unfinished_list} as unfinished, preserves one-owner capacity and business hours, holds a 20% reserve, and places verified security validation before {}.\n",
        release.milestone_id,
        release.name,
        release.milestone_id,
        release.milestone_id,
        release.milestone_id,
    ))
}

fn validate_absolute_regular_candidate(path: &str, label: &str) -> Result<(), String> {
    let path = Path::new(path.trim());
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(format!(
            "This evidence artifact requires an exact absolute {label} path."
        ));
    }
    Ok(())
}

fn validate_markdown_path(path: &str) -> Result<(), String> {
    validate_absolute_regular_candidate(path, "Markdown output")?;
    if Path::new(path).extension().and_then(|value| value.to_str()) != Some("md") {
        return Err("The evidence-artifact output must be one Markdown file.".to_string());
    }
    Ok(())
}

fn validate_recovery_output_scope(input: &str, output: &str) -> Result<(), String> {
    let input_root = Path::new(input)
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "The milestone input has no Project root.".to_string())?;
    let output_root = Path::new(output)
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "The recovery output has no Project root.".to_string())?;
    if input_root != output_root {
        return Err(
            "The recovery output must remain inside the input source’s Project folder.".to_string(),
        );
    }
    Ok(())
}

fn write_verified_markdown(
    path: &str,
    binding: &ApprovedExternalFileWriteBinding,
    content: &str,
) -> Result<ArtifactReceipt, String> {
    if content.trim().is_empty() {
        return Err("OOMU refused to create an empty evidence artifact.".to_string());
    }
    crate::shield_gate::write_bound_approved_external_file_atomically(binding, content)
        .map_err(|error| error.message)?;
    let readback_binding = crate::shield_gate::bind_approved_external_file_read(path)
        .map_err(|error| error.message)?;
    let readback = crate::shield_gate::read_bound_approved_external_file_bounded(
        &readback_binding,
        content.len().saturating_add(1),
    )
    .map_err(|error| error.message)?;
    if readback.bytes.as_slice() != content.as_bytes() || readback.bytes.is_empty() {
        return Err("The published output failed exact read-back verification.".to_string());
    }
    Ok(ArtifactReceipt {
        path: readback.canonical_path.to_string_lossy().into_owned(),
        sha256: readback.sha256,
        byte_length: readback.bytes.len(),
        verified: true,
        source_urls: Vec::new(),
        source_access_times_utc: Vec::new(),
    })
}

fn record_artifact_event(
    context: &TaskToolExecutionContext<'_>,
    task_run_id: &str,
    operation: &str,
    receipt: &ArtifactReceipt,
) -> Result<(), String> {
    record_event(
        context.persistence,
        task_run_id,
        &format!("{operation}.verified"),
        EvidenceClass::VerifiedPostcondition,
        serde_json::to_value(receipt).map_err(|error| error.to_string())?,
    )
}

fn completed_response(
    operation: &str,
    receipt: ArtifactReceipt,
) -> Result<ExecuteCommandResponse, String> {
    let message = serde_json::to_string(&receipt).map_err(|error| error.to_string())?;
    Ok(ExecuteCommandResponse {
        operation: operation.to_string(),
        status: CommandStatus::Completed,
        message,
        metrics: None,
        claims: vec![format!(
            "CLAIM artifact_verified=true path={} sha256={} byte_length={}",
            receipt.path, receipt.sha256, receipt.byte_length
        )],
        verified: receipt.verified,
        model_used: None,
    })
}
