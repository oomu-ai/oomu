use super::{
    compose_output_grammar, run_workflow_compiler_guard, unix_time_ms, ComposeWorkflowResponse,
    WorkflowCompilerError, WORKFLOW_COMPILER_CONTEXT_SIZE,
};
use crate::gemma::InferRequest;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

const WORKFLOW_COMPILER_MAX_NEW_TOKENS: usize = 1_536;
const WORKFLOW_COMPOSER_DEADLINE: Duration = Duration::from_secs(30);
const WORKFLOW_COMPOSER_CLEANUP_GRACE: Duration = Duration::from_secs(2);
const WORKFLOW_COMPOSER_TIMEOUT_REASON: &str = "workflow_composer_timeout";

pub(super) fn compose_disabled_response() -> ComposeWorkflowResponse {
    ComposeWorkflowResponse {
        status: "disabled",
        reason: "Workflow natural-language authoring is disabled by the workflow authoring feature flag."
            .to_string(),
        workflow_ir: None,
        partial_draft: None,
        missing_capabilities: Vec::new(),
        missing_capability_details: Vec::new(),
        composed_by: "not_run",
        attempts: 0,
        latency_ms: 0,
    }
}

pub(super) async fn run_bounded_workflow_compiler(
    operation: &'static str,
    work: impl FnOnce(Arc<AtomicBool>) -> Result<ComposeWorkflowResponse, WorkflowCompilerError>
        + Send
        + 'static,
) -> Result<ComposeWorkflowResponse, WorkflowCompilerError> {
    let started_at = unix_time_ms();
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = Arc::clone(&cancellation);
    let (sender, mut receiver) = tokio::sync::oneshot::channel();
    thread::Builder::new()
        .name(format!("oomu-workflow-{operation}"))
        .spawn(move || {
            let result = run_workflow_compiler_guard(operation, || work(worker_cancellation));
            let _ = sender.send(result);
        })
        .map_err(|error| WorkflowCompilerError::runtime(error.to_string()))?;

    match await_bounded_workflow_worker(
        &mut receiver,
        &cancellation,
        WORKFLOW_COMPOSER_DEADLINE,
        WORKFLOW_COMPOSER_CLEANUP_GRACE,
    )
    .await
    {
        Some(Ok(result)) => result,
        Some(Err(error)) => Err(WorkflowCompilerError::runtime(error.to_string())),
        None => Ok(ComposeWorkflowResponse {
            status: "failed",
            reason: WORKFLOW_COMPOSER_TIMEOUT_REASON.to_string(),
            workflow_ir: None,
            partial_draft: None,
            missing_capabilities: Vec::new(),
            missing_capability_details: Vec::new(),
            composed_by: "gemma",
            attempts: 1,
            latency_ms: unix_time_ms().saturating_sub(started_at),
        }),
    }
}

pub(super) async fn await_bounded_workflow_worker<T>(
    receiver: &mut tokio::sync::oneshot::Receiver<T>,
    cancellation: &Arc<AtomicBool>,
    runtime_limit: Duration,
    cleanup_grace: Duration,
) -> Option<Result<T, tokio::sync::oneshot::error::RecvError>> {
    let cleanup_limit = cleanup_grace.min(runtime_limit);
    let inference_limit = runtime_limit.saturating_sub(cleanup_limit);
    match tokio::time::timeout(inference_limit, &mut *receiver).await {
        Ok(result) => Some(result),
        Err(_) => {
            cancellation.store(true, Ordering::SeqCst);
            let _ = tokio::time::timeout(cleanup_limit, &mut *receiver).await;
            None
        }
    }
}

pub(super) fn compose_infer_request(
    prompt: impl Into<String>,
    session_id: &str,
    cancellation: &Arc<AtomicBool>,
) -> InferRequest {
    let mut request = InferRequest::new(prompt);
    request.session_id = Some(session_id.to_string());
    request.deterministic = true;
    request.context_size = Some(WORKFLOW_COMPILER_CONTEXT_SIZE);
    request.max_tokens = Some(WORKFLOW_COMPILER_MAX_NEW_TOKENS);
    request.grammar = Some(compose_output_grammar().to_string());
    request.cancellation = Arc::clone(cancellation);
    request
}
