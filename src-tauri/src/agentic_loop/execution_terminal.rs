use super::{execution_resume::AgentExecutionOriginGuard, AgenticLoopResponse};

pub(super) fn verified_payload(
    origin: &AgentExecutionOriginGuard,
    response: &AgenticLoopResponse,
) -> String {
    serde_json::json!({
        "schema": "oomu.agent_execution_terminal.v1",
        "executionId": origin.execution_id,
        "planId": origin.plan_id,
        "status": "completed",
        "mlcPath": response.mlc_path,
        "verified": response.verified,
        "verifiedNativeExecutionReceipt": response.verified,
        "outputs": response.outputs.len(),
    })
    .to_string()
}
