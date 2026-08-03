use super::*;

pub(super) fn resolved_gemma_runtime_model(
    app: &tauri::AppHandle,
    gemma: GemmaService,
) -> Result<GemmaRuntimeModel, WorkflowRuntimeError> {
    Ok(GemmaRuntimeModel {
        gemma,
        model_id: resolve_default_workflow_model_id(app)?,
    })
}

fn resolve_default_workflow_model_id(
    app: &tauri::AppHandle,
) -> Result<String, WorkflowRuntimeError> {
    let configured_model_id = crate::settings::resolved_default_prewarmed_model_id(app)
        .map_err(|error| {
            WorkflowRuntimeError::new(
                "workflow_runtime_model_setting_unavailable",
                format!(
                    "OOMU couldn't read your default local model, so this Workflow did not run. Check the local model in Settings and try again. {error}"
                ),
            )
        })?;
    let model_root = crate::settings::resolved_local_model_directory(app).map_err(|error| {
        WorkflowRuntimeError::new(
            "workflow_runtime_model_store_unavailable",
            format!(
                "OOMU couldn't open your local model folder, so this Workflow did not run. Check the folder in Settings and try again. {error}"
            ),
        )
    })?;
    crate::gemma::resolve_exact_ready_local_model(&model_root, &configured_model_id)
        .map(|model| model.id)
        .map_err(|error| {
            WorkflowRuntimeError::new(
                "workflow_runtime_model_unavailable",
                format!(
                    "Your default local model ({}) isn't ready, so this Workflow did not run. Choose an available local model in Settings and try again. {}",
                    configured_model_id, error.message
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn workflow_runtime_has_no_fixed_generation_checkpoint() {
        let runtime = include_str!("../workflow_runtime.rs");
        assert!(!runtime.contains("const WORKFLOW_RUNTIME_MODEL"));
        assert!(!runtime.contains(".infer_model_sync(\""));
        assert!(runtime.contains("infer_model_sync(&self.model_id, request)"));
    }

    #[test]
    fn configured_authority_is_bound_for_run_resume_and_retry() {
        let runtime = include_str!("../workflow_runtime.rs");
        let scheduled = include_str!("scheduled_execution.rs");
        assert_eq!(
            runtime
                .matches("resolved_gemma_runtime_model(&app,")
                .count(),
            4
        );
        assert_eq!(
            scheduled
                .matches("resolved_gemma_runtime_model(&app,")
                .count(),
            1
        );
    }
}
