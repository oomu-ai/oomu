use super::{
    commands::create_presentation_internal, validate_presentation, CreatePresentationRequest,
    ElementContent, PresentationIr,
};
use crate::{
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::task_tool_runtime::{
        TaskToolExecutionContext, TaskToolFuture, TaskToolRegistration, TaskToolValidation,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const MAX_TOOL_ARGUMENT_BYTES: usize = 160 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreatePresentationToolRequest {
    presentation: PresentationIr,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreatePresentationBriefEnvelope {
    brief: PresentationBrief,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PresentationBrief {
    title: String,
    summary: String,
    locale: String,
    #[serde(default)]
    speaker_notes: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum IncomingPresentationRequest {
    Full(CreatePresentationToolRequest),
    Brief(CreatePresentationBriefEnvelope),
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: "create_presentation",
        validate: validate_registration,
        validate_resolved: validate_registration,
        resolve: resolve_registration,
        execute: execute_registration,
        planner_context: None,
        schema: presentation_tool_schema,
        metadata: crate::tools::task_tool_runtime::TaskToolMetadata {
            description: "Create a Project- and Task-bound presentation in the verified private artifact lifecycle.",
            risk_tier: crate::tools::task_tool_runtime::TaskToolRiskTier::FileWrite,
            approval_tier: crate::tools::task_tool_runtime::TaskToolApprovalTier::Background,
            agent_error_code: "presentation_tool_failed",
            agent_error_boundary: "CreatePresentation",
            execution_path: "The native create_presentation tool created a Project-bound private presentation review through the verified artifact lifecycle.",
        },
    })
}

fn presentation_tool_schema() -> Value {
    json!({
        "oneOf":[
            {"type":"object","properties":{"brief":{"type":"object","properties":{
                "title":{"type":"string","minLength":1,"maxLength":256},
                "summary":{"type":"string","minLength":1,"maxLength":2000},
                "locale":{"type":"string","minLength":2,"maxLength":35},
                "speakerNotes":{"type":"string","maxLength":32767}
            },"required":["title","summary","locale"],"additionalProperties":false}},"required":["brief"],"additionalProperties":false},
            {"type":"object","properties":{"presentation":{"type":"object"}},"required":["presentation"],"additionalProperties":false}
        ]
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    if serde_json::to_vec(&arguments)
        .map_err(|error| error.to_string())?
        .len()
        > MAX_TOOL_ARGUMENT_BYTES
    {
        return Err("create_presentation arguments exceed the bounded IR size.".to_string());
    }
    let request = decode_request(arguments)?;
    if request.presentation.revision != 1 {
        return Err("create_presentation requires revision 1.".to_string());
    }
    validate_presentation(&request.presentation)?;
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

fn decode_request(arguments: Value) -> Result<CreatePresentationToolRequest, String> {
    let incoming =
        serde_json::from_value::<IncomingPresentationRequest>(arguments).map_err(|_| {
            "create_presentation arguments do not match the registered schema.".to_string()
        })?;
    let presentation = match incoming {
        IncomingPresentationRequest::Full(request) => request.presentation,
        IncomingPresentationRequest::Brief(value) => presentation_from_brief(value.brief)?,
    };
    Ok(CreatePresentationToolRequest { presentation })
}

fn presentation_from_brief(brief: PresentationBrief) -> Result<PresentationIr, String> {
    let title = brief.title.trim().to_string();
    let summary = brief.summary.trim().to_string();
    let locale = brief.locale.trim().to_string();
    if title.is_empty()
        || title.chars().count() > 256
        || summary.is_empty()
        || summary.chars().count() > 2_000
        || !(2..=35).contains(&locale.len())
        || [&title, &summary, &locale, &brief.speaker_notes]
            .iter()
            .any(|value| value.contains('\0'))
        || brief.speaker_notes.chars().count() > 32_767
    {
        return Err("create_presentation brief is outside the bounded contract.".to_string());
    }
    let mut presentation = super::deterministic_presentation_fixture();
    presentation.title = title.clone();
    presentation.locale = locale;
    presentation.policy.overflow = super::OverflowPolicy::ShrinkToFit;
    let slide = &mut presentation.slides[0];
    slide.title = Some(title.clone());
    slide.notes.speaker_notes = brief.speaker_notes;
    slide
        .elements
        .retain(|element| matches!(element.object_id.as_str(), "title" | "summary"));
    for element in &mut slide.elements {
        let ElementContent::TextBox { text } = &mut element.content else {
            continue;
        };
        text.paragraphs[0].runs[0].text = if element.object_id == "title" {
            title.clone()
        } else {
            summary.clone()
        };
    }
    Ok(presentation)
}

fn resolve_registration(
    _persistence: &crate::db::PersistenceEngine,
    _execution_id: Option<&str>,
    arguments: Value,
    _outputs: &[ExecuteCommandResponse],
) -> Result<Value, String> {
    Ok(validate_registration(arguments)?.arguments)
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request =
            serde_json::from_value::<CreatePresentationToolRequest>(arguments).map_err(|_| {
                "create_presentation arguments do not match the registered schema.".to_string()
            })?;
        let execution_id = context
            .execution_id
            .ok_or_else(|| "create_presentation requires an active agent Task.".to_string())?;
        let app = context
            .app
            .ok_or_else(|| "create_presentation requires the app runtime.".to_string())?;
        let task = crate::tools::task_runtime::require_agent_runtime_task(
            context.persistence,
            execution_id,
        )?;
        let project_id = task.project_id.clone();
        let review = create_presentation_internal(
            CreatePresentationRequest {
                project_id: project_id.clone(),
                task_id: task.task_id.clone(),
                task_run_id: task.task_run_id.clone(),
                title: request.presentation.title.clone(),
                presentation: request.presentation,
            },
            context.persistence,
            context.identity,
            app,
        )
        .await
        .map_err(|value| value.code)?;
        let message = serde_json::to_string(&json!({
            "presentationId": review.summary.presentation_id,
            "artifactId": review.summary.artifact_id,
            "projectId": review.summary.project_id,
            "taskRunId": review.summary.task_run_id,
            "title": review.summary.title,
            "revision": review.summary.current_revision,
            "documentFocus": {"kind":"presentation","id":review.summary.presentation_id},
            "exportReady": review.summary.exportable,
        }))
        .map_err(|error| error.to_string())?;
        Ok(ExecuteCommandResponse {
            operation: "create_presentation".to_string(),
            status: CommandStatus::Completed,
            message,
            metrics: None,
            claims: vec![format!(
                "CLAIM presentation_artifact_created artifact_id={} task_run_id={} revision={} export_ready={}",
                review.summary.artifact_id,
                review.summary.task_run_id,
                review.summary.current_revision,
                review.summary.exportable
            )],
            verified: review.summary.structurally_verified,
            model_used: None,
        })
    })
}
