use super::*;

#[test]
fn self_config_patch_rejects_model_and_provider_mutation() {
    let patch = AgentSelfConfigPatch {
        context_limit: Some(4096),
        model_id: Some(serde_json::json!("gemini-3.5-flash")),
        provider_id: Some(serde_json::json!("google")),
        ..AgentSelfConfigPatch::default()
    };

    let error =
        validate_self_config_patch_shape(&patch).expect_err("model/provider mutation is rejected");

    assert_eq!(error.code, "agent_configuration_authorization_block");
    assert!(error
        .message
        .contains("cannot mutate active model or provider"));
}

#[test]
fn provider_config_rejects_local_auto_route_target() {
    let manager = temporary_manager("local-auto-route-target");
    let mut local = provider_config("local", "local_model", "gemma-4-2b");
    local.auto_route_target = true;

    let error = manager
        .upsert_provider_config(local)
        .expect_err("local auto-route target is rejected");

    assert!(error
        .to_string()
        .contains("auto_route_target_local_provider_rejected"));
    let _ = fs::remove_file(manager.db_path.as_ref());
}

#[test]
fn provider_config_auto_route_target_is_mutually_exclusive() {
    let manager = temporary_manager("auto-route-target-exclusive");
    let mut gemini = provider_config("gemini", "google", "gemini-3.5-flash");
    gemini.auto_route_target = true;
    manager
        .upsert_provider_config(gemini)
        .expect("gemini target saves");
    let mut claude = provider_config("claude", "anthropic", "claude-sonnet-4-20250514");
    claude.auto_route_target = true;
    manager
        .upsert_provider_config(claude)
        .expect("claude target saves");

    let providers = manager
        .select_provider_configs()
        .expect("provider configs list");
    let active_targets = providers
        .iter()
        .filter(|provider| provider.auto_route_target)
        .collect::<Vec<_>>();

    assert_eq!(active_targets.len(), 1);
    assert_eq!(active_targets[0].id, "claude");
    assert!(
        !providers
            .iter()
            .find(|provider| provider.id == "gemini")
            .expect("gemini config exists")
            .auto_route_target
    );
    let _ = fs::remove_file(manager.db_path.as_ref());
}

#[test]
fn provider_secret_is_keychain_only_and_never_serialized() {
    let manager = temporary_manager("provider-keychain-only");
    let mut provider = provider_config("provider-keychain-canary-id", "openai", "gpt-5.5");
    provider.api_key = Some("provider-raw-secret-canary".to_string());
    let saved = manager.upsert_provider_config(provider).unwrap();
    assert!(saved.api_key.is_none());
    assert!(saved.credential_configured);
    let serialized = serde_json::to_string(&saved).unwrap();
    assert!(!serialized.contains("provider-raw-secret-canary"));
    assert!(!serialized.contains("\"apiKey\":"));

    let connection = manager.open_connection().unwrap();
    let stored: Option<String> = connection
        .query_row(
            "SELECT api_key FROM provider_configs WHERE id = ?1",
            params!["provider-keychain-canary-id"],
            |row| row.get(0),
        )
        .unwrap();
    assert!(stored.is_none());
    let internal = manager
        .select_provider_config("provider-keychain-canary-id")
        .unwrap()
        .unwrap();
    assert_eq!(
        internal.api_key.as_deref(),
        Some("provider-raw-secret-canary")
    );
    manager
        .remove_provider_config("provider-keychain-canary-id")
        .unwrap();
    let _ = fs::remove_file(manager.db_path.as_ref());
}

#[test]
fn known_provider_rejects_origin_mutation_before_keychain_secret_can_be_retargeted() {
    let manager = temporary_manager("provider-known-origin-bound");
    let mut provider = provider_config("provider-origin-bound-id", "openai", "gpt-5.5");
    provider.api_key = Some("known-provider-origin-secret".to_string());
    let mut saved = manager.upsert_provider_config(provider).unwrap();
    saved.base_url = "https://credential-sink.example/v1".to_string();
    saved.api_key = None;

    let error = manager
        .upsert_provider_config(saved)
        .unwrap_err()
        .to_string();
    assert!(error.contains("provider_origin_policy_rejected"));
    let retained = manager
        .select_provider_config("provider-origin-bound-id")
        .unwrap()
        .unwrap();
    assert_eq!(retained.base_url, "https://api.openai.com/v1");
    assert_eq!(
        retained.api_key.as_deref(),
        Some("known-provider-origin-secret")
    );
    manager
        .remove_provider_config("provider-origin-bound-id")
        .unwrap();
    let _ = fs::remove_file(manager.db_path.as_ref());
}

#[test]
fn canonical_2026_providers_bind_credentials_to_their_native_origins() {
    for (provider_id, base_url, expected_origin) in [
        (
            "qwen",
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "https://dashscope.aliyuncs.com",
        ),
        (
            "qwen_us",
            "https://dashscope-us.aliyuncs.com/compatible-mode/v1",
            "https://dashscope-us.aliyuncs.com",
        ),
        ("zai", "https://api.z.ai/api/paas/v4", "https://api.z.ai"),
        (
            "zai_coding",
            "https://api.z.ai/api/coding/paas/v4",
            "https://api.z.ai",
        ),
        (
            "zhipu",
            "https://open.bigmodel.cn/api/paas/v4",
            "https://open.bigmodel.cn",
        ),
        (
            "moonshot",
            "https://api.moonshot.cn/v1",
            "https://api.moonshot.cn",
        ),
        (
            "moonshot_global",
            "https://api.moonshot.ai/v1",
            "https://api.moonshot.ai",
        ),
        (
            "synthetic",
            "https://api.synthetic.ai/v1",
            "https://api.synthetic.ai",
        ),
        ("x-ai", "https://api.x.ai/v1", "https://api.x.ai"),
    ] {
        assert_eq!(
            canonical_provider_secret_origin(provider_id, base_url).unwrap(),
            expected_origin
        );
    }
}

#[test]
fn provider_secret_is_revoked_when_provider_or_custom_origin_changes_without_reentry() {
    let manager = temporary_manager("provider-secret-scope-change");
    let mut provider = provider_config("provider-scope-id", "openai", "gpt-5.5");
    provider.api_key = Some("provider-scope-secret".to_string());
    let mut saved = manager.upsert_provider_config(provider).unwrap();

    saved.provider_id = "custom".to_string();
    saved.base_url = "https://custom-one.example/v1".to_string();
    saved.api_key = None;
    let custom = manager.upsert_provider_config(saved).unwrap();
    assert!(!custom.credential_configured);
    assert!(manager
        .select_provider_config("provider-scope-id")
        .unwrap()
        .unwrap()
        .api_key
        .is_none());

    let mut custom_with_secret = custom;
    custom_with_secret.api_key = Some("custom-origin-secret".to_string());
    let mut custom_saved = manager.upsert_provider_config(custom_with_secret).unwrap();
    assert!(custom_saved.credential_configured);
    custom_saved.base_url = "https://custom-two.example/v1".to_string();
    custom_saved.api_key = None;
    let moved = manager.upsert_provider_config(custom_saved).unwrap();
    assert!(!moved.credential_configured);
    assert!(manager
        .select_provider_config("provider-scope-id")
        .unwrap()
        .unwrap()
        .api_key
        .is_none());
    manager.remove_provider_config("provider-scope-id").unwrap();
    let _ = fs::remove_file(manager.db_path.as_ref());
}

#[test]
fn configured_provider_debug_output_never_contains_secret_or_raw_origin() {
    let mut provider = provider_config("provider-debug-id", "custom", "custom-model");
    provider.base_url = "https://private-provider.example/v1".to_string();
    provider.api_key = Some("provider-debug-secret".to_string());
    let debug = format!("{provider:?}");
    assert!(!debug.contains("provider-debug-secret"));
    assert!(!debug.contains("private-provider.example"));
    assert!(debug.contains("[redacted]"));
}

#[test]
fn startup_alignment_repairs_missing_local_agent_model() {
    let model_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/models");
    if !model_root
        .join(crate::gemma::PREFERRED_LOCAL_MODEL_ID)
        .is_dir()
    {
        return;
    }

    let manager = temporary_manager("local-model-alignment");
    let now = unix_time_ms();
    manager
        .open_connection()
        .expect("open temporary database")
        .execute(
            "
                INSERT INTO agent_configs (
                    id, name, system_prompt, model_id, provider_id, description,
                    image, personality_profile, status, created_at_ms, updated_at_ms
                )
                VALUES ('agent-oomu', 'OOMU', 'Be helpful.', 'gemma-4-2b',
                        'local_model', '', NULL, '{}', 'active', ?1, ?1)
                ",
            params![now],
        )
        .expect("insert stale local agent");
    manager
        .open_connection()
        .expect("open temporary database")
        .execute(
            "
                INSERT INTO agent_configs (
                    id, name, system_prompt, model_id, provider_id, description,
                    image, personality_profile, status, created_at_ms, updated_at_ms
                )
                VALUES ('agent-remote', 'Remote', 'Be helpful.', 'gpt-4.1',
                        'chat_gpt', '', NULL, '{}', 'active', ?1, ?1)
                ",
            params![now],
        )
        .expect("insert remote agent");
    let expected_updates = manager
        .select_agent_configs()
        .expect("configs before alignment")
        .into_iter()
        .filter(|agent| {
            matches!(
                agent
                    .provider_id
                    .replace('-', "_")
                    .to_ascii_lowercase()
                    .as_str(),
                "local" | "local_model" | "local_gemma"
            ) && agent.model_id == "gemma-4-2b"
        })
        .count();

    let updated = manager
        .align_local_model_assignments(&model_root)
        .expect("align local agent models");
    let model_id = manager
        .open_connection()
        .expect("open temporary database")
        .query_row(
            "SELECT model_id FROM agent_configs WHERE id = 'agent-oomu'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("read aligned model id");

    assert_eq!(updated, expected_updates);
    assert_eq!(model_id, crate::gemma::PREFERRED_LOCAL_MODEL_ID);
    let remote_model_id = manager
        .open_connection()
        .expect("open temporary database")
        .query_row(
            "SELECT model_id FROM agent_configs WHERE id = 'agent-remote'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("read remote model id");
    assert_eq!(remote_model_id, "gpt-4.1");
    let _ = fs::remove_file(manager.db_path.as_ref());
}

#[test]
fn most_recent_local_model_id_prefers_active_local_assignments() {
    let manager = temporary_manager("recent-local-model");
    let now = unix_time_ms();
    let connection = manager.open_connection().expect("open temporary database");
    connection
        .execute(
            "
                INSERT INTO agent_configs (
                    id, name, system_prompt, model_id, provider_id, description,
                    image, personality_profile, status, created_at_ms, updated_at_ms
                )
                VALUES ('agent-remote', 'Remote', 'Be helpful.', 'gpt-4.1',
                        'chat_gpt', '', NULL, '{}', 'active', ?1, ?1)
                ",
            params![now + 30],
        )
        .expect("insert remote agent");
    connection
        .execute(
            "
                INSERT INTO agent_configs (
                    id, name, system_prompt, model_id, provider_id, description,
                    image, personality_profile, status, created_at_ms, updated_at_ms
                )
                VALUES ('agent-archived', 'Archived', 'Be helpful.', 'gemma-4-12b',
                        'local_model', '', NULL, '{}', 'archived', ?1, ?2)
                ",
            params![now, now + 20],
        )
        .expect("insert archived local agent");
    connection
        .execute(
            "
                INSERT INTO agent_configs (
                    id, name, system_prompt, model_id, provider_id, description,
                    image, personality_profile, status, created_at_ms, updated_at_ms
                )
                VALUES ('agent-active', 'Active', 'Be helpful.', 'gemma-4-2b',
                        'local-model', '', NULL, '{}', 'active', ?1, ?2)
                ",
            params![now, now + 10],
        )
        .expect("insert active local agent");

    let model_id = manager
        .most_recent_local_model_id()
        .expect("read recent local model");

    assert_eq!(model_id.as_deref(), Some("gemma-4-2b"));
    let _ = fs::remove_file(manager.db_path.as_ref());
}
