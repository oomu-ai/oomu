use super::super::*;
use crate::inference::approved_file_receipts::project_verified_native_execution_receipt;

#[test]
fn signed_direct_read_receipt_projects_verified_assistant_metadata() {
    let identity = SovereignIdentity::initialize_ephemeral();
    let content = "Three verified facts from the approved local file.";
    let receipt = crate::shield_gate::tests::signed_approved_file_receipt_fixture(
        &identity,
        "session-3c",
        "turn-3c",
        "agent-3c",
        "Read the approved file and summarize only its stated facts.",
        "q3_strategic_vendor_proposals.txt",
        content,
    );
    let mut attachments = vec![ChatAttachment {
        name: "q3_strategic_vendor_proposals.txt".to_string(),
        mime_type: "text/plain".to_string(),
        byte_count: content.len(),
        data_base64: None,
        text: None,
        approved_file_receipt: Some(receipt),
    }];

    let hydration = hydrate_approved_file_receipts(
        &mut attachments,
        &identity,
        "session-3c",
        "turn-3c",
        "agent-3c",
    )
    .expect("the native signed receipt must hydrate");
    assert_eq!(hydration.verified_receipt_count, 1);
    assert_eq!(attachments[0].text.as_deref(), Some(content));

    let mut metadata = serde_json::Map::new();
    project_verified_native_execution_receipt(&mut metadata, hydration.verified_receipt_count > 0);
    assert_eq!(
        metadata.get("verifiedNativeExecutionReceipt"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn tampered_direct_read_receipt_cannot_project_verified_metadata() {
    let identity = SovereignIdentity::initialize_ephemeral();
    let content = "Private approved file content.";
    let mut receipt = crate::shield_gate::tests::signed_approved_file_receipt_fixture(
        &identity,
        "session-3c",
        "turn-3c",
        "agent-3c",
        "Read the approved file.",
        "q3_strategic_vendor_proposals.txt",
        content,
    );
    receipt.payload.push('A');
    let mut attachments = vec![ChatAttachment {
        name: "q3_strategic_vendor_proposals.txt".to_string(),
        mime_type: "text/plain".to_string(),
        byte_count: content.len(),
        data_base64: None,
        text: None,
        approved_file_receipt: Some(receipt),
    }];

    assert!(hydrate_approved_file_receipts(
        &mut attachments,
        &identity,
        "session-3c",
        "turn-3c",
        "agent-3c",
    )
    .is_err());
    let mut metadata = serde_json::Map::new();
    project_verified_native_execution_receipt(&mut metadata, false);
    assert!(!metadata.contains_key("verifiedNativeExecutionReceipt"));
}
