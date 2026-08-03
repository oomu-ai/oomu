use serde::{Deserialize, Serialize};

pub const CAPABILITY_KINDS: [&str; 8] = [
    "file",
    "network",
    "connector",
    "model",
    "executable",
    "schedule",
    "child_agent",
    "mutation",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectBundleRequest {
    pub mod_id: String,
    #[serde(default)]
    pub project_ids: Vec<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivateBundleRequest {
    pub bundle_id: String,
    pub package_version: String,
    #[serde(default)]
    pub acknowledge_unreviewed: bool,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleVersionRequest {
    pub bundle_id: String,
    pub package_version: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleAuthorityRequest {
    pub bundle_id: String,
    pub project_id: String,
    pub capability: String,
    pub requested_scope: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityGrant {
    pub capability: String,
    pub bounded_scope: String,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityBundleRecord {
    pub bundle_id: String,
    pub package_version: String,
    pub mod_id: String,
    pub name: String,
    pub publisher_name: String,
    pub publisher_identity_verified: bool,
    pub review_state: String,
    pub integrity_state: String,
    pub compatibility_state: String,
    pub capabilities: Vec<CapabilityGrant>,
    pub project_ids: Vec<String>,
    pub install_state: String,
    pub previous_version: Option<String>,
    pub updated_at_ms: i64,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryCatalogRequest {
    pub catalog: RegistryCatalog,
    pub public_key: String,
    pub signature: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryCatalog {
    pub revision: String,
    pub entries: Vec<RegistryEntryInput>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryEntryInput {
    pub bundle_id: String,
    pub package_version: String,
    pub name: String,
    pub summary: String,
    pub category: String,
    pub publisher_name: String,
    pub review_state: String,
    pub compatibility_state: String,
    pub changelog: String,
    pub payload_sha256: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEntry {
    pub bundle_id: String,
    pub package_version: String,
    pub name: String,
    pub summary: String,
    pub category: String,
    pub publisher_name: String,
    pub review_state: String,
    pub compatibility_state: String,
    pub changelog: String,
    pub installed: bool,
}
