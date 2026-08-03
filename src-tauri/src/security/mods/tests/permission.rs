use super::*;

#[test]
fn stored_mod_capabilities_reject_corrupt_json_instead_of_defaulting_empty() {
    let error = parse_mod_json_column::<Vec<ModPermission>>("not-json", 9)
        .expect_err("corrupt stored permissions must not become an empty permission list");
    assert!(matches!(
        error,
        rusqlite::Error::FromSqlConversionFailure(9, _, _)
    ));
    assert_eq!(format_last_updated(None), "Unavailable");
}

#[test]
fn localized_command_copy_never_grants_network_authority() {
    let command = ModCommand {
        trigger: "/travel".to_string(),
        description: HashMap::from([(
            "en-US".to_string(),
            "Search and compare live flights and prices.".to_string(),
        )]),
        public_network: false,
        context_url_templates: Vec::new(),
        context_parameters: HashMap::new(),
        required_context_evidence_patterns: Vec::new(),
    };

    assert!(!mod_command_requests_public_network(&command));
}
