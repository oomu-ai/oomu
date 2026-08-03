use super::*;

pub(crate) fn signed_approved_file_receipt_fixture(
    identity: &SovereignIdentity,
    session_id: &str,
    root_turn_id: &str,
    agent_id: &str,
    display_message: &str,
    display_name: &str,
    content: &str,
) -> ApprovedFileReceiptToken {
    let now_ms = unix_time_ms_i64();
    let payload = ApprovedFileReceiptPayload {
        version: APPROVED_CHAT_FILE_RECEIPT_VERSION,
        receipt_id: "a".repeat(48),
        session_id: session_id.to_string(),
        issued_turn_id: root_turn_id.to_string(),
        root_turn_id: root_turn_id.to_string(),
        agent_id: agent_id.to_string(),
        target_identity_hash: "b".repeat(64),
        display_name: display_name.to_string(),
        mime_type: "text/plain".to_string(),
        byte_count: content.len(),
        content_sha256: sha256_hex(content.as_bytes()),
        content: content.to_string(),
        media_sha256: None,
        display_message: display_message.to_string(),
        issued_at_ms: now_ms,
        expires_at_ms: now_ms + 60_000,
    };
    let payload_json = serde_json::to_string(&payload).unwrap();
    ApprovedFileReceiptToken {
        payload: URL_SAFE_NO_PAD.encode(payload_json.as_bytes()),
        signature: identity.sign_payload(&payload_json).unwrap(),
    }
}

mod approval;
mod file_binding;
mod patch;
mod security;
