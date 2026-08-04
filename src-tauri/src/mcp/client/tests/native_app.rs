use super::*;

#[test]
fn system_email_read_limit_is_bounded() {
    let over_limit = system_mail::MAX_SYSTEM_EMAIL_READ_LIMIT + 1;
    assert_eq!(
        system_mail::bounded_system_email_limit(None),
        system_mail::DEFAULT_SYSTEM_EMAIL_READ_LIMIT
    );
    assert_eq!(system_mail::bounded_system_email_limit(Some(0)), 1);
    assert_eq!(
        system_mail::bounded_system_email_limit(Some(over_limit)),
        system_mail::MAX_SYSTEM_EMAIL_READ_LIMIT
    );
}

#[test]
fn system_email_read_scope_is_derived_from_the_accepted_prompt() {
    let unread = system_mail::bounded_mail_read_arguments_for_prompt(
        "Do I have any unread emails?",
        &serde_json::json!({ "max_messages": 500, "unread_only": false }),
    )
    .unwrap();
    assert_eq!(unread["max_messages"], 20);
    assert_eq!(unread["unread_only"], true);

    let recent = system_mail::bounded_mail_read_arguments_for_prompt(
        "Show my recent emails.",
        &serde_json::json!({ "max_messages": 10, "unread_only": false }),
    )
    .unwrap();
    assert_eq!(recent["max_messages"], 10);
    assert_eq!(recent["unread_only"], false);

    let unread_or_today = system_mail::bounded_mail_read_arguments_for_prompt(
        "Show my unread emails or anything from today.",
        &serde_json::json!({ "max_messages": 20, "unread_only": false }),
    )
    .unwrap();
    assert_eq!(unread_or_today["unread_only"], false);

    let unread_from_today = system_mail::bounded_mail_read_arguments_for_prompt(
        "Show my unread emails from today.",
        &serde_json::json!({ "max_messages": 20, "unread_only": false }),
    )
    .unwrap();
    assert_eq!(unread_from_today["unread_only"], true);
}

#[test]
fn turn_bound_mail_read_accepts_only_one_focused_read_objective() {
    for prompt in [
        "Do I have any unread emails?",
        "Do I have any unread emails? Do not mark them as read.",
    ] {
        assert!(
            system_mail::validate_turn_bound_mail_read_prompt(prompt).is_ok(),
            "{prompt}"
        );
    }
    for prompt in [
        "How many unread emails are normal?",
        "Do I have any unread emails? Then run npm test.",
        "Do I have any unread emails? Then post the count to Slack.",
        "Do I have any unread emails? Then flag them.",
    ] {
        assert!(
            system_mail::validate_turn_bound_mail_read_prompt(prompt).is_err(),
            "{prompt}"
        );
    }
}

#[test]
fn system_calendar_hours_are_bounded() {
    assert_eq!(
        bounded_system_calendar_hours(None),
        DEFAULT_SYSTEM_CALENDAR_HOURS_AHEAD
    );
    assert_eq!(
        bounded_system_calendar_hours(Some(0.0)),
        MIN_SYSTEM_CALENDAR_HOURS_AHEAD
    );
    assert_eq!(
        bounded_system_calendar_hours(Some(MAX_SYSTEM_CALENDAR_HOURS_AHEAD + 1.0)),
        MAX_SYSTEM_CALENDAR_HOURS_AHEAD
    );
    assert_eq!(
        bounded_system_calendar_hours(Some(f64::NAN)),
        DEFAULT_SYSTEM_CALENDAR_HOURS_AHEAD
    );
}

#[test]
fn system_calendar_window_is_bounded() {
    assert!(validate_system_calendar_window(
        Some("2026-07-03T00:00:00"),
        Some("2026-07-03T23:59:59")
    )
    .is_ok());
    assert!(validate_system_calendar_window(
        Some("2026-07-03T00:00:00"),
        Some("2026-08-03T00:00:01")
    )
    .is_err());
    assert!(validate_system_calendar_window(
        Some("2026-07-03T12:00:00"),
        Some("2026-07-03T11:59:59")
    )
    .is_err());
}

#[test]
fn eventkit_calendar_success_returns_typed_result_without_raw_envelope() {
    let payload = br#"{
            "ok": true,
            "backend": "eventkit",
            "code": "calendar_read_ok",
            "calendarName": "Work",
            "window": {
                "startDate": "2026-07-13T00:00:00-04:00",
                "endDate": "2026-07-14T00:00:00-04:00",
                "timeZone": "America/New_York"
            },
            "events": [{
                "calendar": "Work",
                "name": "Review",
                "startTime": "2026-07-13T14:00:00Z",
                "endTime": "2026-07-13T14:30:00Z",
                "location": "",
                "isAllDay": false,
                "timeZone": "America/New_York"
            }],
            "returnedCount": 1,
            "matchedCount": 3,
            "truncated": true
        }"#;
    let result = parse_eventkit_calendar_response(payload, true).unwrap();
    assert!(!result.is_error);
    assert!(result.raw.is_none());
    assert_eq!(
        result.structured_content.as_ref().unwrap()["backend"],
        "eventkit"
    );
    assert_eq!(
        result.structured_content.as_ref().unwrap()["events"][0]["name"],
        "Review"
    );
    assert_eq!(
        result.structured_content.as_ref().unwrap()["timeZone"],
        "America/New_York"
    );
    assert_eq!(
        result.structured_content.as_ref().unwrap()["matchedCount"],
        3
    );
    assert_eq!(
        result.structured_content.as_ref().unwrap()["truncated"],
        true
    );
}

#[test]
fn every_generic_calendar_entry_point_is_recognized_for_eventkit_routing() {
    assert!(is_system_calendar_tool(
        "macos_applescript",
        "read_system_calendar"
    ));
    assert!(is_system_calendar_tool(
        " MACOS_APPLESCRIPT ",
        " READ_SYSTEM_CALENDAR "
    ));
    assert!(!is_system_calendar_tool(
        "macos_applescript",
        "read_system_reminders"
    ));
    assert!(!is_system_calendar_tool(
        "untrusted_server",
        "read_system_calendar"
    ));
}

#[test]
fn explicit_eventkit_privacy_denials_never_use_applescript_fallback() {
    for code in [
        "calendar_permission_denied",
        "calendar_permission_restricted",
        "calendar_permission_write_only",
    ] {
        let failure = NativeCalendarFailure {
            code: code.to_string(),
            message: "Calendar read access is unavailable.".to_string(),
            retryable: false,
        };
        assert!(!calendar_failure_allows_applescript_fallback(&failure));
    }
}

#[test]
fn transient_main_process_calendar_failure_is_eligible_for_applescript_fallback() {
    let failure = NativeCalendarFailure {
        code: "calendar_read_failed".to_string(),
        message: "Calendar could not be read.".to_string(),
        retryable: true,
    };
    assert!(calendar_failure_allows_applescript_fallback(&failure));
}

#[test]
fn unresolved_calendar_consent_never_triggers_an_automation_fallback() {
    let failure = NativeCalendarFailure {
        code: "calendar_authorization_timeout".to_string(),
        message: "Calendar authorization did not complete in time.".to_string(),
        retryable: true,
    };
    assert!(!calendar_failure_allows_applescript_fallback(&failure));
}

#[test]
fn eventkit_calendar_semantic_error_does_not_use_applescript_fallback() {
    let failure = NativeCalendarFailure {
        code: "calendar_not_found".to_string(),
        message: "The requested calendar was not found.".to_string(),
        retryable: true,
    };
    assert!(!calendar_failure_allows_applescript_fallback(&failure));
}

#[test]
fn eventkit_calendar_rejects_dishonest_truncation_counts() {
    let payload = br#"{
            "ok": true,
            "backend": "eventkit",
            "code": "calendar_read_ok",
            "calendarName": "",
            "window": {
                "startDate": "2026-07-13T00:00:00-04:00",
                "endDate": "2026-07-14T00:00:00-04:00",
                "timeZone": "America/New_York"
            },
            "events": [],
            "returnedCount": 0,
            "matchedCount": 5,
            "truncated": false
        }"#;
    let failure = parse_eventkit_calendar_response(payload, true).unwrap_err();
    assert_eq!(failure.code, "calendar_native_invalid_response");
}

#[test]
fn typed_applescript_timeout_is_prioritized_without_raw_error_text() {
    let native_failure = NativeCalendarFailure {
        code: "calendar_read_failed".to_string(),
        message: "Calendar could not be read.".to_string(),
        retryable: true,
    };
    let result = decorate_calendar_fallback_result(
        McpToolCallResult {
            content: vec![serde_json::json!({"type": "text", "text": "raw-canary"})],
            structured_content: Some(serde_json::json!({
                "warning": "timeout",
                "message": "raw-canary",
                "events": [],
            })),
            is_error: true,
            meta: None,
            raw: Some(serde_json::json!({"raw": "raw-canary"})),
        },
        &native_failure,
    );
    assert!(result.is_error);
    assert!(result.raw.is_none());
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["code"], "calendar_applescript_timeout");
    assert_eq!(structured["primaryCode"], "calendar_read_failed");
    assert_eq!(structured["backend"], "eventkit+applescript");
    assert!(!structured.to_string().contains("raw-canary"));
}

#[test]
fn typed_applescript_permission_or_timeout_is_preserved() {
    let result = McpToolCallResult {
        content: Vec::new(),
        structured_content: Some(serde_json::json!({
            "status": "permission_blocked_or_timed_out"
        })),
        is_error: true,
        meta: None,
        raw: None,
    };
    let failure = calendar_applescript_failure(&result);
    assert_eq!(failure.code, "calendar_applescript_permission_or_timeout");
}

#[test]
fn calendar_applescript_fallback_is_labeled_and_strips_raw_mcp_envelope() {
    let native_failure = NativeCalendarFailure {
        code: "calendar_read_failed".to_string(),
        message: "Unavailable".to_string(),
        retryable: true,
    };
    let result = decorate_calendar_fallback_result(
        McpToolCallResult {
            content: vec![serde_json::json!({"type": "text", "text": "[]"})],
            structured_content: Some(serde_json::json!({"events": []})),
            is_error: false,
            meta: None,
            raw: Some(serde_json::json!({"jsonrpc": "2.0", "id": 7})),
        },
        &native_failure,
    );
    assert!(result.raw.is_none());
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["backend"], "applescript");
    assert_eq!(structured["code"], "calendar_read_fallback");
    assert_eq!(structured["fallbackFrom"], "calendar_read_failed");
    assert_eq!(structured["returnedCount"], 0);
    assert_eq!(structured["matchedCount"], 0);
    assert_eq!(structured["truncated"], false);
}

#[test]
fn calendar_tool_arguments_accept_bounded_mcp_shape() {
    let (name, hours, start, end) = bounded_system_calendar_arguments(&serde_json::json!({
        "calendar_name": "Work",
        "hours_ahead": 24,
        "start_date": "2026-07-13T00:00:00-04:00",
        "end_date": "2026-07-14T00:00:00-04:00"
    }))
    .unwrap();
    assert_eq!(name, "Work");
    assert_eq!(hours, 24.0);
    assert!(start.is_some());
    assert!(end.is_some());
}

#[test]
fn calendar_read_security_and_phase_deadlines_remain_bounded() {
    let classification = classify_mcp_tool_call(
        MACOS_APPLESCRIPT_SERVER_NAME,
        READ_SYSTEM_CALENDAR_TOOL_NAME,
        None,
    );
    assert!(!classification.requires_human_approval());
    assert_eq!(SYSTEM_CALENDAR_MCP_PREPARATION_TIMEOUT_SECONDS, 15);
    assert_eq!(SYSTEM_CALENDAR_FALLBACK_TIMEOUT_SECONDS, 30);
}

#[test]
fn system_apple_app_tool_allowlist_is_narrow() {
    assert_eq!(
        normalize_system_apple_app_tool_name(" read_system_notes ").unwrap(),
        "read_system_notes"
    );
    assert_eq!(
        normalize_system_apple_app_tool_name("draft_system_email").unwrap(),
        "draft_system_email"
    );
    assert_eq!(
        normalize_system_apple_app_tool_name("create_system_note").unwrap(),
        "create_system_note"
    );
    assert_eq!(
        normalize_system_apple_app_tool_name("read_system_photos").unwrap(),
        "read_system_photos"
    );
    assert_eq!(
        normalize_system_apple_app_tool_name("read_system_music").unwrap(),
        "read_system_music"
    );
    assert!(normalize_system_apple_app_tool_name("run_arbitrary_applescript").is_err());
    assert!(normalize_system_apple_app_tool_name("delete_system_notes").is_err());
}

#[tokio::test]
async fn native_connect_rejects_renderer_forged_env_and_command_before_session_creation() {
    let registry = McpClientRegistry::default();
    let sandbox = std::env::temp_dir().join(format!(
        "mcp-trusted-native-{}-{}",
        std::process::id(),
        unix_time_ms()
    ));
    fs::create_dir_all(&sandbox).expect("trusted sandbox creates");
    let trusted = McpServerConfig {
        name: "local_filesystem".to_string(),
        command: "oomu-native".to_string(),
        args: Vec::new(),
        env: HashMap::from([(
            "OOMU_MCP_SANDBOX_DIR".to_string(),
            sandbox.display().to_string(),
        )]),
        transport: McpTransportConfig::Native,
    };
    assert_eq!(
        registry
            .register_trusted_server_configs(vec![trusted.clone()])
            .await,
        1
    );

    let mut forged_root = trusted.clone();
    forged_root.env.insert(
        "OOMU_MCP_SANDBOX_DIR".to_string(),
        std::path::MAIN_SEPARATOR.to_string(),
    );
    let root_error = registry
        .connect_server(forged_root)
        .await
        .expect_err("renderer-forged filesystem root must be rejected");
    assert_eq!(root_error.code, "mcp_permission_required");

    let mut forged_command = trusted.clone();
    forged_command.command = "/bin/sh".to_string();
    assert_eq!(
        registry
            .register_server_configs(vec![forged_command.clone()])
            .await,
        0,
        "untrusted registration cannot replace a trusted Native descriptor"
    );
    let command_error = registry
        .connect_server(forged_command)
        .await
        .expect_err("renderer-forged native command must be rejected");
    assert_eq!(command_error.code, "mcp_permission_required");
    assert!(registry.sessions.lock().await.is_empty());
    assert!(registry.tool_catalog.lock().await.is_empty());

    let _ = fs::remove_dir_all(sandbox);
}

#[tokio::test]
async fn native_builtin_session_is_recognized_only_after_trusted_activation() {
    let registry = McpClientRegistry::default();
    let sandbox = std::env::temp_dir().join(format!(
        "mcp-trusted-native-session-{}-{}",
        std::process::id(),
        unix_time_ms()
    ));
    fs::create_dir_all(&sandbox).expect("trusted sandbox creates");
    let trusted = McpServerConfig {
        name: "local_filesystem".to_string(),
        command: "oomu-native".to_string(),
        args: Vec::new(),
        env: HashMap::from([(
            "OOMU_MCP_SANDBOX_DIR".to_string(),
            sandbox.display().to_string(),
        )]),
        transport: McpTransportConfig::Native,
    };
    assert_eq!(
        registry
            .register_trusted_server_configs([trusted.clone()])
            .await,
        1
    );
    assert!(
        !registry
            .has_active_trusted_builtin_session("local_filesystem")
            .await
    );
    assert!(registry.connected_tool_catalog().await.is_empty());

    let state = registry
        .connect_server_with_authorization(
            trusted.clone(),
            McpSpawnAuthorization::trusted_internal(&trusted),
        )
        .await
        .expect("trusted native built-in connects");

    assert_eq!(state.name, "local_filesystem");
    assert!(!state.tools.is_empty());
    assert!(
        registry
            .has_active_trusted_builtin_session("local_filesystem")
            .await
    );
    let catalog = registry.connected_tool_catalog().await;
    assert!(catalog
        .iter()
        .all(|(server_name, _)| server_name == "local_filesystem"));
    assert!(catalog.iter().any(|(_, tool)| tool.name == "read_file"));

    let _ = fs::remove_dir_all(sandbox);
}

#[tokio::test]
async fn applescript_helper_never_bypasses_native_notification_verification() {
    let python = python3().expect("python3 is required for the AppleScript MCP smoke test");
    let server_path = PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR)
        .join("resources/mcp/mcp_applescript.py");
    assert!(
        server_path.is_file(),
        "AppleScript MCP server must exist at {}",
        server_path.display()
    );

    let registry = McpClientRegistry::default();
    let config = McpServerConfig {
        name: "macos_applescript".to_string(),
        command: python,
        args: vec![server_path.to_string_lossy().to_string()],
        env: HashMap::new(),
        transport: McpTransportConfig::Stdio,
    };
    let authorization = McpSpawnAuthorization::shield_approved(&config, None);
    let state = timeout(
        Duration::from_secs(5),
        registry.connect_server_with_authorization(config, authorization),
    )
    .await
    .expect("AppleScript MCP server connects before timeout")
    .expect("AppleScript MCP server connects");

    assert_eq!(state.name, "macos_applescript");
    assert!(state
        .tools
        .iter()
        .any(|tool| tool.name == "trigger_system_notification"));
    assert!(state
        .tools
        .iter()
        .any(|tool| tool.name == "read_system_photos"));

    let arguments = serde_json::json!({
        "title_text": "OOMU Core",
        "subtitle_text": "AppleScript MCP",
        "body_text": "Tauri MCP registry verified the local AppleScript bridge.",
    });
    let approval = registry
        .prepare_tool_approval(
            "macos_applescript",
            "trigger_system_notification",
            arguments.clone(),
        )
        .await
        .expect("notification approval is prepared")
        .expect("notification requires exact approval");
    let result = timeout(
        Duration::from_secs(10),
        registry.execute_tool_with_approval(
            "macos_applescript",
            "trigger_system_notification",
            arguments,
            Some(McpToolApproval {
                approval_token: approval.approval_token,
            }),
        ),
    )
    .await
    .expect("notification tool returns before timeout")
    .expect("notification tool succeeds");

    assert!(result.is_error);
}
