use super::*;

#[test]
fn mod_package_grant_response_is_pathless_and_one_use() {
    let test_dir = test_temp_dir("grant_one_use");
    let selected_path = test_dir.join("private-customer-name.oomu");
    let archive = b"selected archive bytes";
    fs::write(&selected_path, archive).expect("test package written");
    let registry = Mutex::new(ModPackageGrantRegistry::default());

    let grant = issue_mod_package_grant(&registry, &selected_path, Duration::from_secs(60))
        .expect("grant issued");
    assert!(is_valid_mod_package_grant_id(&grant.grant_id));
    let response = serde_json::to_string(&grant).expect("grant serializes");
    assert!(!response.contains("private-customer-name"));
    assert!(!response.contains(test_dir.to_string_lossy().as_ref()));
    assert!(response.contains("grantId"));
    assert!(response.contains("expiresAtMs"));

    assert_eq!(
        consume_mod_package_grant(&registry, selected_path.to_string_lossy().as_ref())
            .expect_err("an absolute renderer path is not authority"),
        ModPackageGrantError::InvalidOrExpired
    );

    let verified = consume_mod_package_grant(&registry, &grant.grant_id)
        .expect("opaque grant is accepted once");
    assert_eq!(verified.archive, archive);
    assert_eq!(
        consume_mod_package_grant(&registry, &grant.grant_id)
            .expect_err("grant cannot be replayed"),
        ModPackageGrantError::InvalidOrExpired
    );
}

#[test]
fn mod_package_grant_expires_and_is_consumed_on_rejection() {
    let test_dir = test_temp_dir("grant_expired");
    let selected_path = test_dir.join("expired.oomu");
    fs::write(&selected_path, b"archive").expect("test package written");
    let registry = Mutex::new(ModPackageGrantRegistry::default());
    let grant =
        issue_mod_package_grant(&registry, &selected_path, Duration::ZERO).expect("grant issued");

    assert_eq!(
        consume_mod_package_grant(&registry, &grant.grant_id).expect_err("expired grant rejected"),
        ModPackageGrantError::InvalidOrExpired
    );
    assert!(!registry
        .lock()
        .expect("registry lock")
        .grants
        .contains_key(&grant.grant_id));
}

#[cfg(unix)]
#[test]
fn mod_package_grant_rejects_path_replacement_and_cannot_be_replayed() {
    let test_dir = test_temp_dir("grant_replaced");
    let selected_path = test_dir.join("replace-me.oomu");
    let displaced_path = test_dir.join("original.oomu");
    fs::write(&selected_path, b"trusted archive").expect("test package written");
    let registry = Mutex::new(ModPackageGrantRegistry::default());
    let grant = issue_mod_package_grant(&registry, &selected_path, Duration::from_secs(60))
        .expect("grant issued");

    fs::rename(&selected_path, &displaced_path).expect("selected file displaced");
    fs::write(&selected_path, b"attacker archive").expect("replacement written");

    assert_eq!(
        consume_mod_package_grant(&registry, &grant.grant_id).expect_err("replacement rejected"),
        ModPackageGrantError::FileChanged
    );
    assert_eq!(
        consume_mod_package_grant(&registry, &grant.grant_id)
            .expect_err("failed verification still consumes authority"),
        ModPackageGrantError::InvalidOrExpired
    );
}

#[test]
fn mod_package_grant_digest_rejects_content_change_even_if_identity_matches() {
    let test_dir = test_temp_dir("grant_digest");
    let selected_path = test_dir.join("digest.oomu");
    fs::write(&selected_path, b"trusted-content").expect("test package written");
    let registry = Mutex::new(ModPackageGrantRegistry::default());
    let grant = issue_mod_package_grant(&registry, &selected_path, Duration::from_secs(60))
        .expect("grant issued");

    fs::write(&selected_path, b"changed-content").expect("selected package mutated");
    let current_identity = ModPackageFileIdentity::from_metadata(
        &fs::metadata(&selected_path).expect("mutated metadata readable"),
    );
    registry
        .lock()
        .expect("registry lock")
        .grants
        .get_mut(&grant.grant_id)
        .expect("grant exists")
        .identity = current_identity;

    assert_eq!(
        consume_mod_package_grant(&registry, &grant.grant_id)
            .expect_err("digest mismatch rejected"),
        ModPackageGrantError::FileChanged
    );
}

#[test]
fn manifest_permissions_require_snake_case_keys() {
    let manifest: ModManifest = serde_json::from_value(manifest_json_with_permissions(json!({
        "allowed_paths": ["/tmp/oomu_test"],
        "allowed_hosts": ["api.zendesk.com"]
    })))
    .expect("structured permissions deserialize");
    let permissions = manifest.permissions.expect("permissions parsed");

    assert_eq!(
        permissions.allowed_paths,
        Some(vec!["/tmp/oomu_test".to_string()])
    );
    assert_eq!(
        permissions.allowed_hosts,
        Some(vec!["api.zendesk.com".to_string()])
    );

    let legacy_manifest: ModManifest =
        serde_json::from_value(manifest_json_with_permissions(json!([])))
            .expect("legacy permission arrays remain readable");
    assert!(legacy_manifest.permissions.is_none());

    let error = serde_json::from_value::<ModManifest>(manifest_json_with_permissions(json!({
        "allowedPaths": ["/tmp/oomu_test"]
    })))
    .expect_err("camelCase permission keys are rejected")
    .to_string();
    assert!(error.contains("unknown field"));
    assert!(error.contains("allowedPaths"));
}

#[test]
fn mod_identifiers_cannot_alias_the_same_storage_directory() {
    assert!(valid_mod_identifier("com.acme.reports-v1"));
    assert!(!valid_mod_identifier("a/b"));
    assert!(!valid_mod_identifier("a?b"));
    assert!(!valid_mod_identifier(".hidden"));
    assert!(!valid_mod_identifier("Com.acme.mod"));
    assert!(!valid_mod_identifier(" com.acme.mod "));
    assert_ne!(storage_id("com.acme.one"), storage_id("com.acme.two"));
}

#[test]
fn uninstall_never_uses_a_sanitized_fallback_for_an_unknown_id() {
    let engine = test_engine("uninstall_alias");
    let installed = test_temp_dir("uninstall_alias_files");
    fs::write(installed.join("keep"), "keep").unwrap();
    let connection = engine.open_connection().unwrap();
    ensure_schema(&connection).unwrap();
    insert_test_mod(&connection, "a_b", "Real mod", false, None);
    connection
        .execute(
            "UPDATE installed_mods SET installed_path=?2 WHERE id=?1",
            params!["a_b", installed.to_string_lossy()],
        )
        .unwrap();
    drop(connection);
    delete_installed_mod(&engine, "a/b").unwrap();
    assert!(installed.join("keep").exists());
    assert!(ensure_installed_mod_exists(&engine, "a_b").is_ok());
}

#[test]
fn active_manifest_selection_stops_when_files_cannot_be_verified() {
    let engine = test_engine("active_mod_missing_files");
    let connection = engine.open_connection().expect("connection opens");
    ensure_schema(&connection).expect("schema initializes");
    insert_test_mod(&connection, "com.acme.missing", "Missing", true, None);
    connection
            .execute(
                "UPDATE installed_mods SET installed_path='/definitely/missing/oomu-mod' WHERE id='com.acme.missing'",
                [],
            )
            .unwrap();
    drop(connection);
    assert!(active_mod_manifest_records(&engine).is_err());
    let active: i64 = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT is_active FROM installed_mods WHERE id='com.acme.missing'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active, 0);
}

#[test]
fn select_installed_mods_hydrates_commands_and_requirements_from_manifest_json() {
    let engine = test_engine("mod_commands_requirements");
    let manifest = json!({
        "commands": [
            {
                "trigger": "/claim",
                "description": {
                    "en-US": "Analyze a denial letter.",
                    "es-ES": "Analiza una carta de denegación."
                },
                "public_network": true,
                "context_url_templates": [
                    "https://claims.example.com/search?q={query}"
                ]
            }
        ],
        "requirements": {
            "min_cognitive_tier": "capable",
            "supported_provider_classes": ["google", "openai", "anthropic"],
            "supported_local_models": ["gemma-4-12B-it-qat-q4_0-gguf"],
            "error_notice_override": {
                "en-US": "Switch to Google Gemini or Gemma 4 12B.",
                "zh-CN": "请切换到 Google Gemini 或 Gemma 4 12B。"
            }
        }
    });
    {
        let connection = engine.open_connection().expect("connection opens");
        ensure_schema(&connection).expect("schema initializes");
        connection
            .execute(
                "
                    INSERT INTO installed_mods (
                        id, name, description, is_active, version, author, category,
                        package_size, last_updated, permissions_json, endpoints_json,
                        installed_path, manifest_json, default_system_prompt, entrypoint,
                        installed_at_ms, updated_at_ms
                    )
                    VALUES (
                        'ai.eldris.mods.claim_deconstructor', 'Claim Deconstructor', 'Claims.',
                        1, '1.0.0', 'Test', 'Healthcare', '1 KB', 'June 22, 2026',
                        '[]', '[]', '/tmp/test-mod', ?1, 'Prompt.', 'index.js', ?2, ?2
                    )
                    ",
                params![manifest.to_string(), now_ms()],
            )
            .expect("command-bearing test mod inserted");
    }

    let mods = select_installed_mods(&engine).expect("installed mods load");
    let claim = mods
        .iter()
        .find(|installed_mod| installed_mod.id == "ai.eldris.mods.claim_deconstructor")
        .expect("claim mod loads");
    let command = claim
        .commands
        .as_ref()
        .and_then(|commands| commands.first())
        .expect("command hydrates");
    assert_eq!(command.trigger, "/claim");
    assert_eq!(
        command.description.get("es-ES").map(String::as_str),
        Some("Analiza una carta de denegación.")
    );
    assert!(command.public_network);
    assert_eq!(
        command.context_url_templates,
        vec!["https://claims.example.com/search?q={query}".to_string()]
    );
    let requirements = claim.requirements.as_ref().expect("requirements hydrate");
    assert_eq!(
        requirements.supported_local_models.as_deref(),
        Some(&["gemma-4-12B-it-qat-q4_0-gguf".to_string()][..])
    );
}

#[test]
fn select_installed_mods_hydrates_agent_config_schema_from_manifest_json() {
    let engine = test_engine("agent_config_schema");
    let manifest = json!({
        "agent_config_schema": {
            "title": "Risk Guardrails",
            "type": "object",
            "properties": {
                "riskLimit": {
                    "type": "number",
                    "title": "Risk limit",
                    "minimum": 0,
                    "maximum": 1,
                    "default": 0.25
                },
                "auditMode": {
                    "type": "boolean",
                    "title": "Audit mode",
                    "default": true
                }
            }
        }
    });
    {
        let connection = engine.open_connection().expect("connection opens");
        ensure_schema(&connection).expect("schema initializes");
        connection
            .execute(
                "
                    INSERT INTO installed_mods (
                        id, name, description, is_active, version, author, category,
                        package_size, last_updated, permissions_json, endpoints_json,
                        installed_path, manifest_json, default_system_prompt, entrypoint,
                        installed_at_ms, updated_at_ms
                    )
                    VALUES (
                        'ai.eldris.mods.risk-guardrails', 'Risk Guardrails', 'Controls risk.',
                        1, '1.0.0', 'Test', 'Safety', '1 KB', 'June 22, 2026',
                        '[]', '[]', '/tmp/test-mod', ?1, null, 'index.js', ?2, ?2
                    )
                    ",
                params![manifest.to_string(), now_ms()],
            )
            .expect("schema-bearing test mod inserted");
    }

    let mods = select_installed_mods(&engine).expect("installed mods load");
    let schema = mods
        .iter()
        .find(|installed_mod| installed_mod.id == "ai.eldris.mods.risk-guardrails")
        .and_then(|installed_mod| installed_mod.agent_config_schema.as_ref())
        .expect("agent config schema is hydrated");

    assert_eq!(schema["title"], "Risk Guardrails");
    assert_eq!(schema["properties"]["riskLimit"]["default"], 0.25);
    assert_eq!(schema["properties"]["auditMode"]["default"], true);
}

#[test]
fn safe_mode_filters_out_all_installed_mods() {
    let engine = test_engine("safe_mode_filter");
    {
        let connection = engine.open_connection().expect("connection opens");
        ensure_schema(&connection).expect("schema initializes");
        insert_test_mod(
            &connection,
            "ai.eldris.mods.claim_deconstructor",
            "Claim Deconstructor",
            true,
            Some("Analyze claims."),
        );
    }

    let mods = select_installed_mods(&engine).expect("installed mods load");
    let filtered = filter_installed_mods_for_safe_mode(mods, true);
    assert!(filtered.is_empty());

    let filtered_ids =
        filter_mod_ids_for_safe_mode(vec!["ai.eldris.mods.claim_deconstructor".to_string()], true);
    assert!(filtered_ids.is_empty());
}

#[test]
fn update_mod_active_state_recovers_missing_row_from_installed_directory() {
    let engine = test_engine("recover_installed_mod");
    let installed_dir = test_temp_dir("recover_installed_mod_dir");
    fs::write(installed_dir.join("index.js"), "export default {};").expect("entrypoint written");
    let manifest = json!({
        "id": "ai.eldris.mods.recoverable",
        "name": "Recoverable Mod",
        "version": "1.0.0",
        "author": "Eldris AI",
        "description": "Can be recovered from disk.",
        "category": "Behavior",
        "permissions": [],
        "endpoints": [],
        "hooks": {},
        "entrypoint": "index.js",
        "default_system_prompt": "Recovered prompt.",
        "agent_config_schema": {
            "title": "Recoverable Settings",
            "properties": {
                "mode": {
                    "type": "string",
                    "title": "Mode",
                    "enum": ["Concise", "Detailed"],
                    "default": "Concise"
                }
            }
        }
    });
    fs::write(installed_dir.join("manifest.json"), manifest.to_string()).expect("manifest written");

    update_mod_active_state_from_directory(
        &engine,
        "ai.eldris.mods.recoverable",
        true,
        &installed_dir,
    )
    .expect_err("an unsigned recovered mod still requires review");

    let mods = select_installed_mods(&engine).expect("installed mods load");
    let recovered = mods
        .iter()
        .find(|installed_mod| installed_mod.id == "ai.eldris.mods.recoverable")
        .expect("recovered mod is listed");
    assert!(!recovered.is_active);
    assert_eq!(recovered.review_state, "unreviewed");
    assert_eq!(recovered.integrity_state, "unsigned");
    assert_eq!(
        recovered
            .agent_config_schema
            .as_ref()
            .and_then(|schema| schema.pointer("/properties/mode/default")),
        Some(&json!("Concise"))
    );
}

#[test]
fn verify_manifest_permission_isolation() {
    let root = test_temp_dir("permission_gate");
    let allowed_dir = root.join("allowed");
    let transcript_dir = allowed_dir.join("transcripts");
    let outside_dir = root.join("outside");
    fs::create_dir_all(&transcript_dir).expect("allowed transcript directory created");
    fs::create_dir_all(&outside_dir).expect("outside directory created");
    let secure_path = transcript_dir.join("active.json");
    let malicious_path = outside_dir.join("passwd");
    fs::write(&secure_path, "{}").expect("secure file written");
    fs::write(&malicious_path, "root:x:0:0").expect("outside file written");
    let permissions = ModPermissions {
        allowed_paths: Some(vec![allowed_dir.display().to_string()]),
        allowed_hosts: Some(vec!["api.zendesk.com".to_string()]),
    };

    validate_mod_filesystem_access("test.mod.cs", &secure_path, &permissions)
        .expect("path inside allowed prefix is accepted");

    let denied = validate_mod_filesystem_access("test.mod.cs", &malicious_path, &permissions);
    assert!(matches!(
        denied,
        Err(SecurityError::UnauthorizedAccess(message))
            if message.contains("test.mod.cs")
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let escaped_link = transcript_dir.join("escaped-passwd");
        symlink(&malicious_path, &escaped_link).expect("escape symlink created");
        let denied = validate_mod_filesystem_access("test.mod.cs", &escaped_link, &permissions);
        assert!(matches!(
            denied,
            Err(SecurityError::UnauthorizedAccess(message))
                if message.contains("test.mod.cs")
        ));
    }
}
