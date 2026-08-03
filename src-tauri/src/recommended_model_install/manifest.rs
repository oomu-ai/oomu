use serde::{Deserialize, Serialize};

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const CANONICAL_MODEL_ID: &str = "gemma-4-E2B-it-qat-q4_0-gguf";
pub const DISPLAY_NAME: &str = "Gemma 4 E2B IT QAT Q4_0 GGUF";
pub const REPOSITORY: &str = "google/gemma-4-E2B-it-qat-q4_0-gguf";
pub const REPOSITORY_URL: &str = "https://huggingface.co/google/gemma-4-E2B-it-qat-q4_0-gguf";
pub const IMMUTABLE_REVISION: &str = "675cff42a74c774d6cb76f76d8eacb49b48c9b93";
pub const DISPLAYED_LICENSE: &str = "Apache License 2.0";
pub const ATTRIBUTION: &str = "Google";
pub const PACKAGE_TOTAL_BYTES: u64 = 4_336_349_920;

const PRIMARY_URL: &str = "https://huggingface.co/google/gemma-4-E2B-it-qat-q4_0-gguf/resolve/675cff42a74c774d6cb76f76d8eacb49b48c9b93/gemma-4-E2B_q4_0-it.gguf";
const PROJECTOR_URL: &str = "https://huggingface.co/google/gemma-4-E2B-it-qat-q4_0-gguf/resolve/675cff42a74c774d6cb76f76d8eacb49b48c9b93/gemma-4-E2B-it-mmproj.gguf";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetRole {
    PrimaryModel,
    MultimodalProjector,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedModelAsset {
    pub role: AssetRole,
    pub filename: String,
    pub bytes: u64,
    pub sha256: String,
    #[serde(skip_serializing)]
    pub(crate) url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedModelManifest {
    pub schema_version: u32,
    pub model_id: String,
    pub display_name: String,
    pub explanatory_line: String,
    pub repository: String,
    pub repository_url: String,
    pub revision: String,
    pub total_bytes: u64,
    pub displayed_license: String,
    pub attribution: String,
    pub assets: Vec<RecommendedModelAsset>,
}

impl RecommendedModelManifest {
    pub fn primary_asset(&self) -> &RecommendedModelAsset {
        self.assets
            .iter()
            .find(|asset| asset.role == AssetRole::PrimaryModel)
            .expect("the release-controlled manifest must contain a primary model")
    }

    pub fn projector_asset(&self) -> &RecommendedModelAsset {
        self.assets
            .iter()
            .find(|asset| asset.role == AssetRole::MultimodalProjector)
            .expect("the release-controlled manifest must contain a projector")
    }
}

pub fn recommended_model_manifest() -> RecommendedModelManifest {
    RecommendedModelManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        model_id: CANONICAL_MODEL_ID.to_string(),
        display_name: DISPLAY_NAME.to_string(),
        explanatory_line: "The on-device model OOMU is optimized for.".to_string(),
        repository: REPOSITORY.to_string(),
        repository_url: REPOSITORY_URL.to_string(),
        revision: IMMUTABLE_REVISION.to_string(),
        total_bytes: PACKAGE_TOTAL_BYTES,
        displayed_license: DISPLAYED_LICENSE.to_string(),
        attribution: ATTRIBUTION.to_string(),
        assets: vec![
            RecommendedModelAsset {
                role: AssetRole::PrimaryModel,
                filename: "gemma-4-E2B_q4_0-it.gguf".to_string(),
                bytes: 3_349_516_256,
                sha256: "fa401b55b07ee70a54c6dae3903c783a6e65064312529ea57175cb5f8dec6634"
                    .to_string(),
                url: PRIMARY_URL.to_string(),
            },
            RecommendedModelAsset {
                role: AssetRole::MultimodalProjector,
                filename: "gemma-4-E2B-it-mmproj.gguf".to_string(),
                bytes: 986_833_664,
                sha256: "021059cce659fe7f9170d5599761d7bbaf644b798dab9503aca30dc43e6beb14"
                    .to_string(),
                url: PROJECTOR_URL.to_string(),
            },
        ],
    }
}

#[cfg(test)]
pub(crate) fn fixture_manifest(
    base_url: &str,
    primary: &[u8],
    projector: &[u8],
) -> RecommendedModelManifest {
    use crate::foundation::digest::sha256_hex;

    let assets = [
        (AssetRole::PrimaryModel, "model.gguf", primary),
        (
            AssetRole::MultimodalProjector,
            "model-mmproj.gguf",
            projector,
        ),
    ]
    .into_iter()
    .map(|(role, filename, bytes)| RecommendedModelAsset {
        role,
        filename: filename.to_string(),
        bytes: bytes.len() as u64,
        sha256: sha256_hex(bytes),
        url: format!("{base_url}/{filename}"),
    })
    .collect::<Vec<_>>();
    RecommendedModelManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        model_id: CANONICAL_MODEL_ID.to_string(),
        display_name: DISPLAY_NAME.to_string(),
        explanatory_line: "fixture".to_string(),
        repository: "local-test-fixture".to_string(),
        repository_url: base_url.to_string(),
        revision: "fixture-revision".to_string(),
        total_bytes: (primary.len() + projector.len()) as u64,
        displayed_license: DISPLAYED_LICENSE.to_string(),
        attribution: ATTRIBUTION.to_string(),
        assets,
    }
}
