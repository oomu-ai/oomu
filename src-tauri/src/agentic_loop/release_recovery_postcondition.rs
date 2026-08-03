use super::{ActionPlan, AgenticLoopError, Tool};
use crate::shield_gate::ExecuteCommandResponse;
use serde_json::{json, Value};

#[derive(Debug)]
pub(super) struct ReleaseRecoveryPostconditionEvidence {
    pub(super) execution_path: Vec<String>,
    pub(super) audit_payload: Value,
}

/// Freshly reopens all three surfaces immediately before success is certified.
/// This reader is non-mutating and never replays an already completed action.
pub(super) async fn verify_if_required(
    plan: &ActionPlan,
    outputs: &[ExecuteCommandResponse],
    app: Option<&tauri::AppHandle>,
) -> Result<Option<ReleaseRecoveryPostconditionEvidence>, AgenticLoopError> {
    if !super::plan_coverage::matches_deterministic_release_recovery_plan(plan) {
        return Ok(None);
    }
    let calendar =
        registered_arguments(plan, 1, crate::tools::release_recovery::CALENDAR_OPERATION)?;
    let mail = registered_arguments(plan, 2, crate::tools::release_recovery::MAIL_OPERATION)?;
    let requested_calendar =
        super::plan_coverage::release_recovery_requested_calendar_name(&plan.objective)
            .ok_or_else(|| postcondition_error("the originally requested Calendar target"))?;
    let app = app.ok_or_else(|| postcondition_error("the native Calendar and Mail state"))?;
    let evidence = crate::tools::release_recovery::verify_postcondition(
        calendar.clone(),
        mail.clone(),
        requested_calendar,
        outputs,
        app,
    )
    .await
    .map_err(|_| postcondition_error("the agenda, Calendar event, and unsent Mail draft"))?;
    let evidence_sha256 = serde_json::to_vec(&evidence)
        .map(|bytes| crate::foundation::digest::sha256_hex(&bytes))
        .map_err(|_| postcondition_error("the final cross-surface evidence"))?;
    Ok(Some(ReleaseRecoveryPostconditionEvidence {
        execution_path: vec![
            "Final postcondition verification reopened and rehashed the exact Markdown agenda, re-read Calendar and proved exactly one matching conflict-free tentative event, and re-read Mail and proved exactly one matching unsent draft with no sent copy."
                .to_string(),
            format!(
                "CLAIM release_recovery_postcondition_verified=true file_count=1 calendar_exact_match_count=1 mail_exact_match_count=1 sent_match_count=0 evidence_sha256={evidence_sha256}"
            ),
        ],
        audit_payload: json!({
            "fileCount":1,
            "calendarExactMatchCount":1,
            "mailExactMatchCount":1,
            "sentMatchCount":0,
            "evidenceSha256":evidence_sha256,
            "evidence":evidence,
        }),
    }))
}

fn registered_arguments<'a>(
    plan: &'a ActionPlan,
    index: usize,
    operation: &str,
) -> Result<&'a Value, AgenticLoopError> {
    match &plan.steps.get(index).map(|step| &step.tool) {
        Some(Tool::RegisteredTaskTool(request)) if request.operation == operation => {
            Ok(&request.arguments)
        }
        _ => Err(postcondition_error("the signed receipt-bound plan")),
    }
}

fn postcondition_error(subject: &str) -> AgenticLoopError {
    AgenticLoopError {
        code: "release_recovery_postcondition_failed",
        boundary: "ReleaseRecoveryPostcondition",
        message: format!(
            "OOMU finished the approved steps but could not freshly verify {subject}. Completion was not reported and no action was replayed. Review the existing work, then try the verification again."
        ),
        mlc_path: None,
    }
}
