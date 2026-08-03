use serde::{Deserialize, Serialize};

/// Typed MCP result envelope shared by native data providers and the MCP
/// transport without making those providers depend on the transport itself.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpToolCallResult {
    #[serde(default)]
    pub content: Vec<serde_json::Value>,
    #[serde(default, rename = "structuredContent")]
    pub structured_content: Option<serde_json::Value>,
    #[serde(default, rename = "isError")]
    pub is_error: bool,
    #[serde(default, rename = "_meta")]
    pub meta: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}
