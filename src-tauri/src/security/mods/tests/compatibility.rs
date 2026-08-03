use super::*;

#[test]
fn active_mod_rejects_malformed_compatibility_and_command_metadata() {
    let malformed_requirements = json!({"requirements": "not-an-object"});
    let error = strict_manifest_requirements(&malformed_requirements, "mod.invalid")
        .expect_err("malformed requirements must not be treated as no requirements");
    assert!(error.contains("invalid requirements"));

    let malformed_commands = json!({"commands": {"trigger": "/fake"}});
    let error = strict_manifest_commands(&malformed_commands, "mod.invalid")
        .expect_err("malformed commands must not be silently omitted");
    assert!(error.contains("invalid commands"));
}

#[test]
fn declarative_network_commands_render_only_allowlisted_https_context_urls() {
    let command = ModCommand {
        trigger: "/compare".to_string(),
        description: HashMap::new(),
        public_network: true,
        context_url_templates: vec![
            "https://travel.example.com/search?q={query}&currency=USD".to_string()
        ],
        context_parameters: HashMap::new(),
        required_context_evidence_patterns: Vec::new(),
    };
    assert!(mod_command_requests_public_network(&command));

    let authority = AuthorizedNetworkModCommand {
        mod_id: "com.example.travel".to_string(),
        search_query: "compare ROC to SIN".to_string(),
        allowed_hosts: vec!["*.example.com".to_string()],
        context_urls: Vec::new(),
        required_context_evidence_patterns: Vec::new(),
    };
    let urls = render_mod_context_urls(
        &command.context_url_templates,
        &command.context_parameters,
        "ROC to SIN & return March 21",
        &authority,
    )
    .expect("declared source renders");
    assert_eq!(
        urls,
        vec![
            "https://travel.example.com/search?q=ROC+to+SIN+%26+return+March+21&currency=USD"
                .to_string()
        ]
    );

    let escaped_host = render_mod_context_urls(
        &["https://attacker.example.net/?q={query}".to_string()],
        &HashMap::new(),
        "public facts",
        &authority,
    )
    .expect_err("a manifest cannot escape its reviewed host allowlist");
    assert!(escaped_host.contains("outside its declared hosts"));

    let insecure = render_mod_context_urls(
        &["http://travel.example.com/?q={query}".to_string()],
        &HashMap::new(),
        "public facts",
        &authority,
    )
    .expect_err("context sources must use HTTPS");
    assert!(insecure.contains("credential-free HTTPS"));
}

#[test]
fn declarative_context_parameters_build_an_exact_provider_route() {
    let parameters = HashMap::from([
        (
            "origin".to_string(),
            ModContextParameter {
                pattern: r"(?i)\bfrom\s+([a-z]{3})\b".to_string(),
                transform: "uppercase".to_string(),
            },
        ),
        (
            "destination".to_string(),
            ModContextParameter {
                pattern: r"(?i)\bto\s+([a-z]{3})\b".to_string(),
                transform: "uppercase".to_string(),
            },
        ),
        (
            "departure_date".to_string(),
            ModContextParameter {
                pattern: r"(?i)\bon\s+([a-z]+\s+\d{1,2},?\s+\d{4})".to_string(),
                transform: "date_iso".to_string(),
            },
        ),
        (
            "return_date".to_string(),
            ModContextParameter {
                pattern: r"(?i)\breturn(?:ing)?(?:\s+on)?\s+([a-z]+\s+\d{1,2},?\s+\d{4})"
                    .to_string(),
                transform: "date_iso".to_string(),
            },
        ),
    ]);
    let authority = AuthorizedNetworkModCommand {
        mod_id: "com.example.travel".to_string(),
        search_query: "travel ROC to SIN".to_string(),
        allowed_hosts: vec!["*.kayak.com".to_string()],
        context_urls: Vec::new(),
        required_context_evidence_patterns: Vec::new(),
    };
    let templates = vec![
            "https://www.kayak.com/flights/{origin}-{destination}/{departure_date}/{return_date}?sort=bestflight_a"
                .to_string(),
        ];

    let urls = render_mod_context_urls(
        &templates,
        &parameters,
        "I need the best flight from ROC to SIN on March 14, 2027 and returning on March 21, 2027",
        &authority,
    )
    .expect("structured route renders");
    assert_eq!(
        urls,
        vec![
            "https://www.kayak.com/flights/ROC-SIN/2027-03-14/2027-03-21?sort=bestflight_a"
                .to_string()
        ]
    );

    let incomplete = render_mod_context_urls(
        &templates,
        &parameters,
        "Compare flights from ROC to SIN",
        &authority,
    )
    .expect("missing optional extraction falls back cleanly");
    assert!(incomplete.is_empty());
}

#[test]
fn validate_active_mod_compatibility_uses_provider_and_local_model_requirements() {
    let mut localized_errors = HashMap::new();
    localized_errors.insert(
        "en-US".to_string(),
        "The Claim Deconstructor requires Google Gemini or Gemma 4 12B.".to_string(),
    );
    localized_errors.insert(
        "zh-CN".to_string(),
        "医疗理赔分析器需要切换到 Google Gemini 或 Gemma 4 12B。".to_string(),
    );
    let manifest = InstalledMod {
        id: "ai.eldris.mods.claim_deconstructor".to_string(),
        name: "Claim Deconstructor".to_string(),
        description: String::new(),
        is_active: true,
        version: "1.0.0".to_string(),
        author: "Test".to_string(),
        category: "Healthcare".to_string(),
        package_size: String::new(),
        last_updated: String::new(),
        review_state: "unreviewed".to_string(),
        publisher_identity_verified: false,
        integrity_state: "unsigned".to_string(),
        is_built_in: false,
        permissions: Vec::new(),
        endpoints: Vec::new(),
        agent_config_schema: None,
        commands: None,
        requirements: Some(ModRequirements {
            min_cognitive_tier: Some("capable".to_string()),
            supported_provider_classes: Some(vec![
                "google".to_string(),
                "openai".to_string(),
                "anthropic".to_string(),
            ]),
            supported_local_models: Some(vec!["gemma-4-12B-it-qat-q4_0-gguf".to_string()]),
            error_notice_override: Some(localized_errors),
        }),
    };

    assert!(
        validate_active_mod_compatibility(&manifest, "gemini", "gemini-3.5-flash", "en-US").is_ok()
    );
    assert!(validate_active_mod_compatibility(
        &manifest,
        "local_model",
        "gemma-4-12B-it-qat-q4_0-gguf",
        "en-US"
    )
    .is_ok());
    let error = validate_active_mod_compatibility(
        &manifest,
        "local_model",
        "gemma-4-E4B-it-qat-q4_0-gguf",
        "zh-CN",
    )
    .expect_err("unsupported local model is blocked");
    assert_eq!(
        error,
        "医疗理赔分析器需要切换到 Google Gemini 或 Gemma 4 12B。"
    );
    let provider_error =
        validate_active_mod_compatibility(&manifest, "mistral", "ministral-8b", "fr-FR")
            .expect_err("unsupported provider is blocked");
    assert_eq!(
        provider_error,
        "The Claim Deconstructor requires Google Gemini or Gemma 4 12B."
    );
}

#[test]
fn ensure_schema_does_not_seed_unshipped_internal_mods() {
    let engine = test_engine("unshipped_internal_mods");
    let mods = select_installed_mods(&engine).expect("installed mods load");
    assert!(mods.is_empty());
}

#[test]
fn schema_refresh_retires_legacy_embedded_rows_and_preserves_installed_mods() {
    let engine = test_engine("retire_embedded_mods");
    let connection = engine.open_connection().expect("connection opens");
    ensure_schema(&connection).expect("schema initializes");
    for (id, name, entrypoint) in [
        (
            "ai.eldris.mods.alignment",
            "Core Alignment Matrix",
            "builtin://alignment",
        ),
        (
            "ai.eldris.mods.developer_bundle",
            "Developer Bundle",
            "builtin://developer_bundle",
        ),
    ] {
        let manifest = json!({
            "id": id,
            "name": name,
            "version": "1.0.0",
            "author": "Eldris AI",
            "description": "Legacy embedded fixture.",
            "entrypoint": entrypoint
        });
        connection
            .execute(
                "
                INSERT INTO installed_mods (
                    id, name, description, is_active, version, author, category,
                    package_size, last_updated, permissions_json, endpoints_json,
                    installed_path, manifest_json, default_system_prompt, entrypoint,
                    is_built_in, installed_at_ms, updated_at_ms
                )
                VALUES (?1, ?2, 'Legacy embedded fixture.', 1, '1.0.0', 'Eldris AI',
                        'Internal', 'Built in', 'Legacy', '[]', '[]', ?3, ?4, NULL,
                        ?3, 1, ?5, ?5)
                ",
                params![id, name, entrypoint, manifest.to_string(), now_ms()],
            )
            .expect("legacy embedded row inserted");
    }
    insert_test_mod(
        &connection,
        "ai.eldris.mods.travel_companion",
        "Travel Companion",
        true,
        Some("Build a grounded itinerary."),
    );

    ensure_schema(&connection).expect("schema refresh retires embedded rows");
    ensure_schema(&connection).expect("retirement is idempotent");
    let installed_ids = connection
        .prepare("SELECT id FROM installed_mods ORDER BY id")
        .expect("installed mod query prepares")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("installed mod query runs")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("installed mod ids load");
    assert_eq!(
        installed_ids,
        vec!["ai.eldris.mods.travel_companion".to_string()]
    );
    let retired_count: i64 = connection
        .query_row(
            "
            SELECT COUNT(*) FROM installed_mods
            WHERE id IN ('ai.eldris.mods.alignment', 'ai.eldris.mods.developer_bundle')
            ",
            [],
            |row| row.get(0),
        )
        .expect("retired row count loads");
    assert_eq!(retired_count, 0);
}
