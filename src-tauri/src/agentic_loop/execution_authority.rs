use crate::{
    db::PersistenceEngine,
    foundation::digest::sha256_hex,
    gemma::{GemmaService, LocalDecisionDirective, LocalWorkflowDecision},
    tools::task_tool_runtime,
};
use std::future::Future;

pub(super) fn approved_registered_action_authorization(
    plan_approved: bool,
    operation: &str,
) -> Option<LocalWorkflowDecision> {
    if !plan_approved || !task_tool_runtime::is_registered(operation) {
        return None;
    }
    Some(LocalWorkflowDecision {
        directive: LocalDecisionDirective::Execute,
        thought_summary: format!(
            "The signed plan, registered {operation} contract, permission preflight, and required Shield authority were verified."
        ),
        premises: vec![
            "The user approved the signed action plan after deterministic contract verification."
                .to_string(),
            format!("The production task-tool registry contains {operation}."),
            "Permission preflight and Shield remain the authoritative execution boundaries."
                .to_string(),
        ],
        execution_path: vec![
            "Execute the verifier-approved registered action without a second model authority gate."
                .to_string(),
        ],
        formal_conclusion: format!(
            "Execute {operation} exactly as signed, registered, and approved."
        ),
        output_sha256: None,
    })
}

pub(super) fn approved_registered_action_certification(
    plan_approved: bool,
    operation: &str,
    output_json: &str,
) -> Option<LocalWorkflowDecision> {
    if !plan_approved || !task_tool_runtime::is_registered(operation) {
        return None;
    }
    Some(LocalWorkflowDecision {
        directive: LocalDecisionDirective::Certify,
        thought_summary: format!(
            "The verified {operation} result is bound to its exact runtime output digest."
        ),
        premises: vec![
            format!("The production task-tool registry contains {operation}."),
            "The runtime returned a completed, verified action result before certification."
                .to_string(),
        ],
        execution_path: vec![
            "Compute the certificate digest from the exact serialized runtime output bytes."
                .to_string(),
        ],
        formal_conclusion: format!(
            "Certify the verified {operation} result without a second model authority gate."
        ),
        output_sha256: Some(sha256_hex(output_json.as_bytes())),
    })
}

pub(super) async fn authorize_registered_or_model<F, E>(
    plan_approved: bool,
    operation: &str,
    model_decision: F,
) -> Result<(LocalWorkflowDecision, &'static str), E>
where
    F: Future<Output = Result<LocalWorkflowDecision, E>>,
{
    match approved_registered_action_authorization(plan_approved, operation) {
        Some(decision) => Ok((decision, "verified_registered_action")),
        None => model_decision
            .await
            .map(|decision| (decision, "deterministic_model")),
    }
}

pub(super) async fn certify_registered_or_model<F, E>(
    plan_approved: bool,
    operation: &str,
    output_json: &str,
    model_decision: F,
) -> Result<(LocalWorkflowDecision, &'static str), E>
where
    F: Future<Output = Result<LocalWorkflowDecision, E>>,
{
    match approved_registered_action_certification(plan_approved, operation, output_json) {
        Some(decision) => Ok((decision, "verified_registered_output")),
        None => model_decision
            .await
            .map(|decision| (decision, "deterministic_model")),
    }
}

pub(super) struct ExecutionDecisionContext<'a> {
    pub gemma: &'a GemmaService,
    pub persistence: &'a PersistenceEngine,
    pub app: Option<&'a tauri::AppHandle>,
    pub execution_id: Option<&'a str>,
    pub plan_id: &'a str,
    pub session_id: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    pub block_id: Option<&'a String>,
    pub step_index: usize,
}

impl ExecutionDecisionContext<'_> {
    pub async fn authorize(
        &self,
        plan_approved: bool,
        operation: &str,
        decision_session: &str,
        objective: &str,
        action_json: &str,
    ) -> Result<(LocalWorkflowDecision, &'static str), super::AgenticLoopError> {
        authorize_registered_or_model(
            plan_approved,
            operation,
            super::generate_workflow_decision_with_transient_retry(
                self.gemma,
                self.persistence,
                self.app,
                self.execution_id,
                self.plan_id,
                self.session_id,
                self.agent_id,
                self.block_id,
                self.step_index,
                operation,
                "authorize",
                decision_session,
                objective,
                action_json,
                None,
            ),
        )
        .await
    }

    pub async fn certify(
        &self,
        plan_approved: bool,
        operation: &str,
        decision_session: &str,
        objective: &str,
        action_json: &str,
        output_json: &str,
    ) -> Result<(LocalWorkflowDecision, &'static str), super::AgenticLoopError> {
        certify_registered_or_model(
            plan_approved,
            operation,
            output_json,
            super::generate_workflow_decision_with_transient_retry(
                self.gemma,
                self.persistence,
                self.app,
                self.execution_id,
                self.plan_id,
                self.session_id,
                self.agent_id,
                self.block_id,
                self.step_index,
                operation,
                "certify",
                decision_session,
                objective,
                action_json,
                Some(output_json),
            ),
        )
        .await
    }
}
