use serde::{Deserialize, Serialize};

#[cfg(test)]
pub const REMOTE_SCOPES: [&str; 7] = [
    "create_task",
    "view_task",
    "steer_task",
    "stop_task",
    "answer_clarification",
    "approve_bounded_action",
    "request_artifact",
];

#[cfg(test)]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatePairingChallengeRequest {
    pub allowed_project_ids: Vec<String>,
    pub scopes: Vec<String>,
}

#[cfg(test)]
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingChallenge {
    pub challenge_id: String,
    pub qr_svg: String,
    pub expires_at_ms: i64,
    pub status: String,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitPairingResponseRequest {
    pub challenge_id: String,
    pub secret: String,
    pub device_label: String,
    pub public_key: String,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmPairingRequest {
    pub challenge_id: String,
    pub allow: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteDeviceRequest {
    pub remote_device_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenameRemoteDeviceRequest {
    pub remote_device_id: String,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDeviceRecord {
    pub remote_device_id: String,
    pub label: String,
    pub allowed_project_ids: Vec<String>,
    pub scopes: Vec<String>,
    pub paired_at_ms: i64,
    pub expires_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedRemoteCommand {
    pub command_id: String,
    pub remote_device_id: String,
    pub project_id: String,
    pub task_run_id: Option<String>,
    pub command_kind: String,
    pub nonce: String,
    pub expires_at_ms: i64,
    pub expected_task_sequence: Option<u64>,
    pub payload_sha256: String,
    pub signer_public_key: String,
    pub payload: serde_json::Value,
    pub signature: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCommandResult {
    pub command_id: String,
    pub status: String,
    pub outcome_code: String,
    pub message: String,
    pub task: Option<crate::tasks::TaskRunRecord>,
    pub artifact_grant: Option<RemoteArtifactGrant>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteArtifactGrant {
    pub token: String,
    pub artifact_id: String,
    pub format: String,
    pub content_state: String,
    pub transfer_sha256: String,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetrieveRemoteArtifactRequest {
    pub remote_device_id: String,
    pub token: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedRemoteArtifact {
    pub artifact_id: String,
    pub format: String,
    pub nonce_base64: String,
    pub ciphertext_base64: String,
    pub ciphertext_sha256: String,
    pub source_sha256: String,
    pub transfer_sha256: String,
    pub content_state: String,
    pub expires_at_ms: i64,
    pub associated_data_base64: String,
}
