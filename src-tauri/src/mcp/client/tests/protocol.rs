use super::*;

#[test]
fn parses_request_response_and_notification_shapes() {
    let request =
        parse_json_rpc_message(r#"{"jsonrpc":"2.0","method":"tools/list","params":{},"id":1}"#)
            .expect("request parses");
    assert!(matches!(request, JsonRpcMessage::Request(_)));

    let response = parse_json_rpc_message(r#"{"jsonrpc":"2.0","result":{"ok":true},"id":1}"#)
        .expect("response parses");
    assert!(matches!(response, JsonRpcMessage::Response(_)));

    let notification =
        parse_json_rpc_message(r#"{"jsonrpc":"2.0","method":"notifications/ready"}"#)
            .expect("notification parses");
    assert!(matches!(notification, JsonRpcMessage::Notification(_)));
}

#[tokio::test]
async fn dispatches_json_rpc_request_through_stdio_echo_server() {
    let python = python3().expect("python3 is required for the MCP stdio echo test");
    let script_path = std::env::temp_dir().join(format!(
        "oomu-mcp-echo-{}-{}.py",
        std::process::id(),
        unix_time_ms()
    ));
    fs::write(
        &script_path,
        r#"
import json
import sys

for line in sys.stdin:
    payload = json.loads(line)
    response = {
        "jsonrpc": "2.0",
        "result": {
            "method": payload.get("method"),
            "params": payload.get("params"),
        },
        "id": payload.get("id"),
    }
    print(json.dumps(response), flush=True)
"#,
    )
    .expect("echo script writes");

    let config = McpServerConfig {
        name: "mock_echo".to_string(),
        command: python,
        args: vec![script_path.to_string_lossy().to_string()],
        env: HashMap::new(),
        transport: McpTransportConfig::Stdio,
    };
    let session = McpClientSession::spawn(config)
        .await
        .expect("mock MCP server spawns");
    let response = timeout(
        Duration::from_secs(5),
        session.send_request(JsonRpcRequest::new(
            "tools/call",
            serde_json::json!({"name": "echo"}),
            serde_json::json!("req-1"),
        )),
    )
    .await
    .expect("response arrives before timeout")
    .expect("response is successful");

    assert_eq!(response.jsonrpc, "2.0");
    assert_eq!(response.id, serde_json::json!("req-1"));
    assert_eq!(
        response.result,
        Some(serde_json::json!({
            "method": "tools/call",
            "params": {"name": "echo"}
        }))
    );

    session.shutdown().await.expect("mock server shuts down");
    let _ = fs::remove_file(script_path);
}

#[tokio::test]
async fn trusted_stdio_session_retains_immutable_activation_binding_after_spawn_grant_is_consumed()
{
    let python = python3().expect("python3 is required for the MCP activation-binding test");
    let script_path = std::env::temp_dir().join(format!(
        "oomu-mcp-activation-binding-{}-{}.py",
        std::process::id(),
        unix_time_ms()
    ));
    fs::write(
        &script_path,
        r#"
import json
import sys

for line in sys.stdin:
    payload = json.loads(line)
    request_id = payload.get("id")
    if request_id is None:
        continue
    method = payload.get("method")
    if method == "initialize":
        result = {
            "protocolVersion": payload.get("params", {}).get("protocolVersion"),
            "capabilities": {},
            "serverInfo": {"name": "activation-binding-test", "version": "1"},
        }
    elif method == "tools/list":
        result = {"tools": [{
            "name": "draft_system_email",
            "description": "Create an unsent draft",
            "inputSchema": {"type": "object"},
        }]}
    elif method == "tools/call":
        result = {
            "content": [{"type": "text", "text": "draft created"}],
            "structuredContent": {"draftId": "test-draft"},
            "isError": False,
        }
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "result": result, "id": request_id}), flush=True)
"#,
    )
    .expect("activation-binding script writes");

    let config = McpServerConfig {
        name: MACOS_APPLESCRIPT_SERVER_NAME.to_string(),
        command: python,
        args: vec![script_path.to_string_lossy().to_string()],
        env: HashMap::new(),
        transport: McpTransportConfig::Stdio,
    };
    let registry = McpClientRegistry::default();
    registry
        .register_trusted_server_configs(vec![config.clone()])
        .await;
    registry
        .connect_server_with_authorization(
            config.clone(),
            McpSpawnAuthorization::trusted_internal(&config),
        )
        .await
        .expect("trusted built-in server connects");

    assert!(matches!(
        registry.spawn_authorization_for(&config.name).await,
        McpSpawnAuthorization::Unapproved
    ));
    let session = registry
        .session(&config.name)
        .await
        .expect("active trusted session remains registered");
    assert!(session.has_trusted_internal_activation_for(&config));
    assert!(
        registry
            .has_active_trusted_builtin_session(&config.name)
            .await
    );
    let result = registry
        .execute_trusted_mail_draft_after_native_shield(serde_json::json!({
            "to_recipients": ["test@example.invalid"],
            "subject": "Test draft",
            "body": "Test body"
        }))
        .await
        .expect("the active trusted session reaches the exact Mail tool");
    assert!(!result.is_error);
    assert_eq!(
        result.structured_content,
        Some(serde_json::json!({"draftId": "test-draft"}))
    );

    let mut changed = config.clone();
    changed.args.push("--unexpected".to_string());
    assert!(!session.has_trusted_internal_activation_for(&changed));

    drop(session);
    drop(registry);
    let _ = fs::remove_file(script_path);
}

#[tokio::test]
async fn one_use_shield_stdio_activation_never_becomes_trusted_internal_authority() {
    let python = python3().expect("python3 is required for the MCP activation-binding test");
    let script_path = std::env::temp_dir().join(format!(
        "oomu-mcp-shield-binding-{}-{}.py",
        std::process::id(),
        unix_time_ms()
    ));
    fs::write(
        &script_path,
        r#"
import json
import sys

for line in sys.stdin:
    payload = json.loads(line)
    request_id = payload.get("id")
    if request_id is None:
        continue
    method = payload.get("method")
    if method == "initialize":
        result = {
            "protocolVersion": payload.get("params", {}).get("protocolVersion"),
            "capabilities": {},
            "serverInfo": {"name": "shield-binding-test", "version": "1"},
        }
    elif method == "tools/list":
        result = {"tools": [{
            "name": "draft_system_email",
            "description": "Create an unsent draft",
            "inputSchema": {"type": "object"},
        }]}
    elif method == "tools/call":
        result = {
            "content": [{"type": "text", "text": "must not execute"}],
            "structuredContent": {"draftId": "unexpected"},
            "isError": False,
        }
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "result": result, "id": request_id}), flush=True)
"#,
    )
    .expect("shield-binding script writes");

    let config = McpServerConfig {
        name: MACOS_APPLESCRIPT_SERVER_NAME.to_string(),
        command: python,
        args: vec![script_path.to_string_lossy().to_string()],
        env: HashMap::new(),
        transport: McpTransportConfig::Stdio,
    };
    let registry = McpClientRegistry::default();
    registry
        .register_trusted_server_configs(vec![config.clone()])
        .await;
    registry
        .connect_server_with_authorization(
            config.clone(),
            McpSpawnAuthorization::shield_approved(&config, None),
        )
        .await
        .expect("one-use Shield-approved server connects");
    let session = registry
        .session(&config.name)
        .await
        .expect("active Shield-approved session remains registered");
    assert!(!session.has_trusted_internal_activation_for(&config));
    assert!(
        !registry
            .has_active_trusted_builtin_session(&config.name)
            .await
    );
    let error = registry
        .execute_trusted_mail_draft_after_native_shield(serde_json::json!({
            "to_recipients": ["test@example.invalid"],
            "subject": "Test draft",
            "body": "Test body"
        }))
        .await
        .expect_err("a one-use Shield session cannot become trusted Mail authority");
    assert_eq!(error.code, "mcp_permission_required");
    assert!(error.message.contains("trusted built-in binding"));

    drop(session);
    drop(registry);
    let _ = fs::remove_file(script_path);
}

#[tokio::test]
async fn remote_http_request_sanitizes_payload_before_network() {
    let server = spawn_one_shot_http_server(
        "application/json",
        r#"{"jsonrpc":"2.0","result":{"ok":true},"id":"req-remote"}"#.to_string(),
    );
    let session = McpClientSession::spawn(McpServerConfig {
        name: "remote_http_mock".to_string(),
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

    let response = timeout(
        Duration::from_secs(5),
        session.send_request(JsonRpcRequest::new(
            "tools/call",
            serde_json::json!({
                "name": "echo",
                "arguments": {
                    "path": "/Users/example/OOMU/sandbox/test.txt",
                    "api_key": "sk-proj-1234567890abcdef",
                    "email": "operator@example.com"
                }
            }),
            serde_json::json!("req-remote"),
        )),
    )
    .await
    .expect("HTTP mock returns before timeout")
    .expect("remote HTTP request succeeds");

    assert_eq!(response.result, Some(serde_json::json!({"ok": true})));
    let received = server
        .received
        .recv_timeout(Duration::from_secs(5))
        .expect("mock server captures request body");
    server.handle.join().expect("mock server thread joins");

    assert!(received.contains("~/OOMU/sandbox/test.txt"));
    assert!(received.contains("[REDACTED_SECURITY_TOKEN]"));
    assert!(received.contains("[REDACTED_PII]"));
    assert!(!received.contains("/Users/example/"));
    assert!(!received.contains("sk-proj-1234567890abcdef"));
    assert!(!received.contains("operator@example.com"));
}

#[tokio::test]
async fn malformed_remote_response_terminally_revokes_the_transport() {
    let server = spawn_one_shot_http_server("application/json", "{malformed".to_string());
    let session = McpClientSession::spawn(McpServerConfig {
        name: "remote_malformed_terminal".to_string(),
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

    let malformed = session
        .send_request(JsonRpcRequest::new(
            "tools/list",
            serde_json::json!({}),
            serde_json::json!("malformed-1"),
        ))
        .await
        .expect_err("malformed remote JSON must fail");
    assert_eq!(malformed.code, "mcp_protocol_error");
    assert!(session.remote_cancellation.load(Ordering::Acquire));

    let reused = session
        .send_request(JsonRpcRequest::new(
            "tools/list",
            serde_json::json!({}),
            serde_json::json!("malformed-2"),
        ))
        .await
        .expect_err("a boundary-failed remote transport cannot be reused");
    assert_eq!(reused.code, "mcp_cancelled");

    server
        .received
        .recv_timeout(Duration::from_secs(5))
        .expect("mock server captures the one allowed request");
    server.handle.join().expect("mock server thread joins");
}

#[tokio::test]
async fn remote_sse_request_reads_event_stream_response() {
    let server = spawn_one_shot_http_server(
        "text/event-stream",
        concat!(
            "event: message\n",
            "data: {\"jsonrpc\":\"2.0\",\"result\":{\"streamed\":true},\"id\":\"req-sse\"}\n\n"
        )
        .to_string(),
    );
    let session = McpClientSession::spawn(McpServerConfig {
        name: "remote_sse_mock".to_string(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        transport: McpTransportConfig::Sse {
            url: server.url.clone(),
            local_origin_grant: Some(crate::network_policy::LocalOriginGrant {
                exact_loopback_port: reqwest::Url::parse(&server.url).unwrap().port().unwrap(),
            }),
        },
    })
    .await
    .expect("remote SSE session spawns");

    let response = timeout(
        Duration::from_secs(5),
        session.send_request(JsonRpcRequest::new(
            "tools/list",
            serde_json::json!({"token": "ghp_1234567890abcdef"}),
            serde_json::json!("req-sse"),
        )),
    )
    .await
    .expect("SSE mock returns before timeout")
    .expect("remote SSE request succeeds");

    assert_eq!(response.result, Some(serde_json::json!({"streamed": true})));
    let received = server
        .received
        .recv_timeout(Duration::from_secs(5))
        .expect("mock server captures SSE request body");
    server.handle.join().expect("mock server thread joins");

    assert!(received.contains("[REDACTED_SECURITY_TOKEN]"));
    assert!(!received.contains("ghp_1234567890abcdef"));
}
