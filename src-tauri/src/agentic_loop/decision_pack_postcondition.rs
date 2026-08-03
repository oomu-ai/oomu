use super::{ActionPlan, AgenticLoopError, Tool};
use crate::{
    db::PersistenceEngine,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
};
use serde_json::{json, Value};

const REQUIRED_OPERATIONS: [&str; 3] = [
    "create_decision_pack",
    "create_conflict_free_calendar_event",
    "draft_decision_pack_email",
];
const MAX_RECEIPT_BYTES: usize = 128 * 1024;

#[derive(Debug)]
struct DecisionPackPostconditionContract {
    calendar_arguments: Value,
    mail_arguments: Value,
}

#[derive(Debug)]
pub(super) struct DecisionPackPostconditionEvidence {
    pub(super) execution_path: Vec<String>,
    pub(super) audit_payload: Value,
}

/// Re-proves the deterministic decision-pack exit condition immediately before
/// success is certified. The three readers called here are deliberately
/// non-mutating: a resumed execution can safely run this function again without
/// replaying any already-completed action.
pub(super) async fn verify_if_required(
    plan: &ActionPlan,
    outputs: &[ExecuteCommandResponse],
    persistence: &PersistenceEngine,
    execution_id: Option<&str>,
    app: Option<&tauri::AppHandle>,
) -> Result<Option<DecisionPackPostconditionEvidence>, AgenticLoopError> {
    let Some(contract) = exact_contract(plan) else {
        return Ok(None);
    };
    let [pack_output, calendar_output, mail_output] = exact_outputs(outputs)?;

    let resolved_mail = crate::tools::decision_pack_mail::resolve_and_verify_postcondition(
        persistence,
        execution_id,
        contract.mail_arguments,
        outputs,
    )
    .map_err(|_| postcondition_error("the four decision-pack files"))?;

    let calendar_evidence =
        crate::tools::system_calendar_event::verify_conflict_free_postcondition(
            &contract.calendar_arguments,
            &calendar_output.message,
        )
        .await
        .map_err(|_| postcondition_error("the Calendar event"))?;

    let app = app.ok_or_else(|| postcondition_error("the unsent Mail draft"))?;
    let mail_evidence = crate::tools::system_mail::verify_exact_draft_postcondition(
        app,
        resolved_mail,
        &mail_output.message,
    )
    .await
    .map_err(|_| postcondition_error("the unsent Mail draft"))?;

    let evidence_binding = json!({
        "schemaVersion": 1,
        "planId": plan.id,
        "packReceiptSha256": receipt_sha256(pack_output),
        "calendarReceiptSha256": receipt_sha256(calendar_output),
        "mailReceiptSha256": receipt_sha256(mail_output),
        "calendarEvidence": calendar_evidence,
        "mailEvidence": mail_evidence,
    });
    let evidence_sha256 = serde_json::to_vec(&evidence_binding)
        .map(|bytes| crate::foundation::digest::sha256_hex(&bytes))
        .map_err(|_| postcondition_error("the final verification evidence"))?;

    Ok(Some(DecisionPackPostconditionEvidence {
        execution_path: vec![
            "Final postcondition verification reopened and rehashed all four receipt-bound decision-pack files, re-read Calendar and proved exactly one matching conflict-free tentative event, and re-read Mail and proved exactly one matching unsent draft."
                .to_string(),
            format!(
                "CLAIM decision_pack_postcondition_verified=true file_count=4 calendar_exact_match_count=1 mail_exact_match_count=1 evidence_sha256={evidence_sha256}"
            ),
        ],
        audit_payload: json!({
            "fileCount": 4,
            "calendarExactMatchCount": 1,
            "mailExactMatchCount": 1,
            "evidenceSha256": evidence_sha256,
        }),
    }))
}

fn exact_contract(plan: &ActionPlan) -> Option<DecisionPackPostconditionContract> {
    if plan.steps.len() != REQUIRED_OPERATIONS.len() {
        return None;
    }
    let decision = registered_arguments(plan, 0, REQUIRED_OPERATIONS[0])?;
    let calendar = registered_arguments(plan, 1, REQUIRED_OPERATIONS[1])?;
    let mail = registered_arguments(plan, 2, REQUIRED_OPERATIONS[2])?;
    let output_directory = decision
        .get("outputDirectory")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if !super::plan_coverage::matches_deterministic_decision_pack_plan(plan, output_directory) {
        return None;
    }
    Some(DecisionPackPostconditionContract {
        calendar_arguments: calendar.clone(),
        mail_arguments: mail.clone(),
    })
}

fn registered_arguments<'a>(
    plan: &'a ActionPlan,
    index: usize,
    operation: &str,
) -> Option<&'a Value> {
    match &plan.steps.get(index)?.tool {
        Tool::RegisteredTaskTool(request) if request.operation == operation => {
            Some(&request.arguments)
        }
        _ => None,
    }
}

fn exact_outputs(
    outputs: &[ExecuteCommandResponse],
) -> Result<[&ExecuteCommandResponse; 3], AgenticLoopError> {
    if outputs.len() != REQUIRED_OPERATIONS.len() {
        return Err(postcondition_error("the completed action receipts"));
    }
    let mut matched = Vec::with_capacity(REQUIRED_OPERATIONS.len());
    for operation in REQUIRED_OPERATIONS {
        let candidates = outputs
            .iter()
            .filter(|output| output.operation == operation)
            .collect::<Vec<_>>();
        let [output] = candidates.as_slice() else {
            return Err(postcondition_error("the completed action receipts"));
        };
        if !output.verified
            || !matches!(&output.status, CommandStatus::Completed)
            || output.message.is_empty()
            || output.message.len() > MAX_RECEIPT_BYTES
            || serde_json::from_str::<Value>(&output.message).is_err()
        {
            return Err(postcondition_error("the completed action receipts"));
        }
        matched.push(*output);
    }
    matched
        .try_into()
        .map_err(|_| postcondition_error("the completed action receipts"))
}

fn receipt_sha256(output: &ExecuteCommandResponse) -> String {
    crate::foundation::digest::sha256_hex(output.message.as_bytes())
}

fn postcondition_error(subject: &str) -> AgenticLoopError {
    AgenticLoopError {
        code: "decision_pack_postcondition_failed",
        boundary: "DecisionPackPostcondition",
        message: format!(
            "OOMU finished the approved steps but could not freshly verify {subject}. Completion was not reported and no action was replayed. Try again to recheck the existing work."
        ),
        mlc_path: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBJECTIVE: &str = "Prepare a board-ready supplier decision pack. Read /tmp/oomu-postcondition/supplier_proposals.json and /tmp/oomu-postcondition/q3_strategic_vendor_proposals.txt. Reconcile every quoted amount and margin, identify all exceptions, and independently research current primary or official web sources for fuel or freight conditions. Cite every web claim with its URL and access time. Create a new /tmp/oomu-postcondition/ship_test_01 folder and deliver supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md. The workbook must contain source data, formulas, exception flags, and a recommendation sheet. The presentation and PDF must be executive-ready and mutually consistent. Then create a tentative 30-minute event in my OOMU Test calendar on the next weekday between 1:00 PM and 4:00 PM titled Supplier Decision Review, avoiding conflicts, and create a Mail draft to test@example.invalid summarizing the recommendation and listing the four output files. Do not send the email.";
    const OUTPUT_DIRECTORY: &str = "/tmp/oomu-postcondition/ship_test_01";

    fn output(operation: &str) -> ExecuteCommandResponse {
        ExecuteCommandResponse {
            operation: operation.to_string(),
            status: CommandStatus::Completed,
            message: json!({"operation": operation, "verified": true}).to_string(),
            metrics: None,
            claims: Vec::new(),
            verified: true,
            model_used: None,
        }
    }

    fn deterministic_plan() -> ActionPlan {
        let draft =
            super::super::plan_coverage::compile_decision_pack(OBJECTIVE, Some(OUTPUT_DIRECTORY))
                .expect("production contract should compile")
                .expect("objective should select the production decision-pack contract");
        super::super::generated_draft_to_plan(
            OBJECTIVE.to_string(),
            draft,
            super::super::ModelRouteDecision {
                selected_model: crate::shield_gate::ModelMetadata::local_gemma(),
                provider_config_id: None,
                provider_id: Some("local_model".to_string()),
                recommended_model: None,
                requires_principal_authorization: false,
                reason: "postcondition test route".to_string(),
                context_excerpt_count: 0,
                context_sources: Vec::new(),
            },
            super::super::ContextBundle {
                excerpts: Vec::new(),
                claim_sources: Vec::new(),
                inherited_artifact_hashes: Vec::new(),
            },
        )
    }

    #[test]
    fn exact_contract_selects_only_the_untampered_production_plan() {
        let plan = deterministic_plan();
        let contract = exact_contract(&plan).expect("exact deterministic plan must be selected");
        assert_eq!(contract.calendar_arguments["calendarName"], "OOMU Test");
        assert_eq!(contract.mail_arguments["to"], "test@example.invalid");

        let mut tampered = plan;
        let Tool::RegisteredTaskTool(calendar) = &mut tampered.steps[1].tool else {
            panic!("calendar step must remain registered");
        };
        calendar.arguments["durationMinutes"] = json!(45);
        assert!(exact_contract(&tampered).is_none());
    }

    #[test]
    fn exact_outputs_require_one_verified_receipt_per_operation() {
        let outputs = REQUIRED_OPERATIONS.map(output);
        let matched = exact_outputs(&outputs).expect("exact receipts should bind");
        assert_eq!(matched[0].operation, REQUIRED_OPERATIONS[0]);
        assert_eq!(matched[1].operation, REQUIRED_OPERATIONS[1]);
        assert_eq!(matched[2].operation, REQUIRED_OPERATIONS[2]);

        let duplicate = vec![
            output(REQUIRED_OPERATIONS[0]),
            output(REQUIRED_OPERATIONS[1]),
            output(REQUIRED_OPERATIONS[1]),
        ];
        assert_eq!(
            exact_outputs(&duplicate).unwrap_err().code,
            "decision_pack_postcondition_failed"
        );
    }

    #[test]
    fn exact_outputs_reject_unverified_or_non_json_receipts() {
        let mut unverified = REQUIRED_OPERATIONS.map(output);
        unverified[2].verified = false;
        assert!(exact_outputs(&unverified).is_err());

        let mut invalid = REQUIRED_OPERATIONS.map(output);
        invalid[0].message = "not-json".to_string();
        assert!(exact_outputs(&invalid).is_err());
    }

    #[test]
    fn receipt_binding_changes_with_the_receipt() {
        let original = output(REQUIRED_OPERATIONS[0]);
        let mut changed = original.clone();
        changed.message = json!({"verified": true, "files": 4}).to_string();
        assert_ne!(receipt_sha256(&original), receipt_sha256(&changed));
    }
}
