use super::WorkflowCompilerError;

impl WorkflowCompilerError {
    pub(super) fn invalid_request(message: &str) -> Self {
        Self::new("workflow_save_invalid", message.to_string())
    }

    pub(super) fn invalid_ir(errors: Vec<String>) -> Self {
        let detail = errors.join("; ");
        Self::new(
            "workflow_ir_invalid",
            format!(
                "This storyboard is not runnable yet. Review the highlighted steps, keep one input and one output, and save again. Details: {}",
                compact_error(&detail)
            ),
        )
    }

    pub(super) fn invalid_output(error: serde_json::Error) -> Self {
        Self::new(
            "workflow_compiler_json_invalid",
            format!("Gemma returned invalid compiler JSON: {error}"),
        )
    }

    pub(super) fn contract(message: String) -> Self {
        Self::new("workflow_compiler_contract_invalid", message)
    }

    pub(super) fn serialization(error: serde_json::Error) -> Self {
        Self::new("workflow_compiler_serialization_failed", error.to_string())
    }

    pub(super) fn metadata(message: String) -> Self {
        Self::new("workflow_compiler_metadata_failed", message)
    }

    pub(super) fn inference(error: crate::gemma::GemmaError) -> Self {
        Self::new(error.code, error.message)
    }

    pub(super) fn database(error: rusqlite::Error) -> Self {
        Self::new("workflow_compiler_database_failed", error.to_string())
    }

    pub(super) fn runtime(message: String) -> Self {
        Self::new("workflow_compiler_worker_failed", message)
    }

    pub(super) fn topological_anomaly(code: &'static str, message: String) -> Self {
        Self::new(code, message)
    }

    fn new(code: &'static str, message: String) -> Self {
        Self {
            code,
            boundary: "WorkflowCompiler",
            message,
        }
    }
}

pub(super) fn compact_error(message: &str) -> String {
    message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(220)
        .collect()
}
