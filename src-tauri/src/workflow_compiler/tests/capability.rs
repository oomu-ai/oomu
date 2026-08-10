use super::*;

#[test]
fn workflow_catalog_only_admits_tools_the_runtime_can_execute() {
    assert!(workflow_runtime_supports_mcp_tool(
        "macos_applescript",
        "read_system_emails"
    ));
    assert!(!workflow_runtime_supports_mcp_tool(
        "macos_applescript",
        "read_system_music"
    ));
    assert!(!workflow_runtime_supports_mcp_tool(
        "macos_applescript",
        "send_system_email"
    ));
}

const SCENARIO_FIVE_PROMPT: &str = "At each run, read `/Users/example/Library/Mobile\\ Documents/com\\~apple\\~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json` and `/Users/example/Library/Mobile\\ Documents/com\\~apple\\~CloudDocs/OOMU Test Data/mock_data/project_milestones.json` from the testing folder. Retrieve current information from at least two relevant primary or official public web sources, including one current energy/fuel source and one transport, logistics, or government source. Record each URL and access time. Reconcile supplier rate variances, identify unfinished milestones, and explain only evidence-backed changes since the local fixture dates. Create `/Users/example/Library/Mobile\\ Documents/com\\~apple\\~CloudDocs/OOMU Test Data/mock_data/ship_test_05/operations_brief_<YYYY-MM-DD_HH-mm>.md` and a matching PDF. Both must include a one-paragraph executive summary, data table, exceptions, milestone risks, current web evidence, source links, and next actions. Read both files back or validate them before completion. Deliver a concise summary to the Routine's configured channel with the two exact filenames and a truthful success/failure status. Never report a file as created unless it exists.";

const SCENARIO_SIX_PROMPT: &str = "Read `/Users/example/Library/Mobile\\ Documents/com\\~apple\\~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json`. Retrieve one current primary or official public source relevant to US freight or fuel conditions. Create `ship_test_06/supplier_exception_<YYYY-MM-DD_HH-mm>.md` containing the local variances, live source URL/access time, risk assessment, and next actions. If any supplier's active quote exceeds its historical settled rate, create one 30-minute event titled `Supplier Exception Follow-up` in the `OOMU Test` calendar on the next conflict-free weekday at 2:00 PM or later, and send one email to `recipient@example.com` with subject `OOMU Test — Supplier Exception` and the report attached or linked. These Calendar and send actions require explicit user approval. If approval is pending, preserve the run and resume from that exact step after approval. Never create duplicate events, messages, reports, or deliveries when retrying or recovering. Finally, deliver the run result and exact report filename to the configured private channel.";
const LAB_AUDIT_PROMPT: &str = "Create a recurring daily scheduled workflow named \"Lab Inventory & Maintenance Audit\" that runs every morning at 8:00 AM. It should inspect Maintenance_Tickets.csv and Lab_Inventory.csv in \"/Users/jeffreyallan/Documents/OOMU/Projects/mock_data\", flag open critical tickets or depleted inventory, and generate a daily operational digest.";

#[test]
fn quoted_folder_and_plain_file_names_bind_two_exact_project_inputs() {
    assert_eq!(
        registered_task_capabilities::requested_local_input_paths(LAB_AUDIT_PROMPT),
        vec![
            "/Users/jeffreyallan/Documents/OOMU/Projects/mock_data/Maintenance_Tickets.csv",
            "/Users/jeffreyallan/Documents/OOMU/Projects/mock_data/Lab_Inventory.csv",
        ]
    );

    let complete: WorkflowIr = serde_json::from_value(json!({
        "schemaVersion":"1.0.0","workflowId":"lab-audit","workflowVersion":1,
        "name":"Lab Inventory & Maintenance Audit","description":"Daily operational digest.",
        "compiler":{"model":"gemma-4-e2b-qat"},
        "nodes":[
            {"kind":"input","id":"input","label":"Run input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
            {"kind":"mcp_tool","id":"read-tickets","label":"Read maintenance tickets","serverName":"oomu_task_tools","toolName":"read_project_file","arguments":{"path":"/Users/jeffreyallan/Documents/OOMU/Projects/mock_data/Maintenance_Tickets.csv"}},
            {"kind":"mcp_tool","id":"read-inventory","label":"Read lab inventory","serverName":"oomu_task_tools","toolName":"read_project_file","arguments":{"path":"/Users/jeffreyallan/Documents/OOMU/Projects/mock_data/Lab_Inventory.csv"}},
            {"kind":"agent","id":"digest","label":"Prepare digest","objective":"Flag critical tickets and depleted inventory.","inputMappings":{"tickets":"{{nodes.read-tickets.output.data.content}}","inventory":"{{nodes.read-inventory.output.data.content}}"},"outputKey":"nodes.digest.output"},
            {"kind":"output","id":"output","label":"Daily operational digest","inputMapping":"{{nodes.digest.output}}","outputSchema":{"type":"object"}}
        ],
        "edges":[
            {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read-tickets"},
            {"id":"e2","sourceNodeId":"read-tickets","sourcePort":"out","targetNodeId":"read-inventory"},
            {"id":"e3","sourceNodeId":"read-inventory","sourcePort":"out","targetNodeId":"digest"},
            {"id":"e4","sourceNodeId":"digest","sourcePort":"out","targetNodeId":"output"}
        ]
    }))
    .expect("valid lab audit workflow");
    registered_task_capabilities::validate_objective_bindings(LAB_AUDIT_PROMPT, &complete)
        .expect("both exact Project inputs are bound");

    let mut incomplete = complete;
    incomplete
        .nodes
        .retain(|node| node.id() != "read-inventory");
    let error =
        registered_task_capabilities::validate_objective_bindings(LAB_AUDIT_PROMPT, &incomplete)
            .expect_err("omitting either exact input must fail composition validation");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");
}

#[test]
fn consolidated_scenario_five_catalog_and_topology_bind_real_verified_capabilities() {
    registered_task_capabilities::register_test_tools();
    let actions = registered_task_capabilities::catalog_actions().expect("registered actions");
    let operations = actions
        .iter()
        .filter_map(|action| action.tool_name.as_deref())
        .collect::<HashSet<_>>();
    assert!(operations.contains("create_file"));
    assert!(operations.contains("read_project_file"));
    assert!(operations.contains("fetch_official_page"));
    assert!(operations.contains("analyze_supplier_exceptions"));
    assert!(operations.contains("analyze_project_milestones"));
    let create_file = actions
        .iter()
        .find(|action| action.tool_name.as_deref() == Some("create_file"))
        .expect("create_file capability");
    let formats = create_file.input_schema.as_ref().expect("schema")["properties"]["file"]
        ["properties"]["format"]["enum"]
        .as_array()
        .expect("formats");
    assert!(formats.contains(&json!("md")) && formats.contains(&json!("pdf")));
    let milestone = actions
        .iter()
        .find(|action| action.tool_name.as_deref() == Some("analyze_project_milestones"))
        .expect("milestone analysis capability");
    let milestone_fields = milestone.output_schema.as_ref().expect("output schema")["properties"]
        ["milestones"]["items"]["required"]
        .as_array()
        .expect("typed milestone fields");
    assert!(milestone_fields.contains(&json!("targetDate")));
    assert!(milestone_fields.contains(&json!("unfinished")));
    let validator = actions
        .iter()
        .find(|action| action.tool_name.as_deref() == Some("validate_evidence_report"))
        .expect("evidence report validation capability");
    assert!(validator
        .input_schema
        .as_ref()
        .expect("validator input schema")["required"]
        .as_array()
        .expect("validator required inputs")
        .contains(&json!("officialPageReceipts")));

    let ir: WorkflowIr = serde_json::from_value(json!({
        "schemaVersion":"1.0.0","workflowId":"scenario-five","workflowVersion":1,
        "name":"Scenario five","description":"Verified recurring operations brief.",
        "compiler":{"model":"gemma-4-e2b-qat"},
        "nodes":[
            {"kind":"input","id":"input","label":"Run input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
            {"kind":"mcp_tool","id":"read-suppliers","label":"Read exact supplier fixture","serverName":"oomu_task_tools","toolName":"read_project_file","arguments":{"path":"/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json"}},
            {"kind":"mcp_tool","id":"analyze-suppliers","label":"Calculate typed supplier variances","serverName":"oomu_task_tools","toolName":"analyze_supplier_exceptions","arguments":{"content":"{{nodes.read-suppliers.output.data.content}}"}},
            {"kind":"mcp_tool","id":"read-milestones","label":"Read exact milestone fixture","serverName":"oomu_task_tools","toolName":"read_project_file","arguments":{"path":"/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/project_milestones.json"}},
            {"kind":"mcp_tool","id":"analyze-milestones","label":"Calculate typed unfinished milestones","serverName":"oomu_task_tools","toolName":"analyze_project_milestones","arguments":{"content":"{{nodes.read-milestones.output.data.content}}"}},
            {"kind":"mcp_tool","id":"fuel-source","label":"Read official fuel source","serverName":"oomu_task_tools","toolName":"fetch_official_page","arguments":{"url":"https://www.eia.gov/petroleum/gasdiesel/","maxContentChars":3000}},
            {"kind":"mcp_tool","id":"transport-source","label":"Read official transport source","serverName":"oomu_task_tools","toolName":"fetch_official_page","arguments":{"url":"https://www.bts.gov/","maxContentChars":3000}},
            {"kind":"agent","id":"brief","label":"Prepare evidence-bound brief","objective":"Reconcile the typed supplier and milestone analyses with bounded official-source receipts.","inputMappings":{"supplierAnalysis":"{{nodes.analyze-suppliers.output.data}}","milestoneAnalysis":"{{nodes.analyze-milestones.output.data}}","fuelEvidence":"{{nodes.fuel-source.output.data}}","transportEvidence":"{{nodes.transport-source.output.data}}"},"outputKey":"nodes.brief.output"},
            {"kind":"mcp_tool","id":"validate-brief","label":"Validate evidence-bound brief","serverName":"oomu_task_tools","toolName":"validate_evidence_report","arguments":{"content":"{{nodes.brief.output.data}}","supplierAnalysis":"{{nodes.analyze-suppliers.output.data}}","milestoneAnalysis":"{{nodes.analyze-milestones.output.data}}","officialPageReceipts":["{{nodes.fuel-source.output.data}}","{{nodes.transport-source.output.data}}"],"requiredSections":["Executive summary","Supplier data","Exceptions","Milestone risks","Current evidence","Sources","Next actions"]}},
            {"kind":"mcp_tool","id":"write-md","label":"Create verified Markdown","serverName":"oomu_task_tools","toolName":"create_file","arguments":{"file":{"title":"Operations brief","content":"{{nodes.validate-brief.output.data.content}}","locale":"en-US","format":"md","destinationPath":"/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/ship_test_05/operations_brief_<YYYY-MM-DD_HH-mm>.md"}}},
            {"kind":"mcp_tool","id":"write-pdf","label":"Create verified PDF","serverName":"oomu_task_tools","toolName":"create_file","arguments":{"file":{"title":"Operations brief","content":"{{nodes.validate-brief.output.data.content}}","locale":"en-US","format":"pdf","destinationPath":"/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/ship_test_05/operations_brief_<YYYY-MM-DD_HH-mm>.pdf"}}},
            {"kind":"output","id":"output","label":"Verified result","inputMapping":"{{nodes.write-pdf.output}}","outputSchema":{"type":"object"}}
        ],
        "edges":[
            {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read-suppliers"},
            {"id":"e2","sourceNodeId":"read-suppliers","sourcePort":"out","targetNodeId":"analyze-suppliers"},
            {"id":"e3","sourceNodeId":"analyze-suppliers","sourcePort":"out","targetNodeId":"read-milestones"},
            {"id":"e4","sourceNodeId":"read-milestones","sourcePort":"out","targetNodeId":"analyze-milestones"},
            {"id":"e5","sourceNodeId":"analyze-milestones","sourcePort":"out","targetNodeId":"fuel-source"},
            {"id":"e6","sourceNodeId":"fuel-source","sourcePort":"out","targetNodeId":"transport-source"},
            {"id":"e7","sourceNodeId":"transport-source","sourcePort":"out","targetNodeId":"brief"},
            {"id":"e8","sourceNodeId":"brief","sourcePort":"out","targetNodeId":"validate-brief"},
            {"id":"e9","sourceNodeId":"validate-brief","sourcePort":"out","targetNodeId":"write-md"},
            {"id":"e10","sourceNodeId":"write-md","sourcePort":"out","targetNodeId":"write-pdf"},
            {"id":"e11","sourceNodeId":"write-pdf","sourcePort":"out","targetNodeId":"output"}
        ]
    })).expect("scenario five IR");
    ir.validate().expect("valid scenario five graph");
    validate_workflow_ir_topology(&ir).expect("safe scenario five topology");
    registered_task_capabilities::validate_objective_bindings(SCENARIO_FIVE_PROMPT, &ir)
        .expect("scenario five bindings");

    let mut duplicate_input = ir.clone();
    mcp_tool_mut(&mut duplicate_input, "read-milestones").arguments["path"] = json!(
        "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json"
    );
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_FIVE_PROMPT,
        &duplicate_input,
    )
    .expect_err("both exact fixture reads are required");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut unbound_milestones = ir.clone();
    mcp_tool_mut(&mut unbound_milestones, "analyze-milestones").arguments["content"] =
        json!("{{nodes.read-suppliers.output.data.content}}");
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_FIVE_PROMPT,
        &unbound_milestones,
    )
    .expect_err("milestone analysis must consume the exact milestone read");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");
    assert!(validate_workflow_ir_topology(&unbound_milestones).is_err());

    let mut oversized_source = ir.clone();
    mcp_tool_mut(&mut oversized_source, "fuel-source").arguments["maxContentChars"] = json!(50_000);
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_FIVE_PROMPT,
        &oversized_source,
    )
    .expect_err("unbounded official-page evidence must be rejected before save");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");
    assert!(validate_workflow_ir_topology(&oversized_source).is_err());

    let mut duplicate_source = ir.clone();
    mcp_tool_mut(&mut duplicate_source, "transport-source").arguments["url"] =
        json!("https://www.eia.gov/petroleum/gasdiesel/");
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_FIVE_PROMPT,
        &duplicate_source,
    )
    .expect_err("two nodes fetching one URL are not two official sources");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");
    assert!(validate_workflow_ir_topology(&duplicate_source).is_err());

    let mut raw_evidence = ir.clone();
    agent_mut(&mut raw_evidence, "brief").input_mappings.insert(
        "rawMilestones".to_string(),
        "{{nodes.read-milestones.output}}".to_string(),
    );
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_FIVE_PROMPT,
        &raw_evidence,
    )
    .expect_err("the synthesis Agent cannot receive raw or extra evidence");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut envelope_content = ir.clone();
    mcp_tool_mut(&mut envelope_content, "write-md").arguments["file"]["content"] =
        json!("{{nodes.brief.output.data}}");
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_FIVE_PROMPT,
        &envelope_content,
    )
    .expect_err("create_file must receive only the validator's exact verified content");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut wrong_destination = ir.clone();
    mcp_tool_mut(&mut wrong_destination, "write-md").arguments["file"]["destinationPath"] =
        json!("/Users/example/ship_test_05/operations_brief_<YYYY-MM-DD_HH-mm>.md");
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_FIVE_PROMPT,
        &wrong_destination,
    )
    .expect_err("the exact requested artifact destination is required");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut write_before_evidence = ir.clone();
    edge_mut(&mut write_before_evidence, "e7").target_node_id = "output".to_string();
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_FIVE_PROMPT,
        &write_before_evidence,
    )
    .expect_err("every requested read and official fetch must precede synthesis and writes");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");
}

#[test]
fn consolidated_scenario_six_requires_real_send_and_truthful_denial_branches() {
    let ir = scenario_six_ir("send_system_email", "branch", true);
    ir.validate().expect("valid scenario six graph");
    validate_workflow_ir_topology(&ir).expect("safe scenario six topology");
    registered_task_capabilities::validate_objective_bindings(SCENARIO_SIX_PROMPT, &ir)
        .expect("scenario six bindings");

    let mut prose_branch = ir.clone();
    let WorkflowNode::Conditional(conditional) = prose_branch
        .nodes
        .iter_mut()
        .find(|node| node.id() == "has-exception")
        .expect("supplier conditional")
    else {
        panic!("expected supplier conditional");
    };
    conditional.condition = "$.activeQuote > $.historicalSettledRate".to_string();
    conditional.input_mapping = Some("{{nodes.assess.output}}".to_string());
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &prose_branch,
    )
    .expect_err("Agent prose cannot drive an effectful supplier exception branch");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut fabricated_analysis = ir.clone();
    mcp_tool_mut(&mut fabricated_analysis, "analyze-suppliers").arguments["content"] =
        json!("{\"suppliers\":[]}");
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &fabricated_analysis,
    )
    .expect_err("supplier analysis must consume the exact approved read bytes");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut wrong_input = ir.clone();
    mcp_tool_mut(&mut wrong_input, "read-suppliers").arguments["path"] =
        json!("supplier_proposals.json");
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &wrong_input,
    )
    .expect_err("the exact supplier fixture read is required");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut wrong_destination = ir.clone();
    mcp_tool_mut(&mut wrong_destination, "write-report").arguments["file"]["destinationPath"] =
        json!("/Users/example/ship_test_06/supplier_exception_<YYYY-MM-DD_HH-mm>.md");
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &wrong_destination,
    )
    .expect_err("Project-relative output destination must remain exact");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut analysis_before_read = ir.clone();
    edge_mut(&mut analysis_before_read, "e2").target_node_id = "source".to_string();
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &analysis_before_read,
    )
    .expect_err("typed supplier analysis must follow the real Project read");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut effects_before_report = ir.clone();
    edge_mut(&mut effects_before_report, "e6").source_node_id = "assess".to_string();
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &effects_before_report,
    )
    .expect_err("the verified report must precede decisions, effects, and delivery");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut envelope_content = ir.clone();
    mcp_tool_mut(&mut envelope_content, "write-report").arguments["file"]["content"] =
        json!("{{nodes.assess.output}}");
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &envelope_content,
    )
    .expect_err("the supplier report must receive Agent text");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut oversized_source = ir.clone();
    mcp_tool_mut(&mut oversized_source, "source").arguments["maxContentChars"] = json!(50_000);
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &oversized_source,
    )
    .expect_err("Scenario six official evidence must remain bounded");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut source_envelope = ir.clone();
    agent_mut(&mut source_envelope, "assess")
        .input_mappings
        .insert("source".to_string(), "{{nodes.source.output}}".to_string());
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &source_envelope,
    )
    .expect_err("the Agent must receive the exact bounded source receipt");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut legacy_report_path = ir.clone();
    mcp_tool_mut(&mut legacy_report_path, "calendar").arguments["notes"] =
        json!("Report: {{nodes.write-report.output.path}}");
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &legacy_report_path,
    )
    .expect_err("new supplier workflows must use the canonical verified path");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let mut wrong_calendar = ir.clone();
    mcp_tool_mut(&mut wrong_calendar, "calendar").arguments["calendarName"] = json!("Personal");
    registered_task_capabilities::validate_objective_bindings(SCENARIO_SIX_PROMPT, &wrong_calendar)
        .expect_err("Calendar arguments must preserve the exact request");

    let mut wrong_recipient = ir.clone();
    mcp_tool_mut(&mut wrong_recipient, "send").arguments["to"] = json!("other@example.com");
    registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &wrong_recipient,
    )
    .expect_err("mail arguments must preserve the exact request");

    let mut missing_attachment = ir.clone();
    mcp_tool_mut(&mut missing_attachment, "send")
        .arguments
        .as_object_mut()
        .expect("send arguments")
        .remove("attachmentPath");
    registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &missing_attachment,
    )
    .expect_err("the attached report must use the verified receipt path");

    let draft = scenario_six_ir("draft_system_email", "branch", true);
    let error =
        registered_task_capabilities::validate_objective_bindings(SCENARIO_SIX_PROMPT, &draft)
            .expect_err("a draft cannot satisfy a send objective");
    assert_eq!(error.code, "workflow_objective_capability_mismatch");

    let missing_denial = scenario_six_ir("send_system_email", "fail", false);
    let error = registered_task_capabilities::validate_objective_bindings(
        SCENARIO_SIX_PROMPT,
        &missing_denial,
    )
    .expect_err("explicit effects require denial continuation");
    assert_eq!(error.code, "workflow_effect_missing_denial_continuation");
}

fn scenario_six_ir(send_tool: &str, send_on_denied: &str, send_denied_edge: bool) -> WorkflowIr {
    let mut edges = vec![
        json!({"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read-suppliers"}),
        json!({"id":"e2","sourceNodeId":"read-suppliers","sourcePort":"out","targetNodeId":"analyze-suppliers"}),
        json!({"id":"e3","sourceNodeId":"analyze-suppliers","sourcePort":"out","targetNodeId":"source"}),
        json!({"id":"e4","sourceNodeId":"source","sourcePort":"out","targetNodeId":"assess"}),
        json!({"id":"e5","sourceNodeId":"assess","sourcePort":"out","targetNodeId":"validate-report"}),
        json!({"id":"e6","sourceNodeId":"validate-report","sourcePort":"out","targetNodeId":"write-report"}),
        json!({"id":"e7","sourceNodeId":"write-report","sourcePort":"out","targetNodeId":"has-exception"}),
        json!({"id":"e8","sourceNodeId":"has-exception","sourcePort":"false","targetNodeId":"no-exception"}),
        json!({"id":"e9","sourceNodeId":"has-exception","sourcePort":"true","targetNodeId":"approve-calendar"}),
        json!({"id":"e10","sourceNodeId":"approve-calendar","sourcePort":"denied","targetNodeId":"calendar-denied"}),
        json!({"id":"e11","sourceNodeId":"approve-calendar","sourcePort":"approved","targetNodeId":"calendar"}),
        json!({"id":"e12","sourceNodeId":"calendar","sourcePort":"out","targetNodeId":"approve-send"}),
        json!({"id":"e13","sourceNodeId":"approve-send","sourcePort":"approved","targetNodeId":"send"}),
        json!({"id":"e14","sourceNodeId":"send","sourcePort":"out","targetNodeId":"output"}),
    ];
    if send_denied_edge {
        edges.push(json!({"id":"e15","sourceNodeId":"approve-send","sourcePort":"denied","targetNodeId":"send-denied"}));
    }
    serde_json::from_value(json!({
        "schemaVersion":"1.0.0","workflowId":"scenario-six","workflowVersion":1,
        "name":"Scenario six","description":"Durable approved exception workflow.",
        "compiler":{"model":"gemma-4-e2b-qat"},
        "nodes":[
            {"kind":"input","id":"input","label":"Run input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
            {"kind":"mcp_tool","id":"read-suppliers","label":"Read exact supplier fixture","serverName":"oomu_task_tools","toolName":"read_project_file","arguments":{"path":"/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU Test Data/mock_data/supplier_proposals.json"}},
            {"kind":"mcp_tool","id":"analyze-suppliers","label":"Calculate exact supplier variances","serverName":"oomu_task_tools","toolName":"analyze_supplier_exceptions","arguments":{"content":"{{nodes.read-suppliers.output.data.content}}"}},
            {"kind":"mcp_tool","id":"source","label":"Read official freight source","serverName":"oomu_task_tools","toolName":"fetch_official_page","arguments":{"url":"https://www.bts.gov/","maxContentChars":3000}},
            {"kind":"agent","id":"assess","label":"Assess exception","objective":"Explain the verified supplier variances and official evidence and prepare the report.","inputMappings":{"supplierAnalysis":"{{nodes.analyze-suppliers.output.data}}","source":"{{nodes.source.output.data}}"},"outputKey":"nodes.assess.output"},
            {"kind":"mcp_tool","id":"validate-report","label":"Validate evidence-bound report","serverName":"oomu_task_tools","toolName":"validate_evidence_report","arguments":{"content":"{{nodes.assess.output.data}}","supplierAnalysis":"{{nodes.analyze-suppliers.output.data}}","officialPageReceipts":["{{nodes.source.output.data}}"],"requiredSections":["Supplier variance","Current evidence","Risk assessment","Next actions"]}},
            {"kind":"mcp_tool","id":"write-report","label":"Create verified exception report","serverName":"oomu_task_tools","toolName":"create_file","arguments":{"file":{"title":"Supplier exception","content":"{{nodes.validate-report.output.data.content}}","locale":"en-US","format":"md","destinationPath":"ship_test_06/supplier_exception_<YYYY-MM-DD_HH-mm>.md"}}},
            {"kind":"conditional","id":"has-exception","label":"Supplier exceeds settled rate","condition":"$.hasException == true","inputMapping":"{{nodes.analyze-suppliers.output.data}}"},
            {"kind":"output","id":"no-exception","label":"No follow-up required","inputMapping":"{{nodes.write-report.output}}","outputSchema":{"type":"object"}},
            {"kind":"permission","id":"approve-calendar","label":"Approve exact Calendar event","permission":"mcp_tool","reason":"Create the named tentative event in OOMU Test.","onDenied":"branch"},
            {"kind":"output","id":"calendar-denied","label":"Calendar action declined","inputMapping":"{{nodes.write-report.output}}","outputSchema":{"type":"object"}},
            {"kind":"mcp_tool","id":"calendar","label":"Create exact conflict-free event","serverName":"oomu_task_tools","toolName":"create_conflict_free_calendar_event","arguments":{"calendarName":"OOMU Test","title":"Supplier Exception Follow-up","day":"next_weekday","windowStartLocal":"14:00","windowEndLocal":"18:00","durationMinutes":30,"location":"","notes":"Report: {{nodes.write-report.output.data.structuredContent.path}}","availability":"tentative"}},
            {"kind":"permission","id":"approve-send","label":"Approve exact email send","permission":"mcp_tool","reason":"Send one exact message to recipient@example.com.","onDenied":send_on_denied},
            {"kind":"output","id":"send-denied","label":"Email send declined","inputMapping":"{{nodes.write-report.output}}","outputSchema":{"type":"object"}},
            {"kind":"mcp_tool","id":"send","label":"Send and verify exact email","serverName":"oomu_task_tools","toolName":send_tool,"arguments":{"to":"recipient@example.com","subject":"OOMU Test — Supplier Exception","body":"Report: {{nodes.write-report.output.data.structuredContent.path}}","attachmentPath":"{{nodes.write-report.output.data.structuredContent.path}}"}},
            {"kind":"output","id":"output","label":"Verified completion","inputMapping":"{{nodes.send.output}}","outputSchema":{"type":"object"}}
        ],
        "edges":edges
    })).expect("scenario six IR")
}

fn mcp_tool_mut<'a>(ir: &'a mut WorkflowIr, id: &str) -> &'a mut McpToolNode {
    ir.nodes
        .iter_mut()
        .find_map(|node| match node {
            WorkflowNode::McpTool(tool) if tool.id == id => Some(tool),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing MCP tool {id}"))
}

fn agent_mut<'a>(ir: &'a mut WorkflowIr, id: &str) -> &'a mut crate::workflow_ir::AgentNode {
    ir.nodes
        .iter_mut()
        .find_map(|node| match node {
            WorkflowNode::Agent(agent) if agent.id == id => Some(agent),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing Agent {id}"))
}

fn edge_mut<'a>(ir: &'a mut WorkflowIr, id: &str) -> &'a mut WorkflowEdge {
    ir.edges
        .iter_mut()
        .find(|edge| edge.id == id)
        .unwrap_or_else(|| panic!("missing Workflow edge {id}"))
}

#[test]
fn authoritative_output_contract_reaches_prompt_and_replaces_generated_schema() {
    let contract = json!({
        "type": "object",
        "x-oomu-result-contract": {
            "kind": "collection",
            "path": "/structuredContent/results",
            "emptyIsSuccess": true
        }
    });
    let mut catalog = compose_catalog(true);
    let search = catalog
        .actions
        .iter_mut()
        .find(|action| action.tool_name.as_deref() == Some("query"))
        .unwrap();
    search.output_schema = Some(contract.clone());

    let prompt_catalog = compose_catalog_prompt_payload(&catalog);
    let prompt_action = prompt_catalog["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["toolName"] == json!("query"))
        .unwrap();
    assert_eq!(prompt_action["outputSchema"], contract);

    let mut workflow_ir = workflow_ir_with_heavy_metadata();
    let WorkflowNode::McpTool(tool) = &mut workflow_ir.nodes[1] else {
        panic!("expected MCP tool");
    };
    tool.output_schema = Some(json!({"modelInvented": true}));
    hydrate_mcp_output_schemas(&mut workflow_ir, &catalog);
    let WorkflowNode::McpTool(tool) = &workflow_ir.nodes[1] else {
        panic!("expected MCP tool");
    };
    assert_eq!(tool.output_schema.as_ref(), Some(&contract));
}

#[test]
fn direct_save_contract_hydration_removes_unverified_client_schema() {
    let forged_contract = json!({
        "type": "object",
        "x-oomu-result-contract": {
            "kind": "collection",
            "path": "/structuredContent/items",
            "emptyIsSuccess": true
        }
    });
    let mut workflow_ir: WorkflowIr = serde_json::from_value(json!({
            "schemaVersion":"1.0.0","workflowId":"wf-direct-save-contract","workflowVersion":1,
            "name":"Direct save contract","description":"Only the backend catalog may declare empty success.",
            "compiler":{"model":"gemma-4-e2b-qat"},
            "nodes":[
                {"kind":"input","id":"input","label":"Input","outputKey":"workflow.input","inputSchema":{"type":"object"}},
                {"kind":"mcp_tool","id":"read","label":"Read","serverName":"custom_mcp","toolName":"read_items","arguments":{},"outputSchema":forged_contract},
                {"kind":"output","id":"empty-output","label":"Nothing found","inputMapping":"{{nodes.read.output.data.structuredContent.items}}","outputSchema":{"type":"array"},"completionKind":"empty_collection"}
            ],
            "edges":[
                {"id":"e1","sourceNodeId":"input","sourcePort":"out","targetNodeId":"read"},
                {"id":"e2","sourceNodeId":"read","sourcePort":"out","targetNodeId":"empty-output"}
            ]
        }))
        .unwrap();
    let catalog = CapabilityCatalog {
        version: "test".to_string(),
        authoring_enabled: true,
        generated_at_ms: 1,
        actions: vec![CapabilityAction {
            id: "mcp:custom_mcp:read_items".to_string(),
            kind: "mcp_tool".to_string(),
            title: "Read items".to_string(),
            outcome: "Read items".to_string(),
            detail: "Read items".to_string(),
            source: "mcp".to_string(),
            available: true,
            availability: "available".to_string(),
            unavailable_reason: None,
            server_name: Some("custom_mcp".to_string()),
            tool_name: Some("read_items".to_string()),
            input_schema: Some(json!({"type":"object"})),
            output_schema: None,
            node_kind: Some("mcp".to_string()),
            node_template: None,
        }],
        templates: Vec::new(),
    };

    hydrate_mcp_output_schemas(&mut workflow_ir, &catalog);

    let WorkflowNode::McpTool(tool) = &workflow_ir.nodes[1] else {
        panic!("expected MCP tool");
    };
    assert!(tool.output_schema.is_none());
    let error = validate_workflow_ir_topology(&workflow_ir).unwrap_err();
    assert_eq!(error.code, WORKFLOW_TOPOLOGY_UNSAFE_COLLECTION_CODE);
}

#[test]
fn native_taskflow_capabilities_publish_deserialized_input_schemas() {
    let native_tools = native_taskflow_tools().unwrap();
    assert_eq!(native_tools.len(), 3);

    let actions = taskflow_native_capabilities().expect("native taskflow capabilities");
    for (tool_name, required_keys) in [
        ("folder_read", vec!["folderPath"]),
        ("write_markdown_report", vec!["reportPath", "content"]),
        ("preview_report", vec!["reportPath"]),
    ] {
        let action = actions
            .iter()
            .find(|action| {
                action.server_name.as_deref() == Some(TASKFLOW_NATIVE_SERVER)
                    && action.tool_name.as_deref() == Some(tool_name)
            })
            .unwrap_or_else(|| panic!("missing native action for {tool_name}"));
        let schema = action.input_schema.as_ref().unwrap();

        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["additionalProperties"], json!(false));
        for key in required_keys {
            assert!(schema["required"].as_array().unwrap().contains(&json!(key)));
            assert!(schema["properties"].get(key).is_some());
        }
        if tool_name == "folder_read" {
            let output_schema = action
                .output_schema
                .as_ref()
                .expect("folder_read collection contract");
            assert_eq!(
                output_schema["x-oomu-result-contract"]["kind"],
                json!("collection")
            );
            assert_eq!(
                output_schema["x-oomu-result-contract"]["path"],
                json!("/structuredContent/files")
            );
            assert_eq!(
                output_schema["x-oomu-result-contract"]["emptyIsSuccess"],
                json!(true)
            );
        }
    }
}

#[test]
fn missing_native_taskflow_schema_returns_metadata_error_without_panic() {
    let schemas = HashMap::<String, Value>::new();
    let error = native_schema_for_tool(&schemas, "folder_read").unwrap_err();

    assert_eq!(error.code, "workflow_compiler_metadata_failed");
    assert!(error.message.contains("folder_read"));
}

#[test]
fn mail_draft_intent_does_not_match_report_preview_tools() {
    let request = mail_compose_request_with_report_tools();
    let prompt = request.prompt.to_lowercase();
    let draft_mail = request
        .capability_catalog
        .actions
        .iter()
        .find(|action| action.tool_name.as_deref() == Some("draft_system_email"))
        .unwrap();
    let preview_report = request
        .capability_catalog
        .actions
        .iter()
        .find(|action| action.tool_name.as_deref() == Some("preview_report"))
        .unwrap();
    let write_report = request
        .capability_catalog
        .actions
        .iter()
        .find(|action| action.tool_name.as_deref() == Some("write_markdown_report"))
        .unwrap();

    assert!(action_matches_prompt(draft_mail, &prompt));
    assert!(!action_matches_prompt(preview_report, &prompt));
    assert!(!action_matches_prompt(write_report, &prompt));
}

#[test]
fn merged_catalog_keeps_the_complete_live_tool_catalog() {
    let live_catalog = CapabilityCatalog {
        version: WORKFLOW_CAPABILITY_CATALOG_VERSION.to_string(),
        authoring_enabled: true,
        generated_at_ms: 1,
        templates: Vec::new(),
        actions: vec![
            test_mcp_capability("macos_applescript", "read_system_calendar"),
            test_mcp_capability("macos_applescript", "read_system_emails"),
            test_mcp_capability("macos_applescript", "read_system_reminders"),
            test_mcp_capability("search", "query"),
        ],
    };
    let catalog = merge_compose_catalogs(
        live_catalog,
        CapabilityCatalog {
            version: "client".to_string(),
            authoring_enabled: true,
            generated_at_ms: 2,
            templates: Vec::new(),
            actions: Vec::new(),
        },
    );
    let tool_names = catalog
        .actions
        .iter()
        .filter_map(|action| action.tool_name.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(
        tool_names,
        vec![
            "query",
            "read_system_calendar",
            "read_system_emails",
            "read_system_reminders",
        ],
    );
}

#[test]
fn sanitizer_prunes_schema_metadata_without_mutating_runtime_ir() {
    let workflow_ir = workflow_ir_with_heavy_metadata();
    workflow_ir.validate().unwrap();

    let original_json = serde_json::to_string(&workflow_ir).unwrap();
    let compiler_ir = sanitize_workflow_ir_for_compiler(&workflow_ir);
    let compiler_json = serde_json::to_string(&compiler_ir).unwrap();

    assert!(
        compiler_json.len() < original_json.len() / 4,
        "sanitized compiler IR should be much smaller than original IR"
    );
    assert!(original_json.contains("giant-enum-value-199"));
    assert!(!compiler_json.contains("giant-enum-value-199"));
    assert!(serde_json::to_string(&workflow_ir)
        .unwrap()
        .contains("giant-enum-value-199"));

    match (&workflow_ir.nodes[0], &compiler_ir.nodes[0]) {
        (WorkflowNode::Input(original), WorkflowNode::Input(pruned)) => {
            assert!(original.input_schema.is_object());
            assert!(pruned.input_schema.is_null());
        }
        _ => panic!("expected input nodes"),
    }

    match (&workflow_ir.nodes[1], &compiler_ir.nodes[1]) {
        (WorkflowNode::McpTool(original), WorkflowNode::McpTool(pruned)) => {
            assert!(original.input_schema.is_some());
            assert!(pruned.input_schema.is_none());
            assert_eq!(pruned.arguments, original.arguments);
        }
        _ => panic!("expected MCP tool nodes"),
    }

    match (&workflow_ir.nodes[3], &compiler_ir.nodes[3]) {
        (WorkflowNode::Output(original), WorkflowNode::Output(pruned)) => {
            assert!(original.output_schema.is_object());
            assert!(pruned.output_schema.is_null());
        }
        _ => panic!("expected output nodes"),
    }
}
