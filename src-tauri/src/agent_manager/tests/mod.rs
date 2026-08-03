use super::*;

fn temp_agent_import_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "oomu-agent-import-{label}-{}-{}",
        std::process::id(),
        generate_uuid_v4()
    ))
}

fn temporary_manager(test_name: &str) -> AgentManager {
    let db_path = std::env::temp_dir().join(format!(
        "oomu-{test_name}-{}-{}.db",
        std::process::id(),
        unix_time_ms()
    ));
    let manager = AgentManager {
        db_path: Arc::new(db_path),
        write_lock: Arc::new(Mutex::new(())),
    };
    manager
        .run_migrations()
        .expect("prepare temporary database");
    manager
}

fn configured_agent(personality_profile: serde_json::Value) -> AgentConfig {
    AgentConfig {
        id: "agent-test".to_string(),
        name: "Avery".to_string(),
        system_prompt: "Coordinate the user's work and keep every recommendation practical."
            .to_string(),
        model_id: "gemma-4-2b".to_string(),
        provider_id: "local_model".to_string(),
        description: "A grounded coordination partner.".to_string(),
        image: None,
        personality_profile: personality_profile.to_string(),
        favorited: false,
        status: AgentConfigStatus::Active,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn save_request(id: &str) -> SaveAgentConfigRequest {
    SaveAgentConfigRequest {
        id: id.to_string(),
        name: "Avery".to_string(),
        system_prompt: "Coordinate the user's work.".to_string(),
        model_id: "gemma-4-2b".to_string(),
        provider_id: Some("local_model".to_string()),
        description: Some("A grounded coordination partner.".to_string()),
        image: None,
        personality_profile: Some(serde_json::json!({})),
        favorited: Some(false),
        status: Some(AgentConfigStatus::Active),
    }
}

fn provider_config(id: &str, provider_id: &str, model_id: &str) -> ConfiguredProvider {
    let base_url = match provider_id {
        "local_model" => "",
        "openai" => "https://api.openai.com/v1",
        "google" => "https://generativelanguage.googleapis.com/v1beta",
        "anthropic" => "https://api.anthropic.com/v1",
        "custom" => "https://custom.example.test/v1",
        _ => "https://api.example.test/v1",
    };
    ConfiguredProvider {
        id: id.to_string(),
        provider_id: provider_id.to_string(),
        provider_name: provider_id.to_string(),
        auth_method: if provider_id == "local_model" {
            "custom".to_string()
        } else {
            "api_key".to_string()
        },
        base_url: base_url.to_string(),
        api_key_label: "TEST_API_KEY".to_string(),
        api_key: None,
        credential_configured: false,
        custom_model_ids: model_id.to_string(),
        auto_route_target: false,
        created_at_ms: 0,
        updated_at_ms: 0,
    }
}

fn import_request(agent_name: &str, personality_template: &str) -> ExecuteAgentImportRequest {
    ExecuteAgentImportRequest {
        grant_id: "import-test-grant".to_string(),
        scan_token: "scan-test-token".to_string(),
        keys_to_import: vec!["soul".to_string()],
        agent_name: agent_name.to_string(),
        agent_description: "A McKinsey-grade strategic partner and systems architect.".to_string(),
        model_id: "gemma-4-2b".to_string(),
        provider_id: "local_model".to_string(),
        personality_template: personality_template.to_string(),
        target_agent_id: None,
    }
}

mod import;
mod persistence;
mod persona;
mod provider;
