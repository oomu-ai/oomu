use super::*;

pub(super) fn deterministic_instruction_free_output(
    workflow_ir: &WorkflowIr,
) -> Option<CompilerOutput> {
    (!workflow_ir
        .nodes
        .iter()
        .any(|node| matches!(node, WorkflowNode::Agent(_))))
    .then(|| CompilerOutput {
        compiler_version: COMPILER_VERSION.to_string(),
        instructions: Vec::new(),
    })
}

pub(super) fn compiler_infer_request(prompt: impl Into<String>, session_id: &str) -> InferRequest {
    let mut request = InferRequest::new(prompt);
    request.session_id = Some(session_id.to_string());
    request.deterministic = true;
    request.context_size = Some(WORKFLOW_COMPILER_CONTEXT_SIZE);
    request.max_tokens = Some(WORKFLOW_INSTRUCTION_COMPILER_MAX_NEW_TOKENS);
    request.grammar = Some(compiler_output_grammar().to_string());
    request
}
