use serde::{Deserialize, Serialize};

use super::manifest::{
    RecommendedModelManifest, ATTRIBUTION, CANONICAL_MODEL_ID, DISPLAYED_LICENSE,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInspectionEvidence {
    pub accepted: bool,
    pub architecture: String,
    pub tensor_count: usize,
    pub model_bytes: u64,
    pub multimodal_projector_count: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedProviderEvidence {
    pub provider_id: String,
    pub provider_type: String,
    pub model_id: String,
    pub verified: bool,
    pub activation_receipt_id: Option<String>,
}

impl CompletedProviderEvidence {
    pub fn verified_local(
        provider_id: impl Into<String>,
        activation_receipt_id: Option<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            provider_type: "local_model".to_string(),
            model_id: CANONICAL_MODEL_ID.to_string(),
            verified: true,
            activation_receipt_id,
        }
    }

    pub fn validate(&self) -> bool {
        self.verified
            && self.provider_type == "local_model"
            && self.model_id == CANONICAL_MODEL_ID
            && !self.provider_id.trim().is_empty()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledAssetReceipt {
    pub role: super::manifest::AssetRole,
    pub filename: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedModelInstallReceipt {
    pub receipt_id: String,
    pub manifest_schema_version: u32,
    pub manifest_revision: String,
    pub canonical_model_id: String,
    pub repository: String,
    pub attribution: String,
    pub displayed_license: String,
    pub assets: Vec<InstalledAssetReceipt>,
    pub runtime_inspection: RuntimeInspectionEvidence,
    pub provider: CompletedProviderEvidence,
    #[serde(default)]
    pub package_identity_sha256: Option<String>,
    pub final_state: String,
    pub started_at_ms: u128,
    pub completed_at_ms: u128,
}

impl RecommendedModelInstallReceipt {
    pub(crate) fn completed(
        receipt_id: String,
        manifest: &RecommendedModelManifest,
        inspection: RuntimeInspectionEvidence,
        provider: CompletedProviderEvidence,
        package_identity_sha256: String,
        started_at_ms: u128,
        completed_at_ms: u128,
    ) -> Self {
        Self {
            receipt_id,
            manifest_schema_version: manifest.schema_version,
            manifest_revision: manifest.revision.clone(),
            canonical_model_id: manifest.model_id.clone(),
            repository: manifest.repository.clone(),
            attribution: ATTRIBUTION.to_string(),
            displayed_license: DISPLAYED_LICENSE.to_string(),
            assets: manifest
                .assets
                .iter()
                .map(|asset| InstalledAssetReceipt {
                    role: asset.role,
                    filename: asset.filename.clone(),
                    bytes: asset.bytes,
                    sha256: asset.sha256.clone(),
                })
                .collect(),
            runtime_inspection: inspection,
            provider,
            package_identity_sha256: Some(package_identity_sha256),
            final_state: "ready".to_string(),
            started_at_ms,
            completed_at_ms,
        }
    }
}
