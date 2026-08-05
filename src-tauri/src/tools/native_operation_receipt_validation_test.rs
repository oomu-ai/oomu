#[test]
fn continuation_authority_validation_does_not_consume_the_receipt() {
    let receipt = receipt_fixture(
        NativeOperationOutcome::Succeeded,
        NativePostconditionEvidence {
            evidence_kind: "native_result",
            operation_succeeded: true,
            verified: true,
            bounded_count: None,
            truncated: None,
            native_result_code: Some("mail_read_ok".to_string()),
            durable_operation_binding: None,
            capture_proof: None,
        },
    );
    let mut ledger = ContinuationReceiptLedger::default();
    ledger.register(&receipt);
    let parent = receipt_parent_context();

    assert_eq!(ledger.validate(&receipt.receipt_id, &parent), Ok(()));
    assert_eq!(ledger.validate(&receipt.receipt_id, &parent), Ok(()));
    assert!(ledger.consume(&receipt.receipt_id, &parent).is_ok());
    assert_eq!(
        ledger.validate(&receipt.receipt_id, &parent),
        Err(NativeReceiptConsumptionError::Replayed)
    );
}
