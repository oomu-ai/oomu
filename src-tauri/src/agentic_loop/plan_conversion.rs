//! Deterministic conversions between planner/workflow DTOs and executable steps.
//! This leaf owns no orchestration, persistence, approval, or recovery behavior.
use super::{
    ActionPlan, AgenticLoopError, ContextBundle, ModelRouteDecision, RiskLevel, Step, Tool,
    WorkflowAction, WorkflowActionKind,
};
use crate::{
    foundation::clock::unix_time_ms_u128 as unix_time_ms,
    gemma::{
        GeneratedActionPlanDraft, GeneratedPlanStepDraft, GeneratedRiskLevel, GeneratedToolDraft,
        IntentCategory, StructuredIntent,
    },
    shield_gate::{LogicalCertificate, RequestedAction, TrustPolicy},
};

pub(super) fn workflow_action_to_step(
    index: usize,
    action: WorkflowAction,
) -> Result<Step, AgenticLoopError> {
    let position = index + 1;
    let dependency_note = if action.dependencies.is_empty() {
        "no visual dependencies".to_string()
    } else {
        format!("depends on {}", action.dependencies.join(", "))
    };
    let step_label = format!("{position}. {} ({dependency_note})", action.label);
    let action_id = action.id.clone();
    let action_label = action.label.clone();
    let missing = |field: &'static str| {
        AgenticLoopError {
        code: "workflow_action_input_missing",
        boundary: "WorkflowExecution",
        message: format!(
            "Workflow action '{}' ({}) requires an explicit {field}; no default value was substituted.",
            action_id, action_label
        ),
        mlc_path: None,
    }
    };

    let step = match action.kind {
        WorkflowActionKind::SystemMetric => Step {
            step: step_label,
            tool: Tool::SystemDiagnostics {
                principal: action.label,
            },
            risk_level: RiskLevel::Low,
        },
        WorkflowActionKind::FileWrite => Step {
            step: step_label,
            tool: Tool::FileWrite {
                path: action
                    .path
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| missing("path"))?,
                content: action.content.ok_or_else(|| missing("content"))?,
            },
            risk_level: RiskLevel::High,
        },
        WorkflowActionKind::FileRead => Step {
            step: step_label,
            tool: Tool::FileRead {
                path: action
                    .path
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| missing("path"))?,
            },
            risk_level: RiskLevel::Medium,
        },
        WorkflowActionKind::FileList => Step {
            step: step_label,
            tool: Tool::FileList {
                path: action
                    .path
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| missing("path"))?,
            },
            risk_level: RiskLevel::Low,
        },
        WorkflowActionKind::SystemAudit => Step {
            step: step_label,
            tool: Tool::SystemAudit {
                scope: action
                    .scope
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| missing("scope"))?,
            },
            risk_level: RiskLevel::Low,
        },
        WorkflowActionKind::LocalInference => {
            return Err(AgenticLoopError {
                code: "workflow_action_unsupported",
                boundary: "WorkflowExecution",
                message: format!(
                    "Workflow action '{}' requests local_inference, but no production visual-workflow executor is registered for it.",
                    action_id
                ),
                mlc_path: None,
            });
        }
    };
    Ok(step)
}

pub(super) fn generated_draft_to_plan(
    objective: String,
    draft: GeneratedActionPlanDraft,
    route: ModelRouteDecision,
    context: ContextBundle,
) -> ActionPlan {
    let id = format!("plan-{}", unix_time_ms());
    let category = if draft.steps.iter().any(|step| {
        matches!(
            step.tool,
            GeneratedToolDraft::FileList { .. }
                | GeneratedToolDraft::FileRead { .. }
                | GeneratedToolDraft::FileWrite { .. }
                | GeneratedToolDraft::DeleteFile { .. }
                | GeneratedToolDraft::CodebasePatch { .. }
                | GeneratedToolDraft::CodebaseCompile { .. }
                | GeneratedToolDraft::TerminalExecute { .. }
                | GeneratedToolDraft::SystemAudit { .. }
                | GeneratedToolDraft::TelemetryArchive { .. }
                | GeneratedToolDraft::DocumentIndex { .. }
                | GeneratedToolDraft::AskLocalDocumentIndex { .. }
                | GeneratedToolDraft::RegisteredTaskTool { .. }
        )
    }) {
        IntentCategory::ProjectAnalysis
    } else if draft.steps.iter().any(|step| {
        matches!(
            step.tool,
            GeneratedToolDraft::WebFetch { .. }
                | GeneratedToolDraft::SovereignDuckDuckGoSearch { .. }
        )
    }) {
        IntentCategory::Research
    } else if draft
        .steps
        .iter()
        .any(|step| matches!(step.tool, GeneratedToolDraft::SystemDiagnostics { .. }))
    {
        IntentCategory::SystemDiagnostics
    } else {
        IntentCategory::Unsupported
    };
    let intent = StructuredIntent {
        objective: objective.clone(),
        category,
        source: draft.source,
        degraded_reason: draft.degraded_reason.clone(),
    };
    let steps = draft
        .steps
        .into_iter()
        .map(generated_step_to_step)
        .collect::<Vec<_>>();
    let mut plan = ActionPlan {
        id,
        objective,
        intent,
        steps,
        exit_condition: draft.exit_condition,
        logical_certificate: LogicalCertificate::unsigned(Vec::new(), Vec::new(), String::new()),
        trusted_automatic_execution: false,
        model_route: route,
        parent_artifact_hashes: context.inherited_artifact_hashes,
    };
    plan.trusted_automatic_execution = TrustPolicy::allows_low_risk_plan(
        plan.steps
            .iter()
            .map(|step| step.risk_level.trust_policy_label()),
    );
    plan
}

pub(crate) fn generated_step_to_step(step: GeneratedPlanStepDraft) -> Step {
    let generated_risk_level = match step.risk_level {
        GeneratedRiskLevel::Low => RiskLevel::Low,
        GeneratedRiskLevel::Medium => RiskLevel::Medium,
        GeneratedRiskLevel::High => RiskLevel::High,
    };
    let tool = generated_tool_to_tool(step.tool);
    let risk_level = effective_generated_risk(&tool, generated_risk_level);
    Step {
        step: step.step,
        tool,
        risk_level,
    }
}

fn generated_tool_to_tool(tool: GeneratedToolDraft) -> Tool {
    match tool {
        GeneratedToolDraft::SystemDiagnostics { principal } => {
            Tool::SystemDiagnostics { principal }
        }
        GeneratedToolDraft::FileRead { path } => Tool::FileRead { path },
        GeneratedToolDraft::FileWrite { path, content } => Tool::FileWrite { path, content },
        GeneratedToolDraft::DeleteFile { path } => Tool::DeleteFile { path },
        GeneratedToolDraft::CodebasePatch {
            target_file_path,
            search_pattern,
            replacement_content,
        } => Tool::CodebasePatch {
            target_file_path,
            search_pattern,
            replacement_content,
        },
        GeneratedToolDraft::CodebaseCompile { target } => Tool::CodebaseCompile { target },
        GeneratedToolDraft::TerminalExecute {
            executable,
            args,
            env,
            cwd,
            timeout,
        } => Tool::TerminalExecute {
            executable,
            args,
            env,
            cwd,
            timeout,
        },
        GeneratedToolDraft::FileList { path } => Tool::FileList { path },
        GeneratedToolDraft::SystemAudit { scope } => Tool::SystemAudit { scope },
        GeneratedToolDraft::TelemetryArchive { output_path } => {
            Tool::TelemetryArchive { output_path }
        }
        GeneratedToolDraft::WebFetch { .. } => Tool::Unsupported {
            requested: "web_fetch".to_string(),
        },
        GeneratedToolDraft::DocumentIndex { .. } => Tool::Unsupported {
            requested: "document_index".to_string(),
        },
        GeneratedToolDraft::AskLocalDocumentIndex { .. } => Tool::Unsupported {
            requested: "ask_local_document_index".to_string(),
        },
        GeneratedToolDraft::SovereignDuckDuckGoSearch { query, max_results } => {
            Tool::SovereignDuckDuckGoSearch { query, max_results }
        }
        GeneratedToolDraft::RegisteredTaskTool {
            operation,
            arguments,
        } => Tool::RegisteredTaskTool(
            crate::tools::task_tool_runtime::PlannedTaskToolRequest::new(operation, arguments),
        ),
        GeneratedToolDraft::Unsupported { requested } => Tool::Unsupported { requested },
    }
}

fn effective_generated_risk(tool: &Tool, generated_risk_level: RiskLevel) -> RiskLevel {
    let terminal_requires_approval = match tool {
        Tool::TerminalExecute {
            executable,
            args,
            env,
            cwd,
            timeout,
        } => crate::tools::terminal_contract::NativeTerminalRequest {
            executable: executable.clone(),
            args: args.clone(),
            env: env.clone(),
            cwd: cwd.clone(),
            timeout: *timeout,
        }
        .classification()
        .requires_human_approval(),
        _ => false,
    };
    if terminal_requires_approval
        || matches!(tool, Tool::RegisteredTaskTool(request) if request.potentially_effectful())
        || matches!(
            tool,
            Tool::FileWrite { .. }
                | Tool::DeleteFile { .. }
                | Tool::CodebasePatch { .. }
                | Tool::CodebaseCompile { .. }
                | Tool::TelemetryArchive { .. }
        )
    {
        RiskLevel::High
    } else {
        generated_risk_level
    }
}

pub(crate) fn step_to_request(step: &Step) -> RequestedAction {
    match &step.tool {
        Tool::SystemDiagnostics { principal } => RequestedAction {
            kind: "get_system_metrics".to_string(),
            principal: Some(principal.clone()),
            path: None,
            content: None,
        },
        Tool::FileRead { path } => RequestedAction {
            kind: "file_read".to_string(),
            principal: None,
            path: Some(path.clone()),
            content: None,
        },
        Tool::FileWrite { path, content } => RequestedAction {
            kind: "file_write".to_string(),
            principal: None,
            path: Some(path.clone()),
            content: Some(content.clone()),
        },
        Tool::DeleteFile { path } => RequestedAction {
            kind: "delete_file".to_string(),
            principal: None,
            path: Some(path.clone()),
            content: None,
        },
        Tool::CodebasePatch {
            target_file_path,
            search_pattern,
            replacement_content,
        } => RequestedAction {
            kind: "codebase_patch".to_string(),
            principal: Some(search_pattern.clone()),
            path: Some(target_file_path.clone()),
            content: Some(replacement_content.clone()),
        },
        Tool::CodebaseCompile { target } => RequestedAction {
            kind: "codebase_compile".to_string(),
            principal: Some(target.clone()),
            path: None,
            content: None,
        },
        Tool::TerminalExecute {
            executable,
            args,
            env,
            cwd,
            timeout,
        } => RequestedAction {
            kind: "terminal_execute".to_string(),
            principal: None,
            path: None,
            content: Some(
                serde_json::json!({
                    "executable": executable,
                    "args": args,
                    "env": env,
                    "cwd": cwd,
                    "timeout": timeout,
                })
                .to_string(),
            ),
        },
        Tool::FileList { path } => RequestedAction {
            kind: "file_list".to_string(),
            principal: None,
            path: Some(path.clone()),
            content: None,
        },
        Tool::SystemAudit { scope } => RequestedAction {
            kind: "system_audit".to_string(),
            principal: Some(scope.clone()),
            path: None,
            content: None,
        },
        Tool::TelemetryArchive { output_path } => RequestedAction {
            kind: "telemetry_archive".to_string(),
            principal: None,
            path: Some(output_path.clone()),
            content: None,
        },
        Tool::WebFetch {
            url,
            extraction_hint,
        } => RequestedAction {
            kind: "web_fetch".to_string(),
            principal: extraction_hint.clone(),
            path: Some(url.clone()),
            content: None,
        },
        Tool::DocumentIndex { workspace } => RequestedAction {
            kind: "document_index".to_string(),
            principal: None,
            path: workspace.clone(),
            content: None,
        },
        Tool::AskLocalDocumentIndex { question } => RequestedAction {
            kind: "ask_local_document_index".to_string(),
            principal: Some(question.clone()),
            path: None,
            content: None,
        },
        Tool::SovereignDuckDuckGoSearch { query, max_results } => RequestedAction {
            kind: "sovereign_duckduckgo_search".to_string(),
            principal: Some(query.clone()),
            path: None,
            content: max_results.map(|value| value.to_string()),
        },
        Tool::RegisteredTaskTool(request) => {
            crate::tools::task_tool_runtime::requested_action(request)
        }
        Tool::Unsupported { requested } => RequestedAction {
            kind: "unsupported".to_string(),
            principal: Some(requested.clone()),
            path: None,
            content: None,
        },
    }
}
