use crate::{
    foundation::digest::sha256_hex,
    sovereign_identity::{SignatureBlock, SovereignIdentity},
};
use serde::Serialize;
use serde_json::{json, Value};

const WORKFLOW_ARTIFACT_PROVENANCE_VERSION: &str = "oomu-workflow-artifact-v1";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkflowArtifactProvenance {
    schema_version: &'static str,
    artifact_type: String,
    artifact_id: String,
    content_sha256: String,
    signature: SignatureBlock,
}

pub(crate) fn build_workflow_artifact_provenance(
    artifact_type: &str,
    artifact_id: &str,
    value: &Value,
    identity: &SovereignIdentity,
) -> Result<WorkflowArtifactProvenance, String> {
    if artifact_type.trim().is_empty() || artifact_id.trim().is_empty() {
        return Err("Workflow artifact provenance requires type and id.".to_string());
    }
    identity
        .generate_node_identity()
        .map_err(|error| error.message.clone())?;
    let encoded = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    let content_sha256 = sha256_hex(&encoded);
    let payload = serde_json::to_string(&json!({
        "artifactId": artifact_id,
        "artifactType": artifact_type,
        "contentSha256": content_sha256,
        "schemaVersion": WORKFLOW_ARTIFACT_PROVENANCE_VERSION,
    }))
    .map_err(|error| error.to_string())?;
    let signature = identity
        .sign_node_payload(&payload)
        .map_err(|error| error.message)?;
    Ok(WorkflowArtifactProvenance {
        schema_version: WORKFLOW_ARTIFACT_PROVENANCE_VERSION,
        artifact_type: artifact_type.to_string(),
        artifact_id: artifact_id.to_string(),
        content_sha256,
        signature,
    })
}
