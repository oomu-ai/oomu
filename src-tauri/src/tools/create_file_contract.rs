use super::task_tool_runtime::{
    TaskToolApprovalTier, TaskToolMetadata, TaskToolRiskTier, TaskToolValidation,
};
#[cfg(test)]
use super::task_tool_runtime::{TaskToolExecutionContext, TaskToolFuture, TaskToolRegistration};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

const MAX_FILE_CONTENT_BYTES: usize = 1_048_576;
const SUPPORTED_FORMATS: &[&str] = &[
    "csv", "docx", "html", "json", "md", "pdf", "pptx", "rtf", "txt", "xls", "xlsx", "xml",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateFileEnvelope {
    pub(crate) file: CreateFileBrief,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateFileBrief {
    pub(crate) title: String,
    pub(crate) content: String,
    pub(crate) locale: String,
    pub(crate) format: String,
    pub(crate) destination_path: String,
}

pub(crate) fn schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "file":{
                "type":"object",
                "properties":{
                    "title":{"type":"string","minLength":1,"maxLength":240},
                    "content":{"type":"string","maxLength":1048576},
                    "locale":{"type":"string","minLength":2,"maxLength":35},
                    "format":{"type":"string","enum":SUPPORTED_FORMATS},
                    "destinationPath":{"type":"string","minLength":1,"maxLength":4096}
                },
                "required":["title","content","locale","format","destinationPath"],
                "additionalProperties":false
            }
        },
        "required":["file"],
        "additionalProperties":false
    })
}

pub(crate) fn validate(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request = serde_json::from_value::<CreateFileEnvelope>(arguments)
        .map_err(|_| "create_file arguments do not match the registered schema.".to_string())?;
    request.file.title = request.file.title.trim().to_string();
    request.file.locale = request.file.locale.trim().to_string();
    request.file.format = request.file.format.trim().to_ascii_lowercase();
    request.file.destination_path = request.file.destination_path.trim().to_string();

    let brief = &request.file;
    if brief.title.is_empty()
        || brief.title.chars().count() > 240
        || brief.content.len() > MAX_FILE_CONTENT_BYTES
        || !(2..=35).contains(&brief.locale.len())
        || brief.destination_path.is_empty()
        || brief.destination_path.len() > 4096
        || [
            brief.title.as_str(),
            brief.content.as_str(),
            brief.locale.as_str(),
            brief.destination_path.as_str(),
        ]
        .iter()
        .any(|value| value.contains('\0'))
        || !SUPPORTED_FORMATS.contains(&brief.format.as_str())
    {
        return Err("create_file request is outside the bounded contract.".to_string());
    }
    let extension = Path::new(&brief.destination_path)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| {
            "create_file destination requires a supported file extension.".to_string()
        })?;
    if extension != brief.format {
        return Err("create_file destination does not match the requested format.".to_string());
    }
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: true,
    })
}

pub(crate) const METADATA: TaskToolMetadata = TaskToolMetadata {
    description: "Create a real local file through the native writer for its exact format.",
    risk_tier: TaskToolRiskTier::FileWrite,
    approval_tier: TaskToolApprovalTier::Visual,
    agent_error_code: "file_creation_failed",
    agent_error_boundary: "CreateFile",
    execution_path: "The native create_file tool wrote the requested format to the approved exact destination and verified the resulting bytes.",
};

#[cfg(test)]
pub(crate) fn register_preview_contract() {
    fn preview_only<'a>(
        _context: TaskToolExecutionContext<'a>,
        _arguments: Value,
    ) -> TaskToolFuture<'a> {
        Box::pin(async { Err("create_file_preview_contract_cannot_execute".to_string()) })
    }
    let registration = TaskToolRegistration {
        operation: "create_file",
        validate,
        validate_resolved: validate,
        resolve: super::task_tool_runtime::identity_resolver,
        execute: preview_only,
        planner_context: None,
        schema,
        metadata: METADATA,
    };
    if super::task_tool_runtime::register(registration).is_err() {
        assert!(super::task_tool_runtime::schema("create_file").is_ok());
    }
}
