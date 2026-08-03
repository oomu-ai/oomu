use super::{
    compact_error, looks_like_placeholder_capability, looks_like_placeholder_reason,
    missing_capability_reason, missing_capability_titles, resolve_missing_capability_details,
    unix_time_ms, ComposeAttemptError, ComposeWorkflowRequest, ComposeWorkflowResponse,
    RawComposeOutput,
};

pub(super) fn resolve_non_composed_output(
    raw: RawComposeOutput,
    request: &ComposeWorkflowRequest,
    attempt: usize,
    started_at: i64,
) -> Result<ComposeWorkflowResponse, ComposeAttemptError> {
    let RawComposeOutput {
        status,
        reason,
        workflow_ir,
        partial_draft,
        missing_capabilities,
    } = raw;
    let partial_draft = partial_draft.or(workflow_ir);
    if looks_like_placeholder_reason(&reason)
        || missing_capabilities
            .iter()
            .any(|capability| looks_like_placeholder_capability(capability))
    {
        return Err(ComposeAttemptError {
            message: "Gemma returned placeholder connection guidance instead of catalog-grounded capabilities."
                .to_string(),
            partial_draft,
            missing_capabilities: Vec::new(),
            missing_capability_details: Vec::new(),
        });
    }

    if status == "needs_connection" || !missing_capabilities.is_empty() {
        let details = resolve_missing_capability_details(
            &missing_capabilities,
            &request.capability_catalog,
            partial_draft.clone(),
        )?;
        let missing = missing_capability_titles(&details);
        return Ok(ComposeWorkflowResponse {
            status: "needs_connection",
            reason: missing_capability_reason(&details),
            workflow_ir: None,
            partial_draft,
            missing_capabilities: missing,
            missing_capability_details: details,
            composed_by: "gemma",
            attempts: attempt + 1,
            latency_ms: unix_time_ms().saturating_sub(started_at),
        });
    }

    Err(ComposeAttemptError {
        message: if reason.trim().is_empty() {
            "Gemma returned a failed compose response without a runnable workflow.".to_string()
        } else {
            format!(
                "Gemma returned a failed compose response: {}",
                compact_error(&reason)
            )
        },
        partial_draft,
        missing_capabilities: Vec::new(),
        missing_capability_details: Vec::new(),
    })
}
