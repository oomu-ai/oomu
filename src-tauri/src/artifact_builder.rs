use crate::sovereign_identity::SignatureBlock;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SignedArkArtifact {
    pub artifact_id: String,
    pub objective: String,
    pub distilled_findings: Vec<String>,
    pub logical_certificate: serde_json::Value,
    pub parent_artifact_hashes: Vec<String>,
    pub artifact_hash: String,
    pub signature: SignatureBlock,
    pub created_at_ms: i64,
}

pub fn read_signed_ark_artifact(path: &Path) -> Result<SignedArkArtifact, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}
