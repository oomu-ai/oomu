use super::*;

#[test]
fn disabling_channel_preserves_credentials_for_reenable() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_channels_disable_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    let save_request = SaveChannelConfigRequest {
        platform: "slack".to_string(),
        is_active: true,
        credentials_json: Some(
            r#"{"connectorId":"connector-test","allowlistChannels":["C123"],"secret":"channel-raw-canary"}"#.to_string(),
        ),
        owner_id: Some("U123".to_string()),
    };
    assert!(!format!("{save_request:?}").contains("channel-raw-canary"));
    let saved = engine.upsert_channel_config(save_request).unwrap();
    let summary = ChannelConfigSummary::from(&saved);
    let serialized_summary = serde_json::to_string(&summary).unwrap();
    assert!(summary.credential_configured);
    assert!(!serialized_summary.contains("channel-raw-canary"));
    assert!(!serialized_summary.contains("credentialsJson"));
    assert!(!format!("{saved:?}").contains("channel-raw-canary"));
    let persisted: String = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT credentials_json FROM channel_configs WHERE platform = 'slack'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(is_channel_credential_marker(&persisted));
    assert!(!persisted.contains("channel-raw-canary"));
    let disabled = engine
        .upsert_channel_config(SaveChannelConfigRequest {
            platform: "slack".to_string(),
            is_active: false,
            credentials_json: None,
            owner_id: None,
        })
        .unwrap();

    assert!(!disabled.is_active);
    assert!(disabled.credentials_json.contains("channel-raw-canary"));
    assert_eq!(disabled.owner_id.as_deref(), Some("U123"));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn sovereign_trust_dashboard_lists_and_revokes_policy() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_trust_dashboard_{}", unix_time_ms()));
    let trusted_dir = temp_dir.join("trusted");
    std::fs::create_dir_all(&trusted_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();

    let policy_id = engine
        .upsert_sovereign_trust_policy(
            trusted_dir.to_str().unwrap(),
            &[
                SovereignTrustToolCategory::ExternalWrites,
                SovereignTrustToolCategory::ShellCommands,
            ],
            SovereignTrustPermissionLevel::GlobalTrust,
            None,
            Some(256),
            Some(8.0),
        )
        .unwrap();
    let active_session_id = engine
        .activate_sovereign_trust_session(
            "session-dashboard",
            trusted_dir.to_str().unwrap(),
            &[SovereignTrustToolCategory::ExternalWrites],
            Some(unix_time_ms() + SOVEREIGN_TRUST_SESSION_DURATION_MS),
            None,
            None,
        )
        .unwrap();

    let dashboard = engine.select_sovereign_trust_dashboard(10).unwrap();
    assert_eq!(dashboard.policies.len(), 1);
    assert_eq!(dashboard.policies[0].id, policy_id);
    assert!(dashboard.policies[0]
        .allowed_tool_categories
        .contains(&"shell_commands".to_string()));
    assert_eq!(dashboard.active_sessions.len(), 1);
    assert_eq!(dashboard.active_sessions[0].id, active_session_id);

    let affected_rows = engine.revoke_sovereign_trust_policy(policy_id).unwrap();
    assert!(affected_rows >= 1);
    let dashboard = engine.select_sovereign_trust_dashboard(10).unwrap();
    assert!(dashboard.policies.is_empty());
    assert!(dashboard.active_sessions.is_empty());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn sovereign_trust_audit_events_identify_auto_and_manual_approvals() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_trust_audit_{}", unix_time_ms()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let engine = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    let auto_output = json!({
            "operation": "shell_command",
            "status": "completed",
            "message": "Shield Gate approved the shell command.",
            "claims": [
                "CLAIM shield_gate_approved_shell_command exit_status=0 timed_out=false",
                "CLAIM sovereign_trust_auto_approved tier=global_trust scope=/tmp requested_scope=~/Projects/OOMU estimated_token_cost=1 reserved_cpu_seconds=0.100 observed_elapsed_wall_seconds=0.025"
            ],
            "verified": false
        })
        .to_string();
    let manual_output = json!({
        "operation": "file_write",
        "status": "completed",
        "message": "Shield Gate approved and wrote 4 byte(s).",
        "claims": [
            "CLAIM shield_gate_approved_external_write path=/tmp/note.md min_bytes=4"
        ],
        "verified": false
    })
    .to_string();

    engine
        .insert_action(
            "direct-command",
            "shell_command",
            r#"{"kind":"shell_command","path":"/tmp"}"#,
            Some(&auto_output),
            "completed",
        )
        .unwrap();
    engine
        .insert_action(
            "direct-command",
            "file_write",
            r#"{"kind":"file_write","path":"/tmp/note.md"}"#,
            Some(&manual_output),
            "completed",
        )
        .unwrap();

    let events = engine.select_sovereign_trust_audit_events(10).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].authorization_mode, "manual_popup");
    assert_eq!(events[0].trust_tier, None);
    assert_eq!(events[0].target_path.as_deref(), Some("/tmp/note.md"));
    assert_eq!(events[1].authorization_mode, "global_trust_auto");
    assert_eq!(events[1].trust_tier.as_deref(), Some("global_trust"));
    assert_eq!(events[1].execution_hash.len(), 64);

    let _ = std::fs::remove_dir_all(temp_dir);
}
