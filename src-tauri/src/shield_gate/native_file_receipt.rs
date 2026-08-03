use crate::{
    db::ChatTurnPersistenceContext,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::native_operation_receipt::{
        AppleCapability, NativeActionClass, NativeOperationAttempt, NativePostconditionEvidence,
    },
};

pub(super) async fn begin(
    turn: Option<&ChatTurnPersistenceContext>,
    operation: &str,
    path: Option<&str>,
) -> Option<NativeOperationAttempt> {
    let action = action_for(operation)?;
    let path = path?.trim();
    if path.is_empty() {
        return None;
    }
    NativeOperationAttempt::begin(
        AppleCapability::FilesAndFolders,
        action,
        matches!(action, NativeActionClass::Write | NativeActionClass::Delete),
        crate::foundation::digest::sha256_hex(path.as_bytes()),
        turn,
    )
    .await
}

pub(super) async fn finish(
    attempt: Option<NativeOperationAttempt>,
    response: &ExecuteCommandResponse,
) {
    if let Some(attempt) = attempt {
        let verified = matches!(response.status, CommandStatus::Completed) && response.verified;
        attempt
            .finish(NativePostconditionEvidence {
                evidence_kind: "verified_scoped_file_operation",
                operation_succeeded: verified,
                verified,
                bounded_count: verified.then_some(1),
                truncated: None,
                native_result_code: Some(if verified {
                    "completed".to_string()
                } else {
                    "failed_or_unverified".to_string()
                }),
                durable_operation_binding: None,
                capture_proof: None,
            })
            .await;
    }
}

fn action_for(operation: &str) -> Option<NativeActionClass> {
    match operation {
        "file_read" | "file_list" => Some(NativeActionClass::Read),
        "file_write" => Some(NativeActionClass::Write),
        "delete_file" => Some(NativeActionClass::Delete),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_direct_file_operations_map_to_receipts() {
        assert_eq!(action_for("file_read"), Some(NativeActionClass::Read));
        assert_eq!(action_for("file_list"), Some(NativeActionClass::Read));
        assert_eq!(action_for("file_write"), Some(NativeActionClass::Write));
        assert_eq!(action_for("delete_file"), Some(NativeActionClass::Delete));
        assert_eq!(action_for("terminal_execute"), None);
    }
}
