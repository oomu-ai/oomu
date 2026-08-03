use super::*;

#[test]
fn photos_reads_are_intercepted_only_for_the_trusted_builtin_namespace() {
    assert!(is_system_photos_tool(
        "macos_applescript",
        "read_system_photos"
    ));
    assert!(is_system_photos_tool(
        " MACOS_APPLESCRIPT ",
        " READ_SYSTEM_PHOTOS "
    ));
    assert!(!is_system_photos_tool(
        "macos_applescript",
        "read_apple_app_ui"
    ));
    assert!(!is_system_photos_tool(
        "untrusted_server",
        "read_system_photos"
    ));
}

#[test]
fn music_reads_are_intercepted_only_for_the_trusted_builtin_namespace() {
    assert!(is_system_music_tool(
        "macos_applescript",
        "read_system_music"
    ));
    assert!(is_system_music_tool(
        " MACOS_APPLESCRIPT ",
        " READ_SYSTEM_MUSIC "
    ));
    assert!(!is_system_music_tool(
        "macos_applescript",
        "read_apple_app_ui"
    ));
    assert!(!is_system_music_tool(
        "untrusted_server",
        "read_system_music"
    ));
}

#[test]
fn contacts_reads_are_intercepted_only_for_the_trusted_builtin_namespace() {
    assert!(is_system_contacts_tool(
        "macos_applescript",
        "read_system_contacts"
    ));
    assert!(is_system_contacts_tool(
        " MACOS_APPLESCRIPT ",
        " READ_SYSTEM_CONTACTS "
    ));
    assert!(!is_system_contacts_tool(
        "macos_applescript",
        "read_apple_app_ui"
    ));
    assert!(!is_system_contacts_tool(
        "untrusted_server",
        "read_system_contacts"
    ));
}

#[test]
fn indeterminate_eventkit_status_never_bypasses_native_permission_recovery() {
    let failure = NativeCalendarFailure {
        code: "calendar_permission_unavailable".to_string(),
        message: "Calendar authorization is unavailable.".to_string(),
        retryable: false,
    };
    assert!(!calendar_failure_allows_applescript_fallback(&failure));
}

#[tokio::test]
async fn write_tool_requires_permission_gateway_approval() {
    let registry = McpClientRegistry::default();
    registry.tool_catalog.lock().await.insert(
        "local_filesystem".to_string(),
        vec![McpTool {
            name: "write_file".to_string(),
            description: "Write a file".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: None,
            annotations: None,
            meta: None,
        }],
    );
    let arguments = serde_json::json!({
        "path": "out.txt",
        "content": "approved output"
    });

    let blocked = registry
        .execute_tool_with_approval("local_filesystem", "write_file", arguments.clone(), None)
        .await
        .expect_err("write tool execution is blocked without approval");
    assert_eq!(blocked.code, "mcp_permission_required");
    assert!(blocked.message.contains("FILE_WRITE"));

    let approval = registry
        .prepare_tool_approval("local_filesystem", "write_file", arguments.clone())
        .await
        .expect("approval request can be prepared")
        .expect("filesystem write tool requires approval");
    assert_eq!(approval.server_name, "local_filesystem");
    assert_eq!(approval.tool_name, "write_file");
    assert_eq!(approval.arguments, arguments);
    assert_eq!(approval.capability_risk_tier, "FILE_WRITE");
}

#[tokio::test]
async fn read_tool_bypasses_permission_gateway_approval() {
    let registry = McpClientRegistry::default();
    registry.tool_catalog.lock().await.insert(
        "local_filesystem".to_string(),
        vec![McpTool {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: None,
            annotations: None,
            meta: None,
        }],
    );
    let arguments = serde_json::json!({
        "path": "notes.txt"
    });

    let approval = registry
        .prepare_tool_approval("local_filesystem", "read_file", arguments.clone())
        .await
        .expect("approval preparation succeeds");
    assert!(
        approval.is_none(),
        "filesystem read tools should execute without a manual approval request"
    );
}

#[tokio::test]
async fn renderer_builtin_descriptor_is_opaque_and_resolves_only_to_trusted_config() {
    let registry = McpClientRegistry::default();
    let trusted = McpServerConfig {
        name: "local_filesystem".to_string(),
        command: "/private/app/bin/native-filesystem".to_string(),
        args: vec!["/private/app/resources/entrypoint".to_string()],
        env: HashMap::from([(
            "OOMU_MCP_SANDBOX_DIR".to_string(),
            "/Users/canary/private".to_string(),
        )]),
        transport: McpTransportConfig::Native,
    };
    assert_eq!(
        registry
            .register_trusted_server_configs(vec![trusted.clone()])
            .await,
        1
    );

    let public = trusted.public_builtin_descriptor();
    let encoded = serde_json::to_string(&public).expect("descriptor serializes");
    assert_eq!(public.command, MCP_BACKEND_MANAGED_COMMAND);
    assert!(public.args.is_empty());
    assert!(public.env.is_empty());
    assert!(!encoded.contains("/Users/canary"));
    assert!(!encoded.contains("/private/app"));
    assert_eq!(
        registry
            .resolve_renderer_connect_config(public.clone())
            .await
            .expect("backend-issued descriptor resolves"),
        trusted
    );

    let mut forged = public;
    forged.env.insert(
        "OOMU_MCP_SANDBOX_DIR".to_string(),
        std::path::MAIN_SEPARATOR.to_string(),
    );
    let error = registry
        .resolve_renderer_connect_config(forged)
        .await
        .expect_err("modified public descriptor fails closed");
    assert_eq!(error.code, "mcp_permission_required");
}

#[tokio::test]
async fn trusted_builtin_descriptor_is_immutable_for_the_process_lifetime() {
    let registry = McpClientRegistry::default();
    let trusted = McpServerConfig {
        name: MACOS_APPLESCRIPT_SERVER_NAME.to_string(),
        command: "/trusted/python3".to_string(),
        args: vec!["/trusted/mcp_applescript.py".to_string()],
        env: HashMap::from([("OOMU_MCP_ENV_ISOLATION".to_string(), "strict".to_string())]),
        transport: McpTransportConfig::Stdio,
    };
    assert_eq!(
        registry
            .register_trusted_server_configs([trusted.clone()])
            .await,
        1
    );

    let mut replacement = trusted.clone();
    replacement.command = "/replacement/python3".to_string();
    assert_eq!(
        registry
            .register_trusted_server_configs([replacement])
            .await,
        0,
        "a changed descriptor cannot replace process-rooted built-in trust"
    );
    let stored = registry
        .trusted_builtin_configs
        .lock()
        .await
        .get(MACOS_APPLESCRIPT_SERVER_NAME)
        .cloned();
    assert_eq!(stored, Some(trusted));
}

#[tokio::test]
async fn delete_tool_requires_permission_gateway_approval_and_executes_after_approval() {
    let registry = McpClientRegistry::default();
    let sandbox = std::env::temp_dir().join(format!(
        "mcp-approval-delete-{}-{}",
        std::process::id(),
        unix_time_ms()
    ));
    fs::create_dir_all(&sandbox).expect("sandbox creates");
    fs::write(sandbox.join("scratch.txt"), "temporary").expect("sandbox file writes");
    let config = McpServerConfig {
        name: "local_filesystem".to_string(),
        command: "oomu-native".to_string(),
        args: vec![],
        env: HashMap::from([(
            "OOMU_MCP_SANDBOX_DIR".to_string(),
            sandbox.display().to_string(),
        )]),
        transport: McpTransportConfig::Native,
    };
    registry
        .register_trusted_server_configs(vec![config.clone()])
        .await;
    registry
        .connect_server(config)
        .await
        .expect("native filesystem server connects");
    let arguments = serde_json::json!({"path": "scratch.txt"});

    let blocked = registry
        .execute_tool_with_approval("local_filesystem", "delete_file", arguments.clone(), None)
        .await
        .expect_err("delete tool execution is blocked without approval");
    assert_eq!(blocked.code, "mcp_permission_required");
    assert!(blocked.message.contains("FILE_WRITE"));

    let approval = registry
        .prepare_tool_approval("local_filesystem", "delete_file", arguments.clone())
        .await
        .expect("approval request can be prepared")
        .expect("filesystem delete tool requires approval");

    let result = registry
        .execute_tool_with_approval(
            "local_filesystem",
            "delete_file",
            arguments,
            Some(McpToolApproval {
                approval_token: approval.approval_token,
            }),
        )
        .await
        .expect("approved delete executes");

    assert!(!result.is_error);
    assert!(!sandbox.join("scratch.txt").exists());
    assert!(result.content.iter().any(|item| item
        .get("text")
        .and_then(Value::as_str)
        .is_some_and(|text| text.contains("Deleted file: scratch.txt"))));

    let _ = fs::remove_dir_all(sandbox);
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn trusted_builtin_applescript_connects_without_renderer_spawn_authority() {
    let python = python3().expect("python3 is required for the AppleScript MCP test");
    let server_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/mcp/mcp_applescript.py");
    assert!(
        server_path.is_file(),
        "AppleScript MCP server must exist at {}",
        server_path.display()
    );

    let registry = McpClientRegistry::default();
    let config = McpServerConfig {
        name: MACOS_APPLESCRIPT_SERVER_NAME.to_string(),
        command: python,
        args: vec![server_path.to_string_lossy().to_string()],
        env: HashMap::new(),
        transport: McpTransportConfig::Stdio,
    };
    assert_eq!(
        registry.register_trusted_server_configs(vec![config]).await,
        1
    );

    timeout(
        Duration::from_secs(5),
        registry.ensure_server_connected(MACOS_APPLESCRIPT_SERVER_NAME),
    )
    .await
    .expect("trusted AppleScript helper connects before timeout")
    .expect("trusted AppleScript helper connects from its backend descriptor");

    let tools = registry
        .list_tools(MACOS_APPLESCRIPT_SERVER_NAME)
        .await
        .expect("trusted AppleScript tools remain available");
    assert!(tools
        .iter()
        .any(|tool| tool.name == READ_SYSTEM_EMAILS_TOOL_NAME));
    assert!(tools.iter().any(|tool| tool.name == "draft_system_email"));
}

#[tokio::test]
async fn invalid_outbound_arguments_never_generate_an_approval() {
    let registry = McpClientRegistry::default();
    registry.tool_catalog.lock().await.insert(
        "untrusted".to_string(),
        vec![McpTool {
            name: "do_thing".to_string(),
            description: String::new(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: None,
            annotations: None,
            meta: None,
        }],
    );
    let oversized = serde_json::json!({
        "content": "x".repeat(MCP_MAX_JSON_STRING_BYTES + 1)
    });
    let error = registry
        .prepare_tool_approval("untrusted", "do_thing", oversized)
        .await
        .expect_err("oversized outbound arguments fail before approval");
    assert!(error.message.contains("field limit"));
    assert!(registry.pending_tool_approvals.lock().await.is_empty());

    let mut deep = Value::Null;
    for _ in 0..=MCP_MAX_JSON_DEPTH {
        deep = serde_json::json!({"nested": deep});
    }
    registry
        .prepare_tool_approval("untrusted", "do_thing", deep)
        .await
        .expect_err("deep outbound arguments fail before approval");
    assert!(registry.pending_tool_approvals.lock().await.is_empty());
}

#[tokio::test]
async fn remote_tool_approval_cannot_be_swapped_to_a_replacement_session() {
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let (first_endpoint, first_port, first_server) = spawn_recording_disposable_mcp_http_server(
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
        first_calls.clone(),
    )
    .await;
    let (second_endpoint, second_port, second_server) = spawn_recording_disposable_mcp_http_server(
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
        second_calls.clone(),
    )
    .await;
    let first_config = McpServerConfig {
        name: "session_swap_remote".to_string(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        transport: McpTransportConfig::Http {
            url: first_endpoint.clone(),
            local_origin_grant: Some(crate::network_policy::LocalOriginGrant {
                exact_loopback_port: first_port,
            }),
        },
    };
    let second_config = McpServerConfig {
        name: "session_swap_remote".to_string(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        transport: McpTransportConfig::Http {
            url: second_endpoint.clone(),
            local_origin_grant: Some(crate::network_policy::LocalOriginGrant {
                exact_loopback_port: second_port,
            }),
        },
    };
    let first_destination = crate::network_policy::resolve_destination(
        &first_endpoint,
        crate::network_policy::DestinationTransport::RemoteMcpHttp,
        first_config.transport.local_origin_grant(),
    )
    .await
    .unwrap();
    let second_destination = crate::network_policy::resolve_destination(
        &second_endpoint,
        crate::network_policy::DestinationTransport::RemoteMcpHttp,
        second_config.transport.local_origin_grant(),
    )
    .await
    .unwrap();
    let registry = McpClientRegistry::default();
    registry
        .connect_server_with_authorization(
            first_config.clone(),
            McpSpawnAuthorization::shield_approved(&first_config, Some(first_destination)),
        )
        .await
        .unwrap();
    let first_session = registry.session("session_swap_remote").await.unwrap();
    let arguments = serde_json::json!({"query": "approved-for-first"});
    let approval = registry
        .prepare_remote_tool_approval_after_native_shield_for_test(
            "session_swap_remote",
            "do_thing",
            arguments.clone(),
        )
        .await
        .unwrap()
        .unwrap();
    let _stale_approval = registry
        .prepare_remote_tool_approval_after_native_shield_for_test(
            "session_swap_remote",
            "do_thing",
            serde_json::json!({"query": "must-be-purged"}),
        )
        .await
        .unwrap()
        .unwrap();

    let hook = RemoteToolExecutionTestHook::new();
    *registry.remote_tool_execution_test_hook.lock().await = Some(hook.clone());
    let execution_registry = registry.clone();
    let execution = tokio::spawn(async move {
        execution_registry
            .execute_tool_with_approval(
                "session_swap_remote",
                "do_thing",
                arguments,
                Some(McpToolApproval {
                    approval_token: approval.approval_token,
                }),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), hook.entered.notified())
        .await
        .expect("execution must pause after exact-session approval validation");

    registry
        .connect_server_with_authorization(
            second_config.clone(),
            McpSpawnAuthorization::shield_approved(&second_config, Some(second_destination)),
        )
        .await
        .unwrap();
    assert!(first_session.remote_cancellation.load(Ordering::Acquire));
    assert!(registry.pending_tool_approvals.lock().await.is_empty());
    let active = registry.session("session_swap_remote").await.unwrap();
    assert!(!Arc::ptr_eq(&active, &first_session));

    hook.release.notify_one();
    let error = tokio::time::timeout(Duration::from_secs(1), execution)
        .await
        .expect("replaced-session execution must terminate promptly")
        .unwrap()
        .expect_err("approval for the first session cannot execute on the replacement");
    assert!(matches!(
        error.code,
        "mcp_cancelled" | "mcp_permission_required"
    ));
    assert_eq!(first_calls.load(AtomicOrdering::Acquire), 0);
    assert_eq!(
        second_calls.load(AtomicOrdering::Acquire),
        0,
        "the replacement origin must never receive the approved tools/call"
    );

    first_server.abort();
    second_server.abort();
}
