use super::*;
use serde_json::Value;

#[test]
fn terminal_schema_is_not_removed_by_english_keyword_scoping() {
    for objective in [
        "Esegui il controllo richiesto nel progetto selezionato.",
        "選択したプロジェクトで必要な確認を実行してください。",
    ] {
        assert!(
            local_gemma_action_plan_contract_for_objective(objective)
                .pointer("/tools/terminal_execute/inputSchema")
                .is_some(),
            "terminal capability was hidden for: {objective}"
        );
    }
}

#[test]
fn scoped_production_contract_preserves_registered_schemas_and_effect_boundaries() {
    const ISOLATED_REGISTRY_ENV: &str = "OOMU_TEST_ISOLATED_SCOPED_PLANNER_REGISTRY";
    if std::env::var(ISOLATED_REGISTRY_ENV).ok().as_deref() != Some("1") {
        let output =
            std::process::Command::new(std::env::current_exe().expect("current Rust test executable"))
                .args([
                    "--exact",
                    "tools::registry::objective_scope_tests::scoped_production_contract_preserves_registered_schemas_and_effect_boundaries",
                    "--nocapture",
                ])
                .env(ISOLATED_REGISTRY_ENV, "1")
                .output()
                .expect("isolated production-registry schema test starts");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "isolated production-registry schema test failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        return;
    }

    crate::production_task_tools::register_production_task_tools()
        .expect("complete production planner tool registry");
    let operations = crate::tools::task_tool_runtime::registered_operations();
    assert!(!operations.is_empty());
    for operation in operations {
        let runtime_schema = crate::tools::task_tool_runtime::schema(operation)
            .unwrap_or_else(|error| panic!("runtime schema for {operation}: {error}"));
        let objective = format!("Use {operation} for this exact task.");
        let contract = local_gemma_action_plan_contract_for_objective(&objective);
        let planner_schema = contract
            .pointer(&format!("/tools/{operation}/inputSchema"))
            .unwrap_or_else(|| panic!("scoped contract omitted {operation}"));
        assert_compacted_schema_preserves(
            &runtime_schema,
            planner_schema,
            &format!("$.tools.{operation}.inputSchema"),
        );
    }

    for extension in ["txt", "json", "csv", "xlsx", "pdf"] {
        let objective =
            format!("Read /tmp/create_file_input.{extension} and summarize its stated facts.");
        let contract = local_gemma_action_plan_contract_for_objective(&objective);
        let tools = contract["tools"]
            .as_object()
            .expect("scoped planner tools object");
        assert!(tools.contains_key("file_read"));
        assert!(tools.contains_key("read_project_file"));
        for effectful in [
            "create_file",
            "create_spreadsheet",
            "create_presentation",
            "file_write",
            "delete_file",
            "codebase_patch",
            "codebase_compile",
            "telemetry_archive",
        ] {
            assert!(
                !tools.contains_key(effectful),
                "input-only .{extension} contract exposed {effectful}"
            );
        }
    }

    let write_contract =
        local_gemma_action_plan_contract_for_objective("Write the summary to /tmp/output.txt.");
    assert!(write_contract.pointer("/tools/create_file").is_some());
    assert!(write_contract.pointer("/tools/file_write").is_some());

    let delete_contract =
        local_gemma_action_plan_contract_for_objective("Delete /tmp/obsolete.txt.");
    assert!(delete_contract.pointer("/tools/delete_file").is_some());
    assert!(delete_contract.pointer("/tools/create_file").is_none());

    let spreadsheet_contract =
        local_gemma_action_plan_contract_for_objective("Create a spreadsheet at /tmp/report.xlsx.");
    assert!(spreadsheet_contract
        .pointer("/tools/create_spreadsheet/inputSchema")
        .is_some());

    for connected_objective in [
        "Use the MCP server to list the approved records.",
        "Use the MCP server named CRM to list approved records.",
        "Read my unread email in Apple Mail.",
        "Do I have any unread emails?",
        "Do I have any unread emails in Mail?",
        "Check my calendar for conflicts tomorrow.",
        "Do I have anything on my calendar tomorrow afternoon?",
        "What's on my calendar this afternoon?",
        "Use Apple Calendar to show my next event.",
    ] {
        let contract = local_gemma_action_plan_contract_for_objective(connected_objective);
        assert!(
            contract.pointer("/tools/connected_work").is_some(),
            "ordinary connected-work request was omitted: {connected_objective}"
        );
    }

    for disconnected_objective in [
        "Explain what an MCP server does.",
        "Do not use the MCP server; explain its security model.",
        "Explain how Apple Mail handles privacy.",
        "Do not read my email; explain common phishing warning signs.",
        "Compare Apple Calendar and Google Calendar without accessing either service.",
        "Email review@example.com the verified report.",
    ] {
        let contract = local_gemma_action_plan_contract_for_objective(disconnected_objective);
        assert!(
            contract.pointer("/tools/connected_work").is_none(),
            "non-executable connector mention exposed connected_work: {disconnected_objective}"
        );
    }

    let mixed_read_only = local_gemma_action_plan_contract_for_objective(
        "Read /tmp/input.json. Do not write /tmp/output.txt or delete /tmp/input.json. Summarize in chat.",
    );
    assert!(mixed_read_only.pointer("/tools/file_read").is_some());
    assert!(mixed_read_only
        .pointer("/tools/read_project_file")
        .is_some());
    for prohibited in ["create_file", "file_write", "delete_file"] {
        assert!(
            mixed_read_only
                .pointer(&format!("/tools/{prohibited}"))
                .is_none(),
            "clause-local prohibition exposed {prohibited}"
        );
    }

    for (negative, prohibited) in [
        (
            "Do not create a spreadsheet at /tmp/report.xlsx.",
            &["create_file", "create_spreadsheet", "file_write"][..],
        ),
        (
            "Do not create a presentation at /tmp/brief.pptx.",
            &["create_file", "create_presentation", "file_write"][..],
        ),
        (
            "Do not send an email to review@example.com through Apple Mail.",
            &["send_system_email", "connected_work"][..],
        ),
        (
            "Do not draft an email to review@example.com in Apple Mail.",
            &["draft_system_email", "connected_work"][..],
        ),
        (
            "Do not create an Apple Calendar event titled Review tomorrow.",
            &[
                "create_system_calendar_event",
                "create_conflict_free_calendar_event",
                "connected_work",
            ][..],
        ),
        (
            "Do not configure the Signal channel.",
            &["configure_channel"][..],
        ),
        (
            "Do not use the MCP server to list records.",
            &["connected_work"][..],
        ),
    ] {
        let contract = local_gemma_action_plan_contract_for_objective(negative);
        for operation in prohibited {
            assert!(
                contract.pointer(&format!("/tools/{operation}")).is_none(),
                "negated action exposed {operation}: {negative}"
            );
        }
    }

    for (positive, required) in [
        (
            "Create a presentation at /tmp/brief.pptx.",
            "create_presentation",
        ),
        ("Send an email to review@example.com.", "send_system_email"),
        (
            "Draft an email to review@example.com.",
            "draft_system_email",
        ),
        (
            "Create a calendar event titled Review tomorrow.",
            "create_system_calendar_event",
        ),
        ("Configure the Signal channel.", "configure_channel"),
        ("Use the MCP server to list records.", "connected_work"),
    ] {
        assert!(
            local_gemma_action_plan_contract_for_objective(positive)
                .pointer(&format!("/tools/{required}"))
                .is_some(),
            "legitimate action omitted {required}: {positive}"
        );
    }

    let comparison = "Research current official sources on scheduled/background agent capabilities in OpenClaw and Claude Cowork. Write the comparison to /tmp/background_agents.md and read it back.";
    assert!(local_gemma_action_plan_contract_for_objective(comparison)
        .pointer("/tools/prepare_background_agent_comparison")
        .is_some());
    let recovery = "Read /tmp/milestone_records.json and construct a recovery plan respecting dependencies, one-owner capacity, business hours, a 20% contingency reserve, and the requirement that security validation precede release validation. Write the assumptions, critical path, and three failure contingencies to /tmp/recovery.md and verify the file.";
    assert!(local_gemma_action_plan_contract_for_objective(recovery)
        .pointer("/tools/prepare_milestone_constraint_recovery_plan")
        .is_some());

    for informational_objective in [
        "Explain the differences between OpenClaw and Claude Cowork background agents.",
        "Discuss current official OpenClaw and Claude Cowork background documentation without writing /tmp/comparison.md, then read it back in the discussion.",
        "Explain common milestone recovery-plan approaches and dependency tradeoffs.",
        "Read /tmp/milestone_records.json, but do not write a recovery plan to /tmp/recovery.md; discuss dependencies, one-owner capacity, business hours, a 20% contingency reserve, the requirement that security validation precede release validation, and three failure contingencies.",
    ] {
        let contract = local_gemma_action_plan_contract_for_objective(informational_objective);
        for specialist in [
            "prepare_background_agent_comparison",
            "prepare_milestone_constraint_recovery_plan",
        ] {
            assert!(
                contract.pointer(&format!("/tools/{specialist}")).is_none(),
                "informational or negated objective exposed {specialist}: {informational_objective}"
            );
        }
    }
}

fn assert_compacted_schema_preserves(expected: &Value, actual: &Value, path: &str) {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            let expected_retained = expected
                .keys()
                .filter(|key| !PLANNER_SCHEMA_OMITTED_ANNOTATION_KEYS.contains(&key.as_str()))
                .count();
            assert_eq!(
                actual.len(),
                expected_retained,
                "planner schema changed key count at {path}"
            );
            for (key, expected_value) in expected {
                let child_path = format!("{path}.{key}");
                if PLANNER_SCHEMA_OMITTED_ANNOTATION_KEYS.contains(&key.as_str()) {
                    assert!(
                        !actual.contains_key(key),
                        "planner schema retained annotation at {child_path}"
                    );
                    continue;
                }
                let actual_value = actual.get(key).unwrap_or_else(|| {
                    panic!("planner schema dropped validation key at {child_path}")
                });
                assert_compacted_schema_preserves(expected_value, actual_value, &child_path);
            }
        }
        (Value::Array(expected), Value::Array(actual)) => {
            assert_eq!(
                actual.len(),
                expected.len(),
                "array length changed at {path}"
            );
            for (index, (expected_value, actual_value)) in expected.iter().zip(actual).enumerate() {
                assert_compacted_schema_preserves(
                    expected_value,
                    actual_value,
                    &format!("{path}[{index}]"),
                );
            }
        }
        _ => assert_eq!(actual, expected, "schema value changed at {path}"),
    }
}
