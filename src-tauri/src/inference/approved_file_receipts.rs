use super::ChatAttachment;
use crate::sovereign_identity::SovereignIdentity;
use serde_json::{Map, Value};

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ApprovedFileReceiptHydration {
    pub(super) display_message: Option<String>,
    pub(super) verified_receipt_count: usize,
}

pub(super) fn hydrate_approved_file_receipts(
    attachments: &mut [ChatAttachment],
    identity: &SovereignIdentity,
    session_id: &str,
    root_turn_id: &str,
    agent_id: &str,
) -> Result<ApprovedFileReceiptHydration, String> {
    let mut hydration = ApprovedFileReceiptHydration::default();
    for attachment in attachments {
        let Some(receipt) = attachment.approved_file_receipt.take() else {
            continue;
        };
        let verified = crate::shield_gate::verify_approved_file_receipt(
            &receipt,
            identity,
            session_id,
            root_turn_id,
            agent_id,
        )?;
        if attachment.name.trim() != verified.display_name
            || attachment.mime_type.trim() != verified.mime_type
            || attachment.byte_count != verified.byte_count
        {
            return Err("receipt_attachment_metadata_mismatch".to_string());
        }
        if let Some(existing) = hydration.display_message.as_deref() {
            if existing != verified.display_message {
                return Err("receipt_display_message_mismatch".to_string());
            }
        } else {
            hydration.display_message = Some(verified.display_message.clone());
        }
        attachment.data_base64 = verified.data_base64;
        attachment.text = Some(verified.content);
        hydration.verified_receipt_count = hydration.verified_receipt_count.saturating_add(1);
    }
    Ok(hydration)
}

pub(super) fn project_verified_native_execution_receipt(
    metadata: &mut Map<String, Value>,
    verified: bool,
) {
    if verified {
        metadata.insert(
            "verifiedNativeExecutionReceipt".to_string(),
            Value::Bool(true),
        );
    }
}
