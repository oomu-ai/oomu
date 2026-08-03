use super::{workflow_decision_grammar, InferRequest};

pub(super) fn workflow_decision_request(prompt: String, session_id: &str) -> InferRequest {
    let mut request = InferRequest::new(prompt);
    request.session_id = Some(session_id.to_string());
    // Authorization and certification protocols must not sample different
    // directives for identical inputs.
    request.deterministic = true;
    request.grammar = Some(workflow_decision_grammar().to_string());
    request
}
