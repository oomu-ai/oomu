use super::*;
use crate::gemma::{GeneratedPlanStepDraft, GeneratedRiskLevel, IntentSource};
use serde_json::json;

const SCENARIO_OBJECTIVE: &str = "OOMU, prepare a board-ready supplier decision pack. Read `mock_data/supplier_proposals.json` and `mock_data/q3_strategic_vendor_proposals.txt` from my testing folder. Reconcile every quoted amount and margin, identify all exceptions, and independently research current primary or official web sources for fuel or freight conditions that could materially affect the recommendation. Cite every web claim with its URL and access time. Create a new `ship_test_01` folder in the testing folder and deliver four real files: `supplier_decision.xlsx`, `supplier_decision.pptx`, `supplier_decision.pdf`, and `sources.md`. The workbook must contain source data, formulas, exception flags, and a recommendation sheet. The presentation and PDF must be executive-ready and mutually consistent. Then create a tentative 30-minute event in my `OOMU Test` calendar on the next weekday between 1:00 PM and 4:00 PM titled `Supplier Decision Review`, avoiding conflicts, and create a Mail draft to `<TEST_RECIPIENT>` summarizing the recommendation and listing the four output files. Do not send the email. Ask for any required approvals and continue from the exact stopped step after I approve. Do not claim completion until you have verified that all four files, the calendar event, and the unsent Mail draft actually exist.";

const TEST_FOUR_RESOLVED_OBJECTIVE: &str = "prepare a board-ready supplier decision pack. Read /Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json and /Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt from my testing folder. Reconcile every quoted amount and margin, identify all exceptions, and independently research current primary or official web sources for fuel or freight conditions that could materially affect the recommendation. Cite every web claim with its URL and access time. Create a new ship_test_01 folder in the testing folder and deliver four real files: supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md. The workbook must contain source data, formulas, exception flags, and a recommendation sheet. The presentation and PDF must be executive-ready and mutually consistent. Then create a tentative 30-minute event in my OOMU Test calendar on the next weekday between 1:00 PM and 4:00 PM titled Supplier Decision Review, avoiding conflicts, and create a Mail draft to recipient@example.com summarizing the recommendation and listing the four output files. Do not send the email. Ask for any required approvals and continue from the exact stopped step after I approve. Do not claim completion until you have verified that all four files, the calendar event, and the unsent Mail draft actually exist.";

const TEST_EIGHT_SIMPLIFIED_FAMILY_OBJECTIVE: &str = "Prepare a board-ready supplier decision pack using the two files in my testing folder:\n/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json and\n/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt.\nReconcile every quoted amount and margin, identify all discrepancies or exceptions, and recommend a supplier.\nResearch current fuel or freight conditions that could materially affect the recommendation using primary or official web sources.\nCite every web-based claim with its URL and access time.\nCreate a folder named ship_test_01 inside the testing folder. Deliver four real files inside it: supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md. The workbook should include the source data, formulas, exception flags, and a recommendation sheet. The presentation and PDF should be executive-ready and consistent with each other and the workbook. The Markdown file should document every web source, supported claim, URL, and access time.\nNext, create a tentative 30-minute event titled Supplier Decision Review in my Family calendar. Schedule it on the next weekday between 1:00 PM and 4:00 PM without conflicting with another event. Also create an unsent Mail draft to recipient@example.com that summarizes the recommendation and lists the four output files.\nAsk for approval whenever necessary, then continue from the exact step where you stopped after I approve. Do not ask me to repeat the request or redo completed work. Do not claim completion until you have verified that all four files, the calendar event, and the unsent Mail draft actually exist and contain the required information.";

#[test]
fn literal_test_four_compiles_to_the_exact_native_contract_without_model_json() {
    let output_directory =
        "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/ship_test_01";
    let compiled = compile_decision_pack(TEST_FOUR_RESOLVED_OBJECTIVE, Some(output_directory))
        .expect("the grounded contract should compile")
        .expect("the exact Scenario 1 objective is a decision-pack contract");

    assert!(matches!(compiled.source, IntentSource::Deterministic));
    assert!(compiled.degraded_reason.is_none());
    assert_eq!(compiled.steps.len(), 3);
    let generated_contract: serde_json::Value = serde_json::from_str(&compiled.generated_text)
        .expect("the deterministic audit text must remain valid ActionPlan JSON");
    assert_eq!(
        generated_contract
            .pointer("/steps/0/tool/kind")
            .and_then(serde_json::Value::as_str),
        Some("create_decision_pack")
    );
    assert!(generated_contract
        .pointer("/steps/0/tool/arguments")
        .is_none());
    validate_objective_coverage(TEST_FOUR_RESOLVED_OBJECTIVE, &compiled)
        .expect("the deterministic plan must pass the same strict coverage gate");

    let GeneratedToolDraft::RegisteredTaskTool {
        operation,
        arguments,
    } = &compiled.steps[0].tool
    else {
        panic!("first step must be the native decision-pack operation");
    };
    assert_eq!(operation, "create_decision_pack");
    assert_eq!(
        arguments
            .get("outputDirectory")
            .and_then(serde_json::Value::as_str),
        Some(output_directory)
    );
    assert_eq!(
        arguments
            .get("inputPaths")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert!(arguments
        .get("inputPaths")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|paths| paths.iter().all(|path| {
            path.as_str()
                .is_some_and(|path| Path::new(path).is_absolute() && path.contains("/mock_data/"))
        })));
    assert!(arguments.get("researchQueries").is_none());
    assert_eq!(
        arguments.pointer("/researchPolicy/requirement"),
        Some(&json!("anyOf"))
    );
    assert_eq!(
        arguments.pointer("/researchPolicy/minimumSatisfiedSubjects"),
        Some(&json!(1))
    );
    assert_eq!(
        arguments
            .pointer("/researchPolicy/subjects")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        compiled
            .steps
            .iter()
            .map(|step| match &step.tool {
                GeneratedToolDraft::RegisteredTaskTool { operation, .. } => operation.as_str(),
                _ => "unexpected",
            })
            .collect::<Vec<_>>(),
        vec![
            "create_decision_pack",
            "create_conflict_free_calendar_event",
            "draft_decision_pack_email",
        ]
    );
}

#[test]
fn test_eight_multiline_family_wording_compiles_without_model_json() {
    let output_directory =
        "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/ship_test_01";
    let compiled = compile_decision_pack(
        TEST_EIGHT_SIMPLIFIED_FAMILY_OBJECTIVE,
        Some(output_directory),
    )
    .expect("the harmlessly reformatted objective should compile")
    .expect("the complete cross-surface contract must take the deterministic route");

    assert!(matches!(compiled.source, IntentSource::Deterministic));
    assert_eq!(compiled.steps.len(), 3);
    let GeneratedToolDraft::RegisteredTaskTool {
        operation: decision_operation,
        arguments: decision,
    } = &compiled.steps[0].tool
    else {
        panic!("first step must be the deterministic decision-pack operation");
    };
    assert_eq!(decision_operation, "create_decision_pack");
    assert_eq!(
        decision
            .get("inputPaths")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        decision
            .get("outputDirectory")
            .and_then(serde_json::Value::as_str),
        Some(output_directory)
    );

    let GeneratedToolDraft::RegisteredTaskTool {
        operation: calendar_operation,
        arguments: calendar,
    } = &compiled.steps[1].tool
    else {
        panic!("second step must be the deterministic Calendar operation");
    };
    assert_eq!(calendar_operation, "create_conflict_free_calendar_event");
    assert_eq!(calendar["calendarName"], json!("Family"));
    assert_eq!(calendar["title"], json!("Supplier Decision Review"));

    let GeneratedToolDraft::RegisteredTaskTool {
        operation: mail_operation,
        arguments: mail,
    } = &compiled.steps[2].tool
    else {
        panic!("third step must be the deterministic Mail operation");
    };
    assert_eq!(mail_operation, "draft_decision_pack_email");
    assert_eq!(mail["subject"], json!("Supplier Decision Review"));
    validate_objective_coverage(TEST_EIGHT_SIMPLIFIED_FAMILY_OBJECTIVE, &compiled)
        .expect("the deterministic Test 8 plan must pass the strict coverage gate");
}

#[test]
fn path_leading_truncated_test_eight_still_uses_the_strong_deterministic_contract() {
    let objective = "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt from my testing folder. Reconcile every quoted amount and margin, identify all exceptions, and recommend a supplier. Research current fuel or freight conditions that could materially affect the recommendation using primary or official web sources. Cite every web claim with its URL and access time. Create a folder named ship_test_01 inside the testing folder and deliver supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md. Create a tentative 30-minute event titled Supplier Decision Review in my Family calendar on the next weekday between 1:00 PM and 4:00 PM without conflicts, and create an unsent Mail draft to recipient@example.com listing the four output files. Do not send the email.";
    let output_directory =
        "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/ship_test_01";

    let compiled = compile_decision_pack(objective, Some(output_directory))
        .expect("the strong contract should compile without its truncated opening line")
        .expect("the full structural signature must bypass provider planning");
    assert!(matches!(compiled.source, IntentSource::Deterministic));
    assert_eq!(compiled.steps.len(), 3);
    let GeneratedToolDraft::RegisteredTaskTool { arguments, .. } = &compiled.steps[0].tool else {
        panic!("first step must be create_decision_pack");
    };
    assert_eq!(
        arguments
            .get("inputPaths")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn incomplete_path_leading_prompt_does_not_gain_deterministic_authority() {
    let incomplete = "/Users/example/private/vendor.txt from my testing folder. Create supplier_decision.xlsx, supplier_decision.pptx, supplier_decision.pdf, and sources.md.";
    assert!(
        compile_decision_pack(incomplete, Some("/Users/example/testing/ship_test_01"))
            .expect("an incomplete objective is not a malformed contract")
            .is_none()
    );
}

#[test]
fn structured_policy_cannot_mask_an_unbounded_generic_search() {
    let output_directory =
        "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/ship_test_01";
    let mut compiled = compile_decision_pack(TEST_FOUR_RESOLVED_OBJECTIVE, Some(output_directory))
        .unwrap()
        .unwrap();
    compiled
        .steps
        .push(step(GeneratedToolDraft::SovereignDuckDuckGoSearch {
            query: "private supplier renewal quote 48291".to_string(),
            max_results: Some(5),
        }));
    assert!(!independent_public_searches_only(
        TEST_FOUR_RESOLVED_OBJECTIVE,
        &compiled
    ));
}

fn draft(steps: Vec<GeneratedPlanStepDraft>) -> GeneratedActionPlanDraft {
    GeneratedActionPlanDraft {
        steps,
        exit_condition: "Exit after every requested result is verified.".to_string(),
        generated_text: "synthetic plan".to_string(),
        source: IntentSource::Gemma,
        degraded_reason: None,
    }
}

fn step(tool: GeneratedToolDraft) -> GeneratedPlanStepDraft {
    GeneratedPlanStepDraft {
        step: "Execute the required action.".to_string(),
        tool,
        risk_level: GeneratedRiskLevel::High,
    }
}

fn create_file(path: &str, format: &str) -> GeneratedPlanStepDraft {
    step(GeneratedToolDraft::RegisteredTaskTool {
        operation: "create_file".to_string(),
        arguments: json!({"file":{
            "title":Path::new(path).file_stem().unwrap().to_string_lossy(),
            "content":"verified content",
            "locale":"en-US",
            "format":format,
            "destinationPath":path,
        }}),
    })
}

#[test]
fn exact_cross_surface_scenario_rejects_a_partial_planner_draft() {
    let partial = draft(vec![create_file(
        "ship_test_01/supplier_decision.xlsx",
        "xlsx",
    )]);

    let error = validate_objective_coverage(SCENARIO_OBJECTIVE, &partial)
        .expect_err("one output cannot cover a cross-surface objective");

    assert_eq!(error.code(), "planner_decision_pack_contract_invalid");
    assert!(error.missing[0].contains("exactly three ordered operations"));
}

#[test]
fn scenario_rejects_generic_calendar_and_mail_substitutions() {
    let calendar_arguments = json!({
        "calendarName":"OOMU Test",
        "title":"Supplier Decision Review",
        "startDate":"2026-07-20T13:00:00-04:00",
        "endDate":"2026-07-20T13:30:00-04:00",
        "location":"",
        "notes":"Review the verified supplier decision pack.",
        "availability":"tentative"
    });
    let mail_arguments = json!({
        "to":"test@example.invalid",
        "subject":"Supplier Decision Review",
        "body":"The verified supplier decision pack is ready for review."
    });
    assert!(
        crate::tools::system_calendar_event::validate_registration(calendar_arguments.clone())
            .is_ok()
    );
    assert!(crate::tools::system_mail::validate_registration(mail_arguments.clone()).is_ok());
    let complete = draft(vec![
        step(GeneratedToolDraft::FileRead {
            path: "/testing/mock_data/supplier_proposals.json".to_string(),
        }),
        step(GeneratedToolDraft::FileRead {
            path: "/testing/mock_data/q3_strategic_vendor_proposals.txt".to_string(),
        }),
        step(GeneratedToolDraft::SovereignDuckDuckGoSearch {
            query: "official fuel and freight conditions".to_string(),
            max_results: Some(5),
        }),
        create_file("/testing/ship_test_01/supplier_decision.xlsx", "xlsx"),
        create_file("/testing/ship_test_01/supplier_decision.pptx", "pptx"),
        create_file("/testing/ship_test_01/supplier_decision.pdf", "pdf"),
        create_file("/testing/ship_test_01/sources.md", "md"),
        step(GeneratedToolDraft::RegisteredTaskTool {
            operation: "create_system_calendar_event".to_string(),
            arguments: calendar_arguments,
        }),
        step(GeneratedToolDraft::RegisteredTaskTool {
            operation: "draft_system_email".to_string(),
            arguments: mail_arguments,
        }),
    ]);

    let error = validate_objective_coverage(SCENARIO_OBJECTIVE, &complete)
        .expect_err("generic file, Calendar, and Mail substitutes cannot satisfy Scenario 1");
    assert_eq!(error.code(), "planner_decision_pack_contract_invalid");
    assert!(error.message().contains(
        "create_decision_pack -> create_conflict_free_calendar_event -> draft_decision_pack_email"
    ));
}

fn evidence_bound_scenario_draft() -> GeneratedActionPlanDraft {
    let output_directory = "/testing/ship_test_01";
    let output_paths = [
        format!("{output_directory}/supplier_decision.xlsx"),
        format!("{output_directory}/supplier_decision.pptx"),
        format!("{output_directory}/supplier_decision.pdf"),
        format!("{output_directory}/sources.md"),
    ];
    draft(vec![
        step(GeneratedToolDraft::RegisteredTaskTool {
            operation: "create_decision_pack".to_string(),
            arguments: json!({
                "title":"Supplier Decision Pack",
                "locale":"en-US",
                "inputPaths":[
                    "/testing/mock_data/supplier_proposals.json",
                    "/testing/mock_data/q3_strategic_vendor_proposals.txt"
                ],
                "researchQueries":["official fuel conditions","official freight conditions"],
                "analysisInstructions":"Reconcile every quoted amount and margin and identify every exception.",
                "outputDirectory":output_directory,
                "outputs":{
                    "workbook":"supplier_decision.xlsx",
                    "presentation":"supplier_decision.pptx",
                    "pdf":"supplier_decision.pdf",
                    "sources":"sources.md"
                }
            }),
        }),
        step(GeneratedToolDraft::RegisteredTaskTool {
            operation: "create_conflict_free_calendar_event".to_string(),
            arguments: json!({
                "calendarName":"OOMU Test",
                "title":"Supplier Decision Review",
                "day":"next_weekday",
                "windowStartLocal":"13:00",
                "windowEndLocal":"16:00",
                "durationMinutes":30,
                "location":"",
                "notes":"Review the verified supplier decision pack.",
                "availability":"tentative"
            }),
        }),
        step(GeneratedToolDraft::RegisteredTaskTool {
            operation: "draft_decision_pack_email".to_string(),
            arguments: json!({
                "to":"test@example.invalid",
                "subject":"Supplier Decision Review",
                "expectedOutputPaths":output_paths
            }),
        }),
    ])
}

fn registered_arguments_mut(draft: &mut GeneratedActionPlanDraft, index: usize) -> &mut Value {
    match &mut draft.steps[index].tool {
        GeneratedToolDraft::RegisteredTaskTool { arguments, .. } => arguments,
        _ => panic!("Scenario step {index} is not a registered operation"),
    }
}

#[test]
fn evidence_bound_decision_pack_covers_the_exact_scenario_contract() {
    let complete = evidence_bound_scenario_draft();

    validate_objective_coverage(SCENARIO_OBJECTIVE, &complete)
        .expect("one evidence-bound build plus exact Calendar and Mail steps covers the objective");
    assert!(independent_public_searches_only(
        SCENARIO_OBJECTIVE,
        &complete
    ));
}

#[test]
fn internal_decision_pack_bindings_do_not_invalidate_public_coverage() {
    let root = std::env::temp_dir().join(format!(
        "oomu-plan-coverage-bindings-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let inputs = root.join("inputs");
    std::fs::create_dir(&inputs).unwrap();
    let supplier = inputs.join("rates.json");
    let vendor = inputs.join("margins.txt");
    std::fs::write(&supplier, "{}\n").unwrap();
    std::fs::write(&vendor, "margin\n").unwrap();
    let output = root.join("decision_output");
    let objective = format!(
        "Prepare a board-ready supplier decision pack. Read `{}` and `{}`. Reconcile every quoted amount and margin and identify all exceptions. Independently research current primary or official web sources for fuel or freight conditions. Create a new `{}` folder and deliver `decision.xlsx`, `decision.pptx`, `decision.pdf`, and `sources.md`. Then create a tentative 30-minute event in my `OOMU Test` calendar on the next weekday between 1:00 PM and 4:00 PM titled `Decision Review`, avoiding conflicts, and create a Mail draft to test@example.invalid listing the four output files.",
        supplier.display(),
        vendor.display(),
        output.display()
    );
    let validated_arguments = json!({
        "title":"Supplier Decision Pack",
        "locale":"en-US",
        "inputPaths":[supplier.to_string_lossy(), vendor.to_string_lossy()],
        "researchQueries":["official fuel conditions"],
        "analysisInstructions":"Reconcile every quoted amount and margin and identify every exception.",
        "outputDirectory":output.to_string_lossy(),
        "outputs":{
            "workbook":"decision.xlsx",
            "presentation":"decision.pptx",
            "pdf":"decision.pdf",
            "sources":"sources.md"
        },
        "inputBindings":[],
        "outputBinding":null
    });
    assert!(validated_arguments.get("inputBindings").is_some());
    assert!(validated_arguments.get("outputBinding").is_some());
    let mut complete = evidence_bound_scenario_draft();
    complete.steps[0].tool = GeneratedToolDraft::RegisteredTaskTool {
        operation: "create_decision_pack".to_string(),
        arguments: validated_arguments,
    };
    let output_paths = [
        "decision.xlsx",
        "decision.pptx",
        "decision.pdf",
        "sources.md",
    ]
    .map(|name| output.join(name).to_string_lossy().to_string());
    registered_arguments_mut(&mut complete, 1)["title"] = json!("Decision Review");
    registered_arguments_mut(&mut complete, 2)["to"] = json!("test@example.invalid");
    registered_arguments_mut(&mut complete, 2)["expectedOutputPaths"] = json!(output_paths);

    validate_objective_coverage(&objective, &complete)
        .expect("internal execution bindings cannot invalidate public planner coverage");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn absolute_scenario_paths_and_recipient_remain_exactly_bound() {
    let root = "/Users/example/Test Data";
    for quoted in [true, false] {
        let wrap = |path: String| if quoted { format!("`{path}`") } else { path };
        let objective = SCENARIO_OBJECTIVE
            .replace(
                "`mock_data/supplier_proposals.json`",
                &wrap(format!("{root}/mock_data/supplier_proposals.json")),
            )
            .replace(
                "`mock_data/q3_strategic_vendor_proposals.txt`",
                &wrap(format!(
                    "{root}/mock_data/q3_strategic_vendor_proposals.txt"
                )),
            )
            .replace("<TEST_RECIPIENT>", "recipient@example.com");
        let mut complete = evidence_bound_scenario_draft();
        let decision = registered_arguments_mut(&mut complete, 0);
        decision["inputPaths"] = json!([
            format!("{root}/mock_data/supplier_proposals.json"),
            format!("{root}/mock_data/q3_strategic_vendor_proposals.txt")
        ]);
        decision["outputDirectory"] = json!(format!("{root}/ship_test_01"));
        let output_paths = [
            format!("{root}/ship_test_01/supplier_decision.xlsx"),
            format!("{root}/ship_test_01/supplier_decision.pptx"),
            format!("{root}/ship_test_01/supplier_decision.pdf"),
            format!("{root}/ship_test_01/sources.md"),
        ];
        let mail = registered_arguments_mut(&mut complete, 2);
        mail["to"] = json!("recipient@example.com");
        mail["expectedOutputPaths"] = json!(output_paths);

        validate_objective_coverage(&objective, &complete)
            .unwrap_or_else(|error| panic!("quoted={quoted}: {}", error.message()));
    }
}

#[test]
fn scenario_rejects_reordered_cross_surface_operations() {
    let mut reordered = evidence_bound_scenario_draft();
    reordered.steps.swap(1, 2);

    let error = validate_objective_coverage(SCENARIO_OBJECTIVE, &reordered)
        .expect_err("Mail cannot run before conflict-free Calendar creation");

    assert_eq!(error.code(), "planner_decision_pack_contract_invalid");
    assert!(error.message().contains(
        "Step 2 must use 'create_conflict_free_calendar_event', not 'draft_decision_pack_email'"
    ));
}

#[test]
fn scenario_rejects_wrong_calendar_and_mail_arguments() {
    let mut wrong_calendar = evidence_bound_scenario_draft();
    registered_arguments_mut(&mut wrong_calendar, 1)["durationMinutes"] = json!(45);
    let error = validate_objective_coverage(SCENARIO_OBJECTIVE, &wrong_calendar)
        .expect_err("the Calendar duration is literal");
    assert_eq!(error.code(), "planner_decision_pack_contract_invalid");
    assert!(error.message().contains("durationMinutes must be 30"));

    let mut wrong_mail = evidence_bound_scenario_draft();
    registered_arguments_mut(&mut wrong_mail, 2)["expectedOutputPaths"][0] =
        json!("/testing/ship_test_01/other.xlsx");
    let error = validate_objective_coverage(SCENARIO_OBJECTIVE, &wrong_mail)
        .expect_err("Mail must bind the exact verified outputs");
    assert_eq!(error.code(), "planner_decision_pack_contract_invalid");
    assert!(error.message().contains("expectedOutputPaths"));

    let objective = SCENARIO_OBJECTIVE.replace("<TEST_RECIPIENT>", "recipient@example.com");
    let mut wrong_recipient = evidence_bound_scenario_draft();
    registered_arguments_mut(&mut wrong_recipient, 2)["to"] = json!("other@example.com");
    let error = validate_objective_coverage(&objective, &wrong_recipient)
        .expect_err("Mail must use the exact requested recipient");
    assert!(error.message().contains("recipient@example.com"));
}

#[test]
fn scenario_input_path_drift_has_an_actionable_typed_error() {
    let mut wrong = evidence_bound_scenario_draft();
    registered_arguments_mut(&mut wrong, 0)["inputPaths"][0] =
        json!("/testing/mock_data/other_supplier_proposals.json");

    let error =
        super::super::validate_planner_draft_for_execution(SCENARIO_OBJECTIVE, &wrong, true)
            .expect_err("the planner cannot change a requested source path");

    assert_eq!(error.code, "planner_decision_pack_input_path_invalid");
    assert!(error.message.contains("mock_data/supplier_proposals.json"));
    assert!(error.message.contains("Check the path spelling"));
    assert!(error.message.ends_with("No action was executed."));
}

#[test]
fn an_ungrounded_typo_cannot_be_silently_substituted_by_the_planner() {
    for typo in [
        SCENARIO_OBJECTIVE.replace("mock_data/", "mocked_data/"),
        SCENARIO_OBJECTIVE.replace(
            "`mock_data/supplier_proposals.json`",
            "/Users/example/Test Data/mocked_data/supplier_proposals.json",
        ),
    ] {
        let error = super::super::validate_planner_draft_for_execution(
            &typo,
            &evidence_bound_scenario_draft(),
            true,
        )
        .expect_err("an unresolved input path cannot be silently corrected by a planner");

        assert_eq!(error.code, "planner_decision_pack_input_path_invalid");
        assert!(error.message.contains("changed or omitted"));
        assert!(error.message.contains("Check the path spelling"));
    }
}

#[test]
fn format_only_requests_accept_native_artifact_tools() {
    let objective = "Create a spreadsheet and a presentation for the decision review.";
    let complete = draft(vec![
        step(GeneratedToolDraft::RegisteredTaskTool {
            operation: "create_spreadsheet".to_string(),
            arguments: json!({"workbook":{"title":"Decision Review"}}),
        }),
        step(GeneratedToolDraft::RegisteredTaskTool {
            operation: "create_presentation".to_string(),
            arguments: json!({"brief":{"title":"Decision Review"}}),
        }),
    ]);

    validate_objective_coverage(objective, &complete)
        .expect("native format-specific tools are semantically equivalent");
}

#[test]
fn named_output_directory_rejects_a_different_destination() {
    let objective = "Create a new `final` folder and deliver `report.pdf`.";
    let wrong = draft(vec![create_file("draft/report.pdf", "pdf")]);

    let error = validate_objective_coverage(objective, &wrong)
        .expect_err("a different output directory is not equivalent");

    assert_eq!(error.missing, vec!["output file 'final/report.pdf'"]);
}

#[test]
fn execution_validator_returns_a_distinct_honest_scenario_contract_error() {
    let partial = draft(vec![create_file(
        "ship_test_01/supplier_decision.xlsx",
        "xlsx",
    )]);

    let error =
        super::super::validate_planner_draft_for_execution(SCENARIO_OBJECTIVE, &partial, true)
            .expect_err("an incomplete objective cannot reach plan signing");

    assert_eq!(error.code, "planner_decision_pack_contract_invalid");
    assert_eq!(error.boundary, "AgentPlanning");
    assert!(error.message.contains("exactly three ordered operations"));
    assert!(error
        .message
        .contains("create_conflict_free_calendar_event"));
    assert!(error.message.ends_with("No action was executed."));
}

#[test]
fn authoritative_search_intent_is_identical_for_coverage_and_authorization() {
    let browse = "Browse the web for freight conditions and create `report.md`.";
    let missing_search = draft(vec![create_file("report.md", "md")]);
    let error = validate_objective_coverage(browse, &missing_search)
        .expect_err("an authorized browse directive must require a search step");
    assert!(error.missing.contains(&"external web research".to_string()));

    let negated = "Read `input.json`; do not browse the web; do not create a Calendar event; do not create a Mail draft.";
    let read_only = draft(vec![step(GeneratedToolDraft::FileRead {
        path: "input.json".to_string(),
    })]);
    validate_objective_coverage(negated, &read_only)
        .expect("negated cross-surface actions are not requirements");
}

#[test]
fn questions_and_negated_file_actions_do_not_become_requirements() {
    let objective = "Read `input.json`. Should I create `report.pdf`, a Calendar event, or a Mail draft? Do not create `ignored.pdf`.";
    let read_only = draft(vec![step(GeneratedToolDraft::FileRead {
        path: "input.json".to_string(),
    })]);
    validate_objective_coverage(objective, &read_only)
        .expect("questions and explicit negations cannot authorize actions");
}

#[test]
fn common_input_output_wording_is_classified_by_role() {
    for objective in [
        "Using `input.json`, create `output.pdf`.",
        "Convert `input.csv` to `output.xlsx`.",
        "Create `output.pdf` from `input.json`.",
    ] {
        let input = if objective.contains("csv") {
            "input.csv"
        } else {
            "input.json"
        };
        let output = if objective.contains("xlsx") {
            ("output.xlsx", "xlsx")
        } else {
            ("output.pdf", "pdf")
        };
        let complete = draft(vec![
            step(GeneratedToolDraft::FileRead {
                path: input.to_string(),
            }),
            create_file(output.0, output.1),
        ]);
        validate_objective_coverage(objective, &complete)
            .unwrap_or_else(|error| panic!("{objective}: {:?}", error.missing));
    }
}

#[test]
fn directory_inputs_require_a_real_file_list_step() {
    let objective =
        "Read every file in `/Users/example/Test Data/mock_data` and summarize the set.";
    let missing = validate_objective_coverage(objective, &draft(vec![]))
        .expect_err("a directory input cannot be silently omitted");
    assert_eq!(
        missing.missing,
        vec!["input directory listing '/Users/example/Test Data/mock_data'"]
    );
    validate_objective_coverage(
        objective,
        &draft(vec![step(GeneratedToolDraft::FileList {
            path: "/Users/example/Test Data/mock_data".to_string(),
        })]),
    )
    .expect("FileList is the production directory discovery operation");
}

#[test]
fn named_outputs_require_real_paths_and_binary_producers() {
    let named = "Create `report.xlsx` and `brief.pdf`.";
    let pathless_or_text = draft(vec![
        step(GeneratedToolDraft::RegisteredTaskTool {
            operation: "create_spreadsheet".to_string(),
            arguments: json!({"workbook":{"title":"report"}}),
        }),
        step(GeneratedToolDraft::FileWrite {
            path: "brief.pdf".to_string(),
            content: "not a PDF".to_string(),
        }),
    ]);
    let error = validate_objective_coverage(named, &pathless_or_text)
        .expect_err("pathless artifacts and raw binary writes cannot cover named files");
    assert!(error
        .missing
        .contains(&"output file 'report.xlsx'".to_string()));
    assert!(error
        .missing
        .contains(&"output file 'brief.pdf'".to_string()));

    let archive_as_pdf = draft(vec![step(GeneratedToolDraft::TelemetryArchive {
        output_path: "brief.pdf".to_string(),
    })]);
    assert!(validate_objective_coverage("Create `brief.pdf`.", &archive_as_pdf).is_err());
}

#[test]
fn unicode_before_plain_paths_is_safe_and_absolute_paths_remain_exact() {
    let objective = "Read ééééééé report.pdf.";
    validate_objective_coverage(
        objective,
        &draft(vec![step(GeneratedToolDraft::FileRead {
            path: "report.pdf".to_string(),
        })]),
    )
    .expect("UTF-8 text before a plain filename cannot panic or lose coverage");

    let absolute = "Read `/approved/root/report.pdf`.";
    let wrong_root = draft(vec![step(GeneratedToolDraft::FileRead {
        path: "/other/root/report.pdf".to_string(),
    })]);
    assert!(validate_objective_coverage(absolute, &wrong_root).is_err());
}

#[test]
fn private_tmp_output_paths_keep_their_root_and_require_an_exact_candidate() {
    let destination = "/private/tmp/oomu-artifact-hotfix/output/hello_world.pdf";
    let objective = format!("Create the PDF file at {destination} containing ‘Hello World’. ");

    assert_eq!(normalize_path(destination), destination);
    validate_objective_coverage(&objective, &draft(vec![create_file(destination, "pdf")]))
        .expect("an exact /private/tmp output must satisfy strict coverage");

    let truncated = "/tmp/oomu-artifact-hotfix/output/hello_world.pdf";
    let error =
        validate_objective_coverage(&objective, &draft(vec![create_file(truncated, "pdf")]))
            .expect_err("dropping the /private root must remain a coverage failure");
    assert_eq!(error.missing, vec![format!("output file '{destination}'")]);
}

#[test]
fn escaped_icloud_input_is_one_canonical_requirement_and_path_fragments_cannot_cover_it() {
    let canonical = "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt";
    let objective = r"Read /Users/example/Library/Mobile\ Documents/com\~apple\~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt in my testing folder and summarize only the stated facts in exactly three bullets.";

    assert_eq!(
        objective_input_file_references(objective)
            .into_iter()
            .map(|reference| reference.path)
            .collect::<Vec<_>>(),
        vec![canonical]
    );
    validate_objective_coverage(
        objective,
        &draft(vec![step(GeneratedToolDraft::FileRead {
            path: canonical.to_string(),
        })]),
    )
    .expect("the canonical file identity must cover the escaped spelling");

    for malformed in [
        "/Users/example/Library/Mobile",
        "Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt",
        "OOMU Test Data/mock_data/q3_strategic_vendor_proposals.txt",
    ] {
        assert!(
            validate_objective_coverage(
                objective,
                &draft(vec![step(GeneratedToolDraft::FileRead {
                    path: malformed.to_string(),
                })]),
            )
            .is_err(),
            "malformed planner path unexpectedly covered the canonical input: {malformed}"
        );
    }
}

#[test]
fn escaped_icloud_directory_is_one_explicit_listing_requirement() {
    let canonical =
        "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data";
    let objective = r"Read /Users/example/Library/Mobile\ Documents/com\~apple\~CloudDocs/OOMU Test Data/mock_data in my testing folder and summarize only the stated facts.";

    assert_eq!(
        objective_input_directory_references(objective)
            .into_iter()
            .map(|reference| reference.path)
            .collect::<Vec<_>>(),
        vec![canonical]
    );
    validate_objective_coverage(
        objective,
        &draft(vec![step(GeneratedToolDraft::FileList {
            path: canonical.to_string(),
        })]),
    )
    .expect("a directory stays explicit and requires the exact listing operation");
}

#[test]
fn absolute_directory_clause_does_not_consume_a_later_relative_output_file() {
    let objective = "List /Users/example/Desktop and write report.md.";

    assert_eq!(
        objective_input_directory_references(objective)
            .into_iter()
            .map(|reference| reference.path)
            .collect::<Vec<_>>(),
        vec!["/Users/example/Desktop"]
    );
    assert!(objective_input_file_references(objective).is_empty());
    assert_eq!(
        objective_output_file_references(objective)
            .into_iter()
            .map(|reference| reference.path)
            .collect::<Vec<_>>(),
        vec!["report.md"]
    );
}

#[test]
fn private_data_substitution_stays_blocked_while_independent_public_research_is_allowed() {
    let objective = "Read my unread emails and independently research official web sources for public fuel conditions.";
    let public = draft(vec![step(GeneratedToolDraft::SovereignDuckDuckGoSearch {
        query: "public fuel conditions".to_string(),
        max_results: Some(5),
    })]);
    assert!(independent_public_searches_only(objective, &public));

    let substituted = draft(vec![step(GeneratedToolDraft::SovereignDuckDuckGoSearch {
        query: "Acme renewal quote 48291".to_string(),
        max_results: Some(5),
    })]);
    let error = super::super::validate_planner_draft_for_execution(objective, &substituted, true)
        .expect_err("private content cannot become a web query");
    assert_eq!(error.code, "private_app_web_fallback_blocked");
}
