#![cfg(test)]

use super::{
    approved_registered_action_authorization, approved_registered_action_certification,
    step_to_request, LocalDecisionDirective,
};
use crate::{gemma::GemmaService, verifier::MlcVerifier};
use std::{path::PathBuf, time::Instant};

const TEST_FOUR_RESOLVED_OBJECTIVE: &str = "prepare a board-ready supplier decision pack. Read /Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json and /Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt from my testing folder. Reconcile every quoted amount and margin, identify all exceptions, and independently research current primary or official web sources for fuel or freight conditions that could materially affect the recommendation. Cite every web claim with its URL and access time. Create a new ship_test_01 folder in the testing folder and deliver four real files: supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md. The workbook must contain source data, formulas, exception flags, and a recommendation sheet. The presentation and PDF must be executive-ready and mutually consistent. Then create a tentative 30-minute event in my OOMU Test calendar on the next weekday between 1:00 PM and 4:00 PM titled Supplier Decision Review, avoiding conflicts, and create a Mail draft to recipient@example.com summarizing the recommendation and listing the four output files. Do not send the email. Ask for any required approvals and continue from the exact stopped step after I approve. Do not claim completion until you have verified that all four files, the calendar event, and the unsent Mail draft actually exist.";
const OUTPUT_DIRECTORY: &str =
    "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/ship_test_01";
const SOURCE_PATHS: [&str; 2] = [
    "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json",
    "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt",
];

#[test]
#[ignore = "requires the real Scenario 1 source files in the user's testing folder"]
fn scenario_one_test_four_reaches_the_real_signed_approval_boundary() {
    for source in SOURCE_PATHS {
        let metadata =
            std::fs::symlink_metadata(source).expect("the functional Scenario 1 input must exist");
        assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    }
    crate::decision_pack::register_task_tool().expect("register the production decision-pack tool");
    crate::tools::system_calendar_event::register_task_tool()
        .expect("register the production Calendar tool");
    crate::tools::decision_pack_mail::register_task_tool()
        .expect("register the production Mail-draft tool");
    for operation in [
        "create_decision_pack",
        "create_conflict_free_calendar_event",
        "draft_decision_pack_email",
    ] {
        assert!(crate::tools::task_tool_runtime::is_registered(operation));
    }

    let (plan, identity) = crate::agentic_loop::tests::compile_signed_decision_pack_plan(
        TEST_FOUR_RESOLVED_OBJECTIVE,
        OUTPUT_DIRECTORY,
    )
    .expect("compile and sign the exact grounded Scenario 1 plan");
    let preview = MlcVerifier::new()
        .verify_plan_preview(&plan, &identity)
        .expect("the exact plan should reach the production approval-preview boundary");

    assert_eq!(preview.authorized_actions.len(), 3);
    assert_eq!(preview.execution_path.len(), 3);
}

#[test]
fn scenario_one_plan_authority_matcher_rejects_any_step_drift() {
    let (plan, _) = crate::agentic_loop::tests::compile_signed_decision_pack_plan(
        TEST_FOUR_RESOLVED_OBJECTIVE,
        OUTPUT_DIRECTORY,
    )
    .expect("compile the exact deterministic Scenario 1 plan");
    assert!(
        crate::agentic_loop::scenario_plan::matches_scenario_one_deterministic_plan(
            &plan,
            OUTPUT_DIRECTORY,
        )
    );

    let mut reordered = plan;
    reordered.steps.swap(0, 1);
    assert!(
        !crate::agentic_loop::scenario_plan::matches_scenario_one_deterministic_plan(
            &reordered,
            OUTPUT_DIRECTORY,
        )
    );
}

#[test]
fn approved_scenario_one_registered_action_has_stable_execution_authority() {
    let _ = crate::decision_pack::register_task_tool();
    assert!(crate::tools::task_tool_runtime::is_registered(
        "create_decision_pack"
    ));

    let first = approved_registered_action_authorization(true, "create_decision_pack")
        .expect("approved registered production action has deterministic authority");
    let second = approved_registered_action_authorization(true, "create_decision_pack")
        .expect("identical approved input resolves through the same authority path");
    assert!(matches!(first.directive, LocalDecisionDirective::Execute));
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    assert!(
        approved_registered_action_authorization(false, "create_decision_pack").is_none(),
        "unapproved work must still use the normal authorization boundary"
    );

    let exact_output =
        r#"{"operation":"create_decision_pack","status":"completed","verified":true}"#;
    let first_certificate =
        approved_registered_action_certification(true, "create_decision_pack", exact_output)
            .expect("approved registered output has deterministic certification");
    let second_certificate =
        approved_registered_action_certification(true, "create_decision_pack", exact_output)
            .expect("identical exact output resolves through the same certification path");
    assert!(matches!(
        first_certificate.directive,
        LocalDecisionDirective::Certify
    ));
    assert_eq!(
        first_certificate.output_sha256.as_deref(),
        Some(crate::foundation::digest::sha256_hex(exact_output.as_bytes()).as_str())
    );
    assert_eq!(
        serde_json::to_string(&first_certificate).unwrap(),
        serde_json::to_string(&second_certificate).unwrap()
    );
}

#[test]
#[ignore = "loads the installed E4B GGUF model and exercises the real workflow decision engine"]
fn scenario_one_uses_a_stable_e4b_structured_workflow_decision() {
    for source in SOURCE_PATHS {
        let metadata =
            std::fs::symlink_metadata(source).expect("the functional Scenario 1 input must exist");
        assert!(metadata.is_file() && !metadata.file_type().is_symlink());
    }
    crate::decision_pack::register_task_tool().expect("register the production decision-pack tool");
    crate::tools::system_calendar_event::register_task_tool()
        .expect("register the production Calendar tool");
    crate::tools::decision_pack_mail::register_task_tool()
        .expect("register the production Mail-draft tool");

    let (plan, identity) = crate::agentic_loop::tests::compile_signed_decision_pack_plan(
        TEST_FOUR_RESOLVED_OBJECTIVE,
        OUTPUT_DIRECTORY,
    )
    .expect("compile and sign the exact grounded Scenario 1 plan");
    let preview = MlcVerifier::new()
        .verify_approved_plan(&plan, &identity)
        .expect("the approved exact plan must pass production verification");
    assert_eq!(preview.authorized_actions.len(), 3);

    // This is the exact immutable first-action envelope. Approved registered
    // execution no longer delegates authority to the model; this live probe
    // verifies that the remaining structured-decision path is deterministic.
    // No filesystem, Calendar, or Mail mutation occurs here.
    let action = step_to_request(&plan.steps[0]);
    let action_json = serde_json::to_string(&action).expect("serialize the approved first action");
    let model_directory = PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR)
        .join("../assets/models/gemma-4-E4B-it-qat-q4_0-gguf");
    let service = GemmaService::new_loading();
    service
        .load_model_from_dir(model_directory)
        .expect("load the installed Scenario 1 session model");

    let mut expected_decision = None;
    for run in 1..=3 {
        let started = Instant::now();
        let decision = service
            .generate_workflow_decision_sync(
                &format!("scenario-one-test-seven-execution-check-{run}"),
                &plan.objective,
                &action_json,
                None,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "Scenario 1 first execution decision run {run} failed after {} ms: code={} message={}",
                    started.elapsed().as_millis(),
                    error.code,
                    error.message
                )
            });
        let elapsed_ms = started.elapsed().as_millis();
        eprintln!(
            "SCENARIO_ONE_FIRST_EXECUTION_DECISION run={run} elapsed_ms={elapsed_ms} directive={:?} conclusion={}",
            decision.directive, decision.formal_conclusion
        );
        assert!(
            matches!(decision.directive, LocalDecisionDirective::Execute),
            "the approved exact decision-pack action must not dead-end at Execution check"
        );
        let serialized = serde_json::to_string(&decision).expect("serialize workflow decision");
        if let Some(expected) = expected_decision.as_ref() {
            assert_eq!(
                &serialized, expected,
                "identical E4B structured workflow inputs must not vary between sessions"
            );
        } else {
            expected_decision = Some(serialized);
        }
    }
    service.shutdown();
}
