use super::*;

#[test]
fn remote_connection_shield_receipt_never_persists_endpoint_path_or_query() {
    let port = 48173;
    let endpoint = format!("http://127.0.0.1:{port}/private/mcp?token=remote-query-canary");
    let transport = McpTransportConfig::Http {
        url: endpoint,
        local_origin_grant: Some(crate::network_policy::LocalOriginGrant {
            exact_loopback_port: port,
        }),
    };
    let destination = crate::network_policy::resolve_destination_blocking(
        transport.endpoint().unwrap(),
        crate::network_policy::DestinationTransport::RemoteMcpHttp,
        transport.local_origin_grant(),
    )
    .unwrap();
    let config = McpServerConfig {
        name: "receipt_test".to_string(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        transport,
    };
    let target = mcp_connection_approval_target(&config, Some(&destination));
    let preview = mcp_approval_preview(&config, Some(&destination));
    assert_eq!(target, format!("http://127.0.0.1:{port}"));
    assert!(preview.contains(destination.binding_fingerprint()));
    assert!(!preview.contains("/private/mcp"));
    assert!(!preview.contains("remote-query-canary"));
}

#[tokio::test]
async fn sprint_304_registry_shutdown_stops_intake_and_clears_runtime_authority() {
    let registry = McpClientRegistry::default();
    let config = McpServerConfig {
        name: "sprint_304_native".to_string(),
        command: MCP_BACKEND_MANAGED_COMMAND.to_string(),
        args: Vec::new(),
        env: HashMap::new(),
        transport: McpTransportConfig::Native,
    };
    assert_eq!(
        registry
            .register_trusted_server_configs([config.clone()])
            .await,
        1
    );

    registry.shutdown_all().await.unwrap();

    assert_eq!(registry.register_trusted_server_configs([config]).await, 0);
    assert!(registry.sessions.lock().await.is_empty());
    assert!(registry.connecting_remote.lock().await.is_empty());
    assert!(registry.tool_catalog.lock().await.is_empty());
}

#[test]
fn stdio_connection_shield_receipt_redacts_cli_credentials_and_home_paths() {
    let config = McpServerConfig {
        name: "stdio receipt".to_string(),
        command: "/Users/Alex/private/bin/python3".to_string(),
        args: vec![
            "--token".to_string(),
            "separate-secret-canary".to_string(),
            "--api-key=inline-secret-canary".to_string(),
            "https://user:pass@example.test/private?access_token=url-secret-canary".to_string(),
            "/Users/Alex/private/server.py".to_string(),
        ],
        env: HashMap::from([("API_KEY".to_string(), "env-secret-canary".to_string())]),
        transport: McpTransportConfig::Stdio,
    };
    let preview = mcp_approval_preview(&config, None);
    let target = mcp_connection_approval_target(&config, None);
    assert!(preview.len() <= 4096);
    for canary in [
        "separate-secret-canary",
        "inline-secret-canary",
        "url-secret-canary",
        "env-secret-canary",
        "user:pass",
        "/Users/Alex/",
    ] {
        assert!(
            !preview.contains(canary),
            "receipt leaked {canary}: {preview}"
        );
        assert!(!target.contains(canary), "target leaked {canary}: {target}");
    }
    assert!(preview.contains("[redacted]"));
    assert!(preview.contains("[home]/private"));
}

#[test]
fn mcp_chat_turn_guard_starts_direct_turn_accepts_completed_and_rejects_deleted_origin() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_mcp_turn_guard_{}", unix_time_ms_u64()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let persistence = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    persistence
        .ensure_chat_session_with_id(
            "session-missing",
            crate::db::CreateChatSessionRequest {
                agent_id: "agent-missing".to_string(),
                provider_id: "provider-missing".to_string(),
                model_id: "model-missing".to_string(),
                title: Some("MCP guard test".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            },
        )
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "turn-missing".to_string(),
        generation_token: "generation-missing".to_string(),
        session_id: "session-missing".to_string(),
        agent_id: "agent-missing".to_string(),
        provider_id: "provider-missing".to_string(),
        model_id: "model-missing".to_string(),
        parent_turn_id: None,
        root_turn_id: "turn-missing".to_string(),
        turn_kind: "root".to_string(),
    };

    assert!(validate_mcp_chat_turn(&persistence, Some(&context)).is_ok());
    persistence
        .begin_or_claim_chat_turn_response(&context)
        .unwrap();
    assert!(validate_mcp_chat_turn(&persistence, Some(&context)).is_err());
    persistence.finish_chat_turn(&context, "completed").unwrap();
    assert!(validate_mcp_chat_turn(&persistence, Some(&context)).is_ok());
    persistence
        .delete_chat_session_by_id(&context.session_id)
        .unwrap();
    assert!(validate_mcp_chat_turn(&persistence, Some(&context)).is_err());
    assert!(validate_mcp_chat_turn(&persistence, None).is_ok());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn mail_read_prompt_is_loaded_from_the_exact_durable_turn() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu_mcp_mail_turn_binding_{}", unix_time_ms_u64()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let persistence = PersistenceEngine::initialize_at(temp_dir.join("state.sqlite")).unwrap();
    persistence
        .ensure_chat_session_with_id(
            "mail-session",
            crate::db::CreateChatSessionRequest {
                agent_id: "mail-agent".to_string(),
                provider_id: "mail-provider".to_string(),
                model_id: "mail-model".to_string(),
                title: Some("Mail binding test".to_string()),
                dynamic_routing_override: None,
                workspace_id: None,
            },
        )
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "mail-turn".to_string(),
        generation_token: "mail-generation".to_string(),
        session_id: "mail-session".to_string(),
        agent_id: "mail-agent".to_string(),
        provider_id: "mail-provider".to_string(),
        model_id: "mail-model".to_string(),
        parent_turn_id: None,
        root_turn_id: "mail-turn".to_string(),
        turn_kind: "root".to_string(),
    };
    persistence.begin_chat_turn(&context).unwrap();
    persistence
        .ensure_chat_turn_user_message(&context, "Do I have any unread emails?", "accepted")
        .unwrap();

    assert_eq!(
        system_mail::accepted_user_prompt_for_turn(&persistence, Some(&context)).unwrap(),
        "Do I have any unread emails?"
    );
    assert!(system_mail::accepted_user_prompt_for_turn(&persistence, None).is_err());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn conversational_project_reader_uses_only_the_attached_project_root() {
    let root = std::env::temp_dir().join(format!(
        "oomu-conversational-project-read-{}",
        unix_time_ms_u64()
    ));
    let approved_root = root.join("approved-project");
    let outside_root = root.join("outside-project");
    std::fs::create_dir_all(&approved_root).unwrap();
    std::fs::create_dir_all(&outside_root).unwrap();
    let approved_root = std::fs::canonicalize(approved_root).unwrap();
    let fixture = approved_root.join("Lab_Inventory.csv");
    std::fs::write(&fixture, "item,stock\nPipette tips,2\nGloves,24\n").unwrap();
    let outside = outside_root.join("private.csv");
    std::fs::write(&outside, "private\n").unwrap();
    let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project = crate::projects::repository::create(
        &persistence,
        crate::projects::CreateProjectRequest {
            name: "CSV review".to_string(),
            description: String::new(),
            data_policy: crate::projects::ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let session = persistence
        .ensure_chat_session(crate::db::CreateChatSessionRequest {
            agent_id: "project-reader-agent".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "project-reader-model".to_string(),
            title: Some("Project CSV read".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    crate::projects::repository::bind_record(
        &persistence,
        crate::projects::BindProjectRecordRequest {
            project_id: Some(project.project_id.clone()),
            record_kind: "chat_session".to_string(),
            record_id: session.id.clone(),
        },
    )
    .unwrap();
    persistence
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO project_sources(source_id,project_id,source_kind,canonical_path,grant_reference,grant_state,created_at_ms,updated_at_ms) VALUES ('conversation-source',?1,'local_folder',?2,?3,'active',1,1)",
            rusqlite::params![
                project.project_id,
                approved_root.to_string_lossy(),
                "a".repeat(64)
            ],
        )
        .unwrap();
    let accepted = crate::db::AcceptChatTurnRequest {
        turn_id: "project-reader-turn".to_string(),
        generation_token: "project-reader-generation".to_string(),
        session_id: session.id,
        agent_id: session.agent_id,
        provider_id: session.provider_id,
        model_id: session.model_id,
        parent_turn_id: None,
        root_turn_id: "project-reader-turn".to_string(),
        turn_kind: "root".to_string(),
        message: "Read Lab_Inventory.csv from this Project.".to_string(),
    };
    persistence.accept_chat_turn(accepted.clone()).unwrap();
    let context = accepted.persistence_context();

    assert_eq!(
        apple_command_execution::conversational_project_file_read_project_id(
            &persistence,
            Some(&context),
            "local_filesystem",
            "read_file",
        )
        .unwrap()
        .as_deref(),
        Some(project.project_id.as_str())
    );
    let arguments = serde_json::json!({"path": fixture.to_string_lossy()});
    let result = native_apple_receipts::execute(
        native_apple_receipts::spec_for("local_filesystem", "read_file", &arguments),
        Some(&context),
        &persistence,
        false,
        async {
            apple_command_execution::conversational_project_file_read(
                &persistence,
                &project.project_id,
                fixture.to_string_lossy().as_ref(),
            )
        },
    )
    .await
    .unwrap();
    let native_receipt = &result.meta.as_ref().unwrap()["oomuNativeExecutionReceipt"];
    assert_eq!(native_receipt["outcome"], "succeeded");
    assert_eq!(native_receipt["verified"], true);
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["code"], "project_file_read_ok");
    assert_eq!(structured["verified"], true);
    assert_eq!(structured["path"], "Lab_Inventory.csv");
    assert_eq!(structured["relativePath"], "Lab_Inventory.csv");
    assert!(!structured
        .to_string()
        .contains(approved_root.to_string_lossy().as_ref()));
    assert_eq!(
        structured["content"],
        "item,stock\nPipette tips,2\nGloves,24\n"
    );
    assert!(apple_command_execution::conversational_project_file_read(
        &persistence,
        &project.project_id,
        outside.to_string_lossy().as_ref(),
    )
    .is_err());
    assert!(
        apple_command_execution::conversational_project_file_read_path(
            &serde_json::json!({"path": "Lab_Inventory.csv", "unexpected": true}),
        )
        .is_err()
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn conversational_project_reader_does_not_claim_global_chats() {
    let root = std::env::temp_dir().join(format!(
        "oomu-global-conversational-file-read-{}",
        unix_time_ms_u64()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let session = persistence
        .ensure_chat_session(crate::db::CreateChatSessionRequest {
            agent_id: "global-reader-agent".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "global-reader-model".to_string(),
            title: Some("Global CSV read".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let context = ChatTurnPersistenceContext {
        turn_id: "global-reader-turn".to_string(),
        generation_token: "global-reader-generation".to_string(),
        session_id: session.id,
        agent_id: session.agent_id,
        provider_id: session.provider_id,
        model_id: session.model_id,
        parent_turn_id: None,
        root_turn_id: "global-reader-turn".to_string(),
        turn_kind: "root".to_string(),
    };

    assert!(
        apple_command_execution::conversational_project_file_read_project_id(
            &persistence,
            Some(&context),
            "local_filesystem",
            "read_file",
        )
        .unwrap()
        .is_none()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rejects_malformed_json_rpc_error_objects() {
    let application_error = parse_json_rpc_message(
            r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"denied","data":{"reason":"policy"}},"id":1}"#,
        )
        .expect("well-formed JSON-RPC application error parses");
    assert!(matches!(application_error, JsonRpcMessage::Response(_)));

    for malformed in [
        r#"{"jsonrpc":"2.0","error":"denied","id":1}"#,
        r#"{"jsonrpc":"2.0","error":null,"id":1}"#,
        r#"{"jsonrpc":"2.0","error":{"code":"-32000","message":"denied"},"id":1}"#,
        r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":7},"id":1}"#,
        r#"{"jsonrpc":"2.0","error":{"code":-32000},"id":1}"#,
    ] {
        let error = parse_json_rpc_message(malformed)
            .expect_err("malformed JSON-RPC error object must be terminal protocol input");
        assert_eq!(error.code, "mcp_protocol_error");
    }
}

#[test]
fn stdio_spawn_validation_rejects_relative_and_shell_operator_commands() {
    let relative = validate_mcp_binary_path("./server");
    assert_eq!(
        relative.expect_err("relative command is rejected").code,
        "mcp_permission_required"
    );

    let injected = validate_mcp_binary_path("/usr/bin/python3; touch /tmp/oomu");
    assert_eq!(
        injected.expect_err("operator command is rejected").code,
        "mcp_permission_required"
    );

    assert!(
        validate_mcp_binary_path("python3").is_ok(),
        "vetted runtime aliases are accepted for approval routing"
    );
}

#[test]
fn stdio_spawn_validation_rejects_shell_operator_arguments() {
    let executable = std::env::current_exe().expect("current test executable resolves");
    let config = McpServerConfig {
        name: "operator_arg".to_string(),
        command: executable.display().to_string(),
        args: vec!["--safe".to_string(), "value && injected".to_string()],
        env: HashMap::new(),
        transport: McpTransportConfig::Stdio,
    };

    let error = validate_mcp_stdio_server_config(&config)
        .expect_err("shell operator arguments are rejected");
    assert_eq!(error.code, "mcp_permission_required");
}

#[tokio::test]
async fn well_formed_remote_application_error_is_not_a_transport_failure() {
    let server = spawn_one_shot_http_server(
            "application/json",
            r#"{"jsonrpc":"2.0","error":{"code":-32001,"message":"tool denied"},"id":"application-error"}"#
                .to_string(),
        );
    let session = McpClientSession::spawn(McpServerConfig {
        name: "remote_application_error".to_string(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        transport: McpTransportConfig::Http {
            url: server.url.clone(),
            local_origin_grant: Some(crate::network_policy::LocalOriginGrant {
                exact_loopback_port: reqwest::Url::parse(&server.url).unwrap().port().unwrap(),
            }),
        },
    })
    .await
    .expect("remote HTTP session spawns");

    let response = session
        .send_request(JsonRpcRequest::new(
            "tools/call",
            serde_json::json!({"name": "denied"}),
            serde_json::json!("application-error"),
        ))
        .await
        .expect("well-formed JSON-RPC error crosses the transport as a response");
    assert!(response.error.is_some());
    assert!(!session.remote_cancellation.load(Ordering::Acquire));
    let application_error = json_rpc_response_result(response)
        .expect_err("application error remains an application-level failure");
    assert_eq!(application_error.code, "mcp_protocol_error");
    assert!(!session.remote_cancellation.load(Ordering::Acquire));

    server
        .received
        .recv_timeout(Duration::from_secs(5))
        .expect("mock server captures the request");
    server.handle.join().expect("mock server thread joins");
}

#[tokio::test]
async fn connect_server_initializes_and_lists_tools() {
    let python = python3().expect("python3 is required for the MCP stdio init test");
    let script_path = std::env::temp_dir().join(format!(
        "oomu-mcp-init-{}-{}.py",
        std::process::id(),
        unix_time_ms()
    ));
    fs::write(
        &script_path,
        r#"
import json
import sys

initialized = False

for line in sys.stdin:
    payload = json.loads(line)
    method = payload.get("method")
    if method == "initialize":
        print(json.dumps({
            "jsonrpc": "2.0",
            "result": {
                "protocolVersion": payload.get("params", {}).get("protocolVersion"),
                "capabilities": {"tools": {"listChanged": True}},
                "serverInfo": {"name": "mock_init", "version": "1.0.0"}
            },
            "id": payload.get("id")
        }), flush=True)
    elif method == "notifications/initialized":
        initialized = True
    elif method == "tools/list":
        print(json.dumps({
            "jsonrpc": "2.0",
            "result": {
                "tools": [{
                    "name": "read_status",
                    "description": "Read status",
                    "inputSchema": {"type": "object", "properties": {}},
                    "outputSchema": {"type": "object"},
                    "_meta": {"ui": {"resourceUri": "ui://status/view.html"}}
                }]
            },
            "id": payload.get("id")
        }), flush=True)
"#,
    )
    .expect("init script writes");

    let registry = McpClientRegistry::default();
    let config = McpServerConfig {
        name: "mock_init".to_string(),
        command: python,
        args: vec![script_path.to_string_lossy().to_string()],
        env: HashMap::new(),
        transport: McpTransportConfig::Stdio,
    };
    let authorization = McpSpawnAuthorization::shield_approved(&config, None);
    let state = timeout(
        Duration::from_secs(5),
        registry.connect_server_with_authorization(config, authorization),
    )
    .await
    .expect("connect completes before timeout")
    .expect("server connects");

    assert_eq!(state.name, "mock_init");
    assert_eq!(
        state.protocol_version.as_deref(),
        Some(MCP_PROTOCOL_VERSION)
    );
    assert_eq!(state.tools.len(), 1);
    assert_eq!(state.tools[0].name, "read_status");
    assert!(state.tools[0].output_schema.is_some());
    assert!(state.tools[0].meta.is_some());

    let search_results = registry
        .search_tools("status")
        .await
        .expect("tool catalog search succeeds");
    assert_eq!(search_results.len(), 1);
    assert_eq!(search_results[0].server_name, "mock_init");
    assert_eq!(search_results[0].name, "read_status");

    let details = registry
        .get_tool_details("mock_init", "read_status")
        .await
        .expect("tool details resolve from catalog");
    assert_eq!(details.name, "read_status");
    assert!(details.meta.is_some());

    let _ = fs::remove_file(script_path);
}

#[tokio::test]
async fn execute_tool_self_heals_unregistered_builtin_server() {
    let _auto_approve_mcp = crate::tool_security::AutoApproveMcpTestGuard::enable();
    let registry = McpClientRegistry::default();
    assert!(!registry
        .configs
        .lock()
        .await
        .contains_key("local_filesystem"));
    assert!(!registry
        .sessions
        .lock()
        .await
        .contains_key("local_filesystem"));

    let result = timeout(
        Duration::from_secs(30),
        registry.execute_tool(
            "local_filesystem",
            "list_directory",
            serde_json::json!({"path": ""}),
        ),
    )
    .await
    .expect("fallback tool call completes before timeout")
    .expect("unregistered built-in server self-heals and executes");

    assert!(!result.is_error);
    assert!(result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("files"))
        .is_some());
    assert!(registry
        .configs
        .lock()
        .await
        .contains_key("local_filesystem"));
    assert_eq!(
        registry
            .configs
            .lock()
            .await
            .get("local_filesystem")
            .map(|config| &config.transport),
        Some(&McpTransportConfig::Native)
    );
    assert!(registry
        .sessions
        .lock()
        .await
        .contains_key("local_filesystem"));
    assert!(registry
        .tool_catalog
        .lock()
        .await
        .get("local_filesystem")
        .is_some_and(
            |tools| tools.iter().any(|tool| tool.name == "list_directory")
                && tools.iter().any(|tool| tool.name == "delete_file")
        ));
}

#[test]
fn parses_tool_call_error_result_without_treating_it_as_transport_failure() {
    let result = parse_tool_call_result(serde_json::json!({
        "content": [{
            "type": "text",
            "text": "permission denied at https://user:pass@example.test/private?token=url-canary"
        }],
        "structuredContent": {
            "nested": {"api_key": "structured-secret-canary"},
            "oversized": "x".repeat(64 * 1024),
        },
        "_meta": {"password": "meta-secret-canary"},
        "isError": true
    }))
    .expect("tool result parses");

    assert!(result.is_error);
    assert_eq!(result.content.len(), 1);
    assert!(result.structured_content.is_none());
    assert!(result.meta.is_none());
    assert!(result.raw.is_none());
    let boundary_payload = serde_json::to_string(&result).unwrap();
    assert!(boundary_payload.len() <= 4_500);
    for canary in [
        "user:pass",
        "url-canary",
        "structured-secret-canary",
        "meta-secret-canary",
    ] {
        assert!(
            !boundary_payload.contains(canary),
            "error result leaked {canary}: {boundary_payload}"
        );
    }
    assert!(boundary_payload.contains("[redacted]"));
    assert!(boundary_payload.contains("...[truncated]"));
}

#[test]
fn tool_error_parser_preserves_only_allowlisted_timeout_classification() {
    let result = parse_tool_call_result(serde_json::json!({
        "content": [{"type": "text", "text": "private raw failure"}],
        "structuredContent": {
            "warning": "timeout",
            "message": "private raw failure",
            "token": "secret-canary"
        },
        "isError": true
    }))
    .unwrap();
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({"warning": "timeout"}))
    );
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(!serialized.contains("private raw failure"));
    assert!(!serialized.contains("secret-canary"));
}

#[test]
fn successful_tool_result_preserves_bounded_application_data() {
    let payload = serde_json::json!({
        "content": [{"type": "text", "text": "ordinary success"}],
        "structuredContent": {"value": 7},
        "_meta": {"source": "trusted after caller review"},
        "isError": false
    });
    let result = parse_tool_call_result(payload).unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content[0]["text"], "ordinary success");
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({"value": 7}))
    );
    assert_eq!(
        result.meta,
        Some(serde_json::json!({"source": "trusted after caller review"}))
    );
}

#[test]
fn successful_tool_result_requires_execution_evidence() {
    let error = parse_tool_call_result(serde_json::json!({
        "content": [],
        "isError": false
    }))
    .expect_err("empty success must not be accepted as completed execution");

    assert!(error
        .message
        .contains("without content or structured evidence"));
}

#[test]
fn json_rpc_error_does_not_echo_attacker_controlled_credentials() {
    let error = json_rpc_response_result(JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        result: None,
        error: Some(serde_json::json!({
            "code": -32000,
            "message": "failed",
            "data": {"api_key": "canary-json-rpc-secret"}
        })),
        id: serde_json::json!("error-id"),
    })
    .expect_err("JSON-RPC error remains an error");
    assert!(!error.message.contains("canary-json-rpc-secret"));
    assert!(error.message.contains("[redacted]"));
}

#[test]
fn stdio_log_entry_redacts_credentials_and_home_paths_before_persistence() {
    let entry = mcp_stderr_log_entry(
        "server Authorization=Bearer server-name-canary",
        "failed api_key=log-canary at /Users/Alex/Secret/file.txt with Bearer bearer-canary",
    );
    for canary in [
        "server-name-canary",
        "log-canary",
        "bearer-canary",
        "/Users/Alex/",
    ] {
        assert!(
            !entry.contains(canary),
            "log entry leaked {canary}: {entry}"
        );
    }
    assert!(entry.contains("[redacted]") || entry.contains("[REDACTED_SECURITY_TOKEN]"));
    assert!(entry.len() <= MCP_STDERR_LOG_FIELD_LIMIT + 512);
}

#[test]
fn rejects_deep_json_huge_catalogs_and_oversized_schemas() {
    let mut nested = serde_json::json!({"leaf": true});
    for _ in 0..=MCP_MAX_JSON_DEPTH {
        nested = serde_json::json!({"nested": nested});
    }
    let deep_payload = serde_json::json!({
        "jsonrpc": "2.0",
        "result": nested,
        "id": "deep"
    })
    .to_string();
    let deep_error =
        parse_json_rpc_message(&deep_payload).expect_err("deep remote JSON must be rejected");
    assert!(deep_error.message.contains("structure depth"));

    let tool = serde_json::json!({
        "name": "read_status",
        "description": "status",
        "inputSchema": {"type": "object"}
    });
    let huge_catalog = serde_json::json!({
        "tools": vec![tool; MCP_MAX_TOOL_CATALOG_SIZE + 1]
    });
    let catalog_error =
        parse_tools_list(huge_catalog).expect_err("huge MCP catalog must be rejected");
    assert!(catalog_error.message.contains("tool catalog"));

    let schema_error = parse_tools_list(serde_json::json!({
        "tools": [{
            "name": "oversized_schema",
            "inputSchema": {"description": "x".repeat(MCP_MAX_TOOL_SCHEMA_BYTES + 1)}
        }]
    }))
    .expect_err("oversized schema field must be rejected");
    assert!(
        schema_error.message.contains("field limit") || schema_error.message.contains("schema")
    );
}

#[tokio::test]
async fn disposable_remote_mcp_enforces_connect_and_per_call_one_use_bindings() {
    let revision = Arc::new(AtomicUsize::new(0));
    let (endpoint, port, server_handle) =
        spawn_disposable_mcp_http_server(revision.clone(), Arc::new(AtomicBool::new(false))).await;
    let config = McpServerConfig {
        name: "disposable_remote".to_string(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        transport: McpTransportConfig::Http {
            url: endpoint.clone(),
            local_origin_grant: Some(crate::network_policy::LocalOriginGrant {
                exact_loopback_port: port,
            }),
        },
    };
    let registry = McpClientRegistry::default();

    let unapproved = registry
        .connect_server(config.clone())
        .await
        .expect_err("remote connect cannot bypass native approval");
    assert_eq!(unapproved.code, "mcp_permission_required");

    let destination = crate::network_policy::resolve_destination(
        &endpoint,
        crate::network_policy::DestinationTransport::RemoteMcpHttp,
        config.transport.local_origin_grant(),
    )
    .await
    .unwrap();
    let authorization = McpSpawnAuthorization::shield_approved(&config, Some(destination.clone()));
    registry
        .connect_server_with_authorization(config.clone(), authorization)
        .await
        .expect("exact remote connection approval succeeds");

    let restart = registry
        .restart_server("disposable_remote")
        .await
        .expect_err("one-use connection approval cannot become restart authority");
    assert_eq!(restart.code, "mcp_permission_required");

    let arguments = serde_json::json!({
        "query": "status",
        "api_key": "canary-remote-secret"
    });
    let unkeyed_arguments_binding = {
        let mut hasher = Sha256::new();
        hasher.update(serde_json::to_vec(&arguments).unwrap());
        hex::encode(hasher.finalize())
    };
    let unshielded = registry
        .prepare_tool_approval_candidate("disposable_remote", "do_thing", arguments.clone())
        .await
        .unwrap()
        .unwrap();
    let native_action = remote_mcp_tool_shield_action(&unshielded).unwrap();
    let native_request = shield_gate::build_shield_approval_request(&native_action).unwrap();
    assert!(native_request
        .preview
        .contains(&unshielded.request.audit_id));
    assert!(native_request.preview.contains("http://127.0.0.1:"));
    assert!(native_request.preview.contains("argumentsBinding"));
    assert!(native_request.preview.contains("[redacted]"));
    assert!(!native_request.preview.contains("canary-remote-secret"));
    assert_ne!(unshielded.arguments_binding, unkeyed_arguments_binding);
    assert!(!native_request.preview.contains(&unkeyed_arguments_binding));
    let unshielded_token = unshielded.request.approval_token.clone();
    let activation_error = registry
        .activate_prepared_tool_approval(unshielded, false)
        .await
        .expect_err("remote authority cannot activate without a native Shield decision");
    assert_eq!(activation_error.code, "mcp_permission_required");
    assert!(registry.pending_tool_approvals.lock().await.is_empty());
    let bypass = registry
        .execute_tool_with_approval(
            "disposable_remote",
            "do_thing",
            arguments.clone(),
            Some(McpToolApproval {
                approval_token: unshielded_token,
            }),
        )
        .await
        .expect_err("a renderer-visible candidate token cannot bypass native Shield");
    assert_eq!(bypass.code, "mcp_permission_required");

    let approval = registry
        .prepare_remote_tool_approval_after_native_shield_for_test(
            "disposable_remote",
            "do_thing",
            arguments.clone(),
        )
        .await
        .expect("remote approval can be prepared")
        .expect("readOnlyHint cannot waive remote approval");
    assert!(approval.native_shield_approved);
    assert_eq!(
        approval.canonical_origin.as_deref(),
        Some(format!("http://127.0.0.1:{port}").as_str())
    );
    assert_eq!(approval.transport, "remote_mcp_http");
    assert_eq!(
        approval.resolved_destination_class.as_deref(),
        Some("exact_loopback")
    );
    assert_eq!(
        approval.destination_binding.as_deref(),
        Some(destination.binding_fingerprint())
    );
    assert!(approval
        .sensitive_fields
        .iter()
        .any(|field| field.contains("api_key")));
    assert!(!approval
        .arguments
        .to_string()
        .contains("canary-remote-secret"));
    assert!(!approval.argument_summary.contains("canary-remote-secret"));

    let token = approval.approval_token.clone();
    let result = registry
        .execute_tool_with_approval(
            "disposable_remote",
            "do_thing",
            arguments.clone(),
            Some(McpToolApproval {
                approval_token: token.clone(),
            }),
        )
        .await
        .expect("exact approved remote call succeeds");
    assert!(!result.is_error);

    let reused = registry
        .execute_tool_with_approval(
            "disposable_remote",
            "do_thing",
            arguments.clone(),
            Some(McpToolApproval {
                approval_token: token,
            }),
        )
        .await
        .expect_err("remote call approval is one-use");
    assert_eq!(reused.code, "mcp_permission_required");

    let schema_bound = registry
        .prepare_remote_tool_approval_after_native_shield_for_test(
            "disposable_remote",
            "do_thing",
            arguments.clone(),
        )
        .await
        .unwrap()
        .unwrap();
    revision.store(1, AtomicOrdering::Release);
    let changed_schema = registry
        .execute_tool_with_approval(
            "disposable_remote",
            "do_thing",
            arguments,
            Some(McpToolApproval {
                approval_token: schema_bound.approval_token,
            }),
        )
        .await
        .expect_err("tool schema change revokes pending authority");
    assert_eq!(changed_schema.code, "mcp_permission_required");

    server_handle.abort();
}

#[tokio::test]
async fn public_remote_cancellation_command_interrupts_an_in_flight_tool_call() {
    let revision = Arc::new(AtomicUsize::new(0));
    let stall_tool_calls = Arc::new(AtomicBool::new(true));
    let (endpoint, port, server_handle) =
        spawn_disposable_mcp_http_server(revision, stall_tool_calls).await;
    let config = McpServerConfig {
        name: "cancellable_remote".to_string(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        transport: McpTransportConfig::Http {
            url: endpoint.clone(),
            local_origin_grant: Some(crate::network_policy::LocalOriginGrant {
                exact_loopback_port: port,
            }),
        },
    };
    let destination = crate::network_policy::resolve_destination(
        &endpoint,
        crate::network_policy::DestinationTransport::RemoteMcpHttp,
        config.transport.local_origin_grant(),
    )
    .await
    .unwrap();
    let registry = McpClientRegistry::default();
    registry
        .connect_server_with_authorization(
            config.clone(),
            McpSpawnAuthorization::shield_approved(&config, Some(destination)),
        )
        .await
        .unwrap();

    let arguments = serde_json::json!({"query": "wait"});
    let approval = registry
        .prepare_remote_tool_approval_after_native_shield_for_test(
            "cancellable_remote",
            "do_thing",
            arguments.clone(),
        )
        .await
        .unwrap()
        .unwrap();
    let call_registry = registry.clone();
    let call = tokio::spawn(async move {
        call_registry
            .execute_tool_with_approval(
                "cancellable_remote",
                "do_thing",
                arguments,
                Some(McpToolApproval {
                    approval_token: approval.approval_token,
                }),
            )
            .await
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        registry
            .cancel_remote_operations(Some("cancellable_remote"))
            .await,
        1
    );
    let error = tokio::time::timeout(Duration::from_secs(1), call)
        .await
        .expect("cancellation must bound the in-flight call")
        .unwrap()
        .expect_err("the in-flight tool call must be cancelled");
    assert_eq!(error.code, "mcp_cancelled");
    assert!(registry.session("cancellable_remote").await.is_err());
    assert!(registry.pending_tool_approvals.lock().await.is_empty());
    server_handle.abort();
}

#[tokio::test]
async fn public_remote_cancellation_interrupts_stalled_initialization_before_registration() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (initialize_started, initialize_observed) = tokio::sync::oneshot::channel();
    let server_handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let body = read_async_http_request_body(&mut socket).await;
        let payload = serde_json::from_str::<Value>(&body).unwrap();
        assert_eq!(
            payload.get("method").and_then(Value::as_str),
            Some("initialize")
        );
        socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        let _ = initialize_started.send(());
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    let endpoint = format!("http://127.0.0.1:{port}/mcp");
    let config = McpServerConfig {
        name: "stalled_initialize".to_string(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        transport: McpTransportConfig::Http {
            url: endpoint.clone(),
            local_origin_grant: Some(crate::network_policy::LocalOriginGrant {
                exact_loopback_port: port,
            }),
        },
    };
    let destination = crate::network_policy::resolve_destination(
        &endpoint,
        crate::network_policy::DestinationTransport::RemoteMcpHttp,
        config.transport.local_origin_grant(),
    )
    .await
    .unwrap();
    let registry = McpClientRegistry::default();
    let connect_registry = registry.clone();
    let connect_config = config.clone();
    let connect = tokio::spawn(async move {
        connect_registry
            .connect_server_with_authorization(
                connect_config.clone(),
                McpSpawnAuthorization::shield_approved(&connect_config, Some(destination)),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), initialize_observed)
        .await
        .expect("the real server must observe initialize before cancellation")
        .unwrap();

    assert_eq!(
        registry
            .cancel_remote_operations(Some("stalled_initialize"))
            .await,
        1
    );
    let error = tokio::time::timeout(Duration::from_secs(1), connect)
        .await
        .expect("native cancellation must bound stalled initialization")
        .unwrap()
        .expect_err("stalled initialization must not register after cancellation");
    assert_eq!(error.code, "mcp_cancelled");
    assert!(registry.sessions.lock().await.is_empty());
    assert!(registry.connecting_remote.lock().await.is_empty());
    assert!(registry.tool_catalog.lock().await.is_empty());
    server_handle.abort();
}

#[tokio::test]
async fn public_cancel_reaches_the_remote_build_phase_before_dns_revalidation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicBool::new(false));
    let accepted_by_server = accepted.clone();
    let server_handle = tokio::spawn(async move {
        if listener.accept().await.is_ok() {
            accepted_by_server.store(true, AtomicOrdering::Release);
        }
    });
    let endpoint = format!("http://127.0.0.1:{port}/mcp");
    let config = McpServerConfig {
        name: "cancel_before_build".to_string(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        transport: McpTransportConfig::Http {
            url: endpoint.clone(),
            local_origin_grant: Some(crate::network_policy::LocalOriginGrant {
                exact_loopback_port: port,
            }),
        },
    };
    let destination = crate::network_policy::resolve_destination(
        &endpoint,
        crate::network_policy::DestinationTransport::RemoteMcpHttp,
        config.transport.local_origin_grant(),
    )
    .await
    .unwrap();
    let registry = McpClientRegistry::default();
    let hook = RemoteConnectTestHook::new(RemoteConnectTestPhase::BeforeBuild);
    *registry.remote_connect_test_hook.lock().await = Some(hook.clone());
    let connect_registry = registry.clone();
    let connect_config = config.clone();
    let connect = tokio::spawn(async move {
        connect_registry
            .connect_server_with_authorization(
                connect_config.clone(),
                McpSpawnAuthorization::shield_approved(&connect_config, Some(destination)),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), hook.entered.notified())
        .await
        .expect("connect must register cancellation authority before remote client build");
    assert!(registry
        .connecting_remote
        .lock()
        .await
        .contains_key("cancel_before_build"));
    assert_eq!(
        registry
            .cancel_remote_operations(Some("cancel_before_build"))
            .await,
        1
    );
    hook.release.notify_one();
    let error = tokio::time::timeout(Duration::from_secs(1), connect)
        .await
        .expect("pre-build cancellation must be bounded")
        .unwrap()
        .expect_err("cancelled build cannot proceed to initialization");
    assert_eq!(error.code, "mcp_cancelled");
    tokio::task::yield_now().await;
    assert!(!accepted.load(AtomicOrdering::Acquire));
    assert!(registry.sessions.lock().await.is_empty());
    assert!(registry.connecting_remote.lock().await.is_empty());
    server_handle.abort();
}

#[tokio::test]
async fn activation_and_cancel_are_linearized_without_stale_catalog_repopulation() {
    let revision = Arc::new(AtomicUsize::new(0));
    let (endpoint, port, server_handle) =
        spawn_disposable_mcp_http_server(revision, Arc::new(AtomicBool::new(false))).await;
    let config = McpServerConfig {
        name: "activation_race".to_string(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        transport: McpTransportConfig::Http {
            url: endpoint.clone(),
            local_origin_grant: Some(crate::network_policy::LocalOriginGrant {
                exact_loopback_port: port,
            }),
        },
    };
    let destination = crate::network_policy::resolve_destination(
        &endpoint,
        crate::network_policy::DestinationTransport::RemoteMcpHttp,
        config.transport.local_origin_grant(),
    )
    .await
    .unwrap();
    let registry = McpClientRegistry::default();
    let hook = RemoteConnectTestHook::new(RemoteConnectTestPhase::DuringActivation);
    *registry.remote_connect_test_hook.lock().await = Some(hook.clone());
    let connect_registry = registry.clone();
    let connect_config = config.clone();
    let connect = tokio::spawn(async move {
        connect_registry
            .connect_server_with_authorization(
                connect_config.clone(),
                McpSpawnAuthorization::shield_approved(&connect_config, Some(destination)),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), hook.entered.notified())
        .await
        .expect("connect must pause after the complete session and catalog activation");
    assert!(registry
        .sessions
        .lock()
        .await
        .contains_key("activation_race"));
    assert!(registry
        .tool_catalog
        .lock()
        .await
        .contains_key("activation_race"));

    let cancel_registry = registry.clone();
    let cancel = tokio::spawn(async move {
        cancel_registry
            .cancel_remote_operations(Some("activation_race"))
            .await
    });
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        !cancel.is_finished(),
        "cancel must wait for the connection activation linearization point"
    );
    hook.release.notify_one();
    assert!(matches!(
        connect.await.unwrap().unwrap().status,
        McpServerStatus::Connected
    ));
    assert_eq!(cancel.await.unwrap(), 1);
    assert!(registry.sessions.lock().await.is_empty());
    assert!(registry.tool_catalog.lock().await.is_empty());
    assert!(registry.connecting_remote.lock().await.is_empty());
    server_handle.abort();
}
