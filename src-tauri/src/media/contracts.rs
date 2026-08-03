use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IngestMediaRequest {
    pub project_id: String,
    pub task_id: Option<String>,
    pub task_run_id: Option<String>,
    pub source_kind: String,
    pub source_reference: String,
    pub mime_type: String,
    pub data_base64: String,
    pub retention_mode: String,
    pub expires_at_ms: Option<i64>,
    #[serde(default)]
    pub redaction_categories: Vec<String>,
    pub routing_mode: String,
    #[serde(default)]
    pub provider_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaProjectRequest {
    pub project_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediaAssetRequest {
    pub project_id: String,
    pub media_asset_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveTranscriptRequest {
    pub project_id: String,
    pub media_asset_id: String,
    pub transcript: String,
    pub language: String,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub timestamps: Vec<TranscriptTimestamp>,
    pub route_kind: String,
    pub route_label: String,
    #[serde(default)]
    pub edited_by_user: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptTimestamp {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptRecord {
    pub revision: u64,
    pub transcript: String,
    pub language: String,
    pub confidence: Option<f64>,
    pub timestamps: Vec<TranscriptTimestamp>,
    pub route_kind: String,
    pub route_label: String,
    pub edited_by_user: bool,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaAssetRecord {
    pub media_asset_id: String,
    pub project_id: String,
    pub task_id: Option<String>,
    pub task_run_id: Option<String>,
    pub media_kind: String,
    pub mime_type: String,
    pub sha256: String,
    pub byte_length: u64,
    pub source_kind: String,
    pub source_reference: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub retention_mode: String,
    pub expires_at_ms: Option<i64>,
    pub redaction_state: String,
    pub redaction_categories: Vec<String>,
    pub routing_mode: String,
    pub provider_ids: Vec<String>,
    pub created_at_ms: i64,
    pub latest_transcript: Option<TranscriptRecord>,
    pub related_asset_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaAssetData {
    pub media_asset_id: String,
    pub mime_type: String,
    pub data_base64: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveMediaInterpretationRequest {
    pub project_id: String,
    pub media_asset_id: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInterpretation {
    pub revision: u64,
    pub interpretation_kind: String,
    pub text: String,
    pub route_label: String,
    pub edited_by_user: bool,
    pub created_at_ms: i64,
}
