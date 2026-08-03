use super::*;

#[test]
fn required_context_evidence_patterns_are_bounded_and_non_vacuous() {
    validate_required_context_evidence_patterns(&[
        r"(?i)\$\s*\d{2,}".to_string(),
        r"(?i)\b(?:nonstop|\d+\s+stops?)\b".to_string(),
    ])
    .expect("bounded factual patterns validate");

    assert!(validate_required_context_evidence_patterns(&["(".to_string()]).is_err());
    assert!(validate_required_context_evidence_patterns(&[".*".to_string()]).is_err());
    assert!(validate_required_context_evidence_patterns(&vec!["fare".to_string(); 5]).is_err());
}

#[test]
fn validate_mod_network_access_requires_declared_host() {
    let permissions = ModPermissions {
        allowed_paths: None,
        allowed_hosts: Some(vec![
            "api.zendesk.com".to_string(),
            "https://hooks.slack.com".to_string(),
            "*.google.com".to_string(),
        ]),
    };

    validate_mod_network_access(
        "test.mod.cs",
        "https://API.ZENDESK.COM/v2/tickets",
        &permissions,
    )
    .expect("declared host is accepted");

    let denied = validate_mod_network_access(
        "test.mod.cs",
        "https://sub.api.zendesk.com/v2/tickets",
        &permissions,
    );
    assert!(matches!(
        denied,
        Err(SecurityError::UnauthorizedAccess(message))
            if message.contains("test.mod.cs")
    ));

    assert!(matches!(
        validate_mod_network_access("test.mod.cs", "ftp://api.zendesk.com/file", &permissions),
        Err(SecurityError::EndpointNormalizationFailed { .. })
    ));
    validate_mod_network_access(
        "test.mod.cs",
        "https://www.google.com/travel/flights",
        &permissions,
    )
    .expect("declared wildcard subdomain is accepted");
    assert!(validate_mod_network_access(
        "test.mod.cs",
        "https://google.com/travel/flights",
        &permissions,
    )
    .is_err());
}

#[test]
fn active_mod_prompt_context_requires_active_bound_mod_for_agent() {
    let engine = test_engine("prompt_context");
    {
        let connection = engine.open_connection().expect("connection opens");
        ensure_schema(&connection).expect("schema initializes");
        insert_test_mod(
            &connection,
            "ai.eldris.mods.pundamentals",
            "Pundamentals",
            true,
            Some("Add one contextual pun."),
        );
        insert_test_mod(
            &connection,
            "ai.eldris.mods.auditor",
            "Auditor",
            true,
            Some("Audit every claim."),
        );
        insert_test_mod(
            &connection,
            "ai.eldris.mods.inactive",
            "Inactive",
            false,
            Some("This should never be applied."),
        );
        insert_test_mod(
            &connection,
            "ai.eldris.mods.empty",
            "Empty",
            true,
            Some("   "),
        );
    }

    let agent_a_context = active_mod_prompt_context_details(
        &engine,
        &[
            "ai.eldris.mods.pundamentals".to_string(),
            "ai.eldris.mods.inactive".to_string(),
            "ai.eldris.mods.empty".to_string(),
        ],
    )
    .expect("context query succeeds")
    .expect("agent-a has active bound context");
    assert_eq!(agent_a_context.selection_mode, "agent_binding");
    assert_eq!(
        agent_a_context.applied_mod_ids,
        vec!["ai.eldris.mods.pundamentals".to_string()]
    );
    assert!(agent_a_context
        .prompt
        .contains("Active OOMU Mod Runtime Contract"));
    assert!(agent_a_context
        .prompt
        .contains("Status: mandatory for this turn."));
    assert!(agent_a_context.prompt.contains("Pundamentals"));
    assert!(agent_a_context.prompt.contains("Required behavior:"));
    assert!(agent_a_context.prompt.contains("Add one contextual pun."));
    assert!(!agent_a_context.prompt.contains("Auditor"));
    assert!(!agent_a_context.prompt.contains("Inactive"));
    assert!(!agent_a_context.prompt.contains("Empty"));

    let unbound_context =
        active_mod_prompt_context_details(&engine, &[]).expect("context query succeeds");
    assert!(unbound_context.is_none());

    let mismatched_context =
        active_mod_prompt_context_details(&engine, &["ai.eldris.mods.missing".to_string()])
            .expect("context query succeeds");
    assert!(mismatched_context.is_none());
}
