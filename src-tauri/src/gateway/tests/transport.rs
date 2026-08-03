use super::*;

#[test]
fn configure_channel_tool_has_a_bounded_effectful_contract() {
    let schema = configure_channel_tool_schema();
    assert_eq!(
        schema.get("additionalProperties"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        schema.pointer("/properties/platform/enum"),
        Some(&json!(["telegram", "discord", "slack"]))
    );

    let validated = validate_configure_channel_tool(json!({
        "platform": " Telegram ",
        "credentials_json": "{\"botToken\":\"123:abc\"}",
        "owner_id": " 42 ",
        "is_active": true,
    }))
    .unwrap();
    assert!(validated.potentially_effectful);
    assert_eq!(
        validated.arguments.get("platform"),
        Some(&json!("telegram"))
    );
    assert_eq!(validated.arguments.get("owner_id"), Some(&json!("42")));
    assert!(validate_configure_channel_tool(json!({
        "platform": "matrix",
        "credentials_json": "{}",
        "owner_id": "42",
        "is_active": true,
    }))
    .is_err());
}

#[test]
fn configure_channel_registration_reaches_the_native_tool_registry() {
    if let Err(error) = register_task_tool() {
        assert_eq!(error, "task_tool_registration_duplicate");
    }
    assert!(crate::tools::task_tool_runtime::is_registered(
        "configure_channel"
    ));
    assert_eq!(
        crate::tools::task_tool_runtime::approval_tier("configure_channel"),
        Some(crate::tools::task_tool_runtime::TaskToolApprovalTier::Explicit)
    );
    let schema = crate::tools::task_tool_runtime::schema("configure_channel").unwrap();
    assert!(schema.pointer("/properties/credentials_json").is_some());
}

#[test]
fn gateway_platforms_are_closed_over_real_worker_implementations() {
    for platform in ["telegram", "discord", "slack"] {
        assert!(is_supported_gateway_platform(platform));
    }
    for platform in ["unknown", "matrix", "", "Telegram"] {
        assert!(!is_supported_gateway_platform(platform));
    }
}

#[test]
fn telegram_credentials_accept_camel_case_payload() {
    let config = ChannelConfigRecord {
        platform: "telegram".to_string(),
        label: "Telegram".to_string(),
        is_active: true,
        credentials_json: r#"{"botToken":"123:abc","ownerChatId":"42"}"#.to_string(),
        owner_id: None,
        updated_at_ms: 1,
    };

    let credentials = telegram_credentials_from_config(&config).unwrap();

    assert_eq!(credentials.bot_token, "123:abc");
    assert_eq!(credentials.owner_chat_id.as_deref(), Some("42"));
}

#[test]
fn discord_credentials_accept_api_key_and_allowlist_payload() {
    let config = ChannelConfigRecord {
        platform: "discord".to_string(),
        label: "Discord".to_string(),
        is_active: true,
        credentials_json: r#"{"apiKey":"bot-token","allowlistChannels":["chan-1"]}"#.to_string(),
        owner_id: Some("owner-1".to_string()),
        updated_at_ms: 1,
    };

    let credentials = discord_credentials_from_config(&config).unwrap();

    assert_eq!(credentials.bot_token, "bot-token");
    assert_eq!(credentials.owner_id.as_deref(), Some("owner-1"));
    assert!(credentials.allowlist_channels.contains("chan-1"));
}

#[test]
fn discord_message_requires_owner_and_allowlisted_channel() {
    let credentials = DiscordChannelCredentials {
        bot_token: "bot-token".to_string(),
        owner_id: Some("owner-1".to_string()),
        allowlist_channels: HashSet::from(["chan-1".to_string()]),
    };
    let payload = json!({
        "t": "MESSAGE_CREATE",
        "s": 7,
        "op": 0,
        "d": {
            "id": "message-1",
            "channel_id": "chan-1",
            "content": "status",
            "author": {
                "id": "owner-1",
                "username": "Owner"
            }
        }
    });

    let incoming = discord_message_to_gateway_message(&payload, &credentials).unwrap();

    assert_eq!(incoming.platform, "discord");
    assert_eq!(incoming.sender_id, "owner-1");
    assert_eq!(incoming.channel_id.as_deref(), Some("chan-1"));
    assert_eq!(incoming.body, "status");
}

#[test]
fn remote_chat_session_id_is_stable_and_sanitized() {
    assert_eq!(
        remote_chat_session_id("telegram", "-100 42"),
        "remote-telegram-100-42"
    );
}

#[test]
fn gateway_message_log_contains_metadata_but_never_body_content() {
    let message = GatewayIncomingMessage {
        platform: "telegram".to_string(),
        sender_id: "owner".to_string(),
        sender_display_name: None,
        channel_id: None,
        body: "private message body canary".to_string(),
        message_id: Some("event-123".to_string()),
        received_at_ms: 1,
        requested_actions: Vec::new(),
    };
    let fields = gateway_message_log_fields("telegram", &message);
    assert!(fields.contains("event_id_hash="));
    assert!(!fields.contains("event-123"));
    assert!(fields.contains("body_bytes=27"));
    assert!(fields.contains("correlation_hash="));
    assert!(!fields.contains("private"));
    assert!(!fields.contains("canary"));
}
