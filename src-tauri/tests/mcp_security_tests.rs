use oomu_lib::mcp::{
    client::{McpClientRegistry, McpServerConfig},
    shield::McpTransportConfig,
};
use std::collections::HashMap;

#[tokio::test]
async fn test_mcp_spawn_requires_shield_gate_or_fails() {
    let registry = McpClientRegistry::default();
    let executable = std::env::current_exe().expect("current test executable resolves");

    let error = registry
        .connect_server(McpServerConfig {
            name: "arbitrary_stdio".to_string(),
            command: executable.display().to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            transport: McpTransportConfig::Stdio,
        })
        .await
        .expect_err("custom stdio MCP spawns require Shield Gate authorization");

    assert_eq!(error.code, "mcp_permission_required");
    assert!(
        error.message.contains("Shield Gate approval"),
        "unexpected error message: {}",
        error.message
    );
}

#[tokio::test]
async fn registered_stdio_config_cannot_self_heal_without_shield_gate() {
    let registry = McpClientRegistry::default();
    let executable = std::env::current_exe().expect("current test executable resolves");

    let registered = registry
        .register_server_configs(vec![McpServerConfig {
            name: "registered_stdio".to_string(),
            command: executable.display().to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            transport: McpTransportConfig::Stdio,
        }])
        .await;
    assert_eq!(registered, 1);

    let error = registry
        .execute_tool(
            "registered_stdio",
            "read_status",
            serde_json::json!({ "path": "" }),
        )
        .await
        .expect_err("registered custom stdio config cannot self-heal without approval");

    assert_eq!(error.code, "mcp_permission_required");
    assert!(
        error.message.contains("Shield Gate approval"),
        "unexpected error message: {}",
        error.message
    );
}
