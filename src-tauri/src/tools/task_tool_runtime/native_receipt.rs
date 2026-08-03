use crate::{
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::native_operation_receipt::{
        AppleCapability, NativeActionClass, NativeOperationAttempt, NativePostconditionEvidence,
    },
};
use serde_json::Value;

pub(super) async fn begin(
    persistence: &crate::db::PersistenceEngine,
    execution_id: Option<&str>,
    operation: &str,
    arguments: &Value,
) -> Option<NativeOperationAttempt> {
    let (capability, action) = operation_spec(operation)?;
    let execution_id = execution_id?;
    let action_binding =
        crate::foundation::digest::sha256_hex(serde_json::to_vec(arguments).ok()?.as_slice());
    NativeOperationAttempt::begin_for_registered_task_execution(
        capability,
        action,
        action_binding,
        persistence,
        execution_id,
        operation,
        arguments,
    )
    .await
}

pub(super) async fn finish(
    attempt: Option<NativeOperationAttempt>,
    result: &Result<ExecuteCommandResponse, String>,
) {
    if let Some(attempt) = attempt {
        attempt.finish(evidence(result)).await;
    }
}

fn operation_spec(operation: &str) -> Option<(AppleCapability, NativeActionClass)> {
    matches!(
        operation,
        "create_system_calendar_event"
            | "create_conflict_free_calendar_event"
            | "create_release_recovery_calendar_event"
    )
    .then_some((AppleCapability::Calendar, NativeActionClass::Write))
}

fn evidence(result: &Result<ExecuteCommandResponse, String>) -> NativePostconditionEvidence {
    let Ok(response) = result else {
        return failure_evidence("execution_failed");
    };
    if !matches!(response.status, CommandStatus::Completed) || !response.verified {
        return failure_evidence("execution_unverified");
    }
    let value = serde_json::from_str::<Value>(&response.message).ok();
    let event_id = value
        .as_ref()
        .and_then(|value| value.get("eventId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let verified = value
        .as_ref()
        .and_then(|value| value.get("verified"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && value
            .as_ref()
            .and_then(|value| value.get("exists"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        && event_id.is_some();
    NativePostconditionEvidence {
        evidence_kind: "verified_calendar_event",
        operation_succeeded: true,
        verified,
        bounded_count: verified.then_some(1),
        truncated: Some(false),
        native_result_code: Some(if verified {
            "calendar_event_verified".to_string()
        } else {
            "calendar_event_unverified".to_string()
        }),
        durable_operation_binding: event_id.map(|event_id| {
            crate::foundation::digest::sha256_hex(
                format!("calendar-event-v1\0{event_id}").as_bytes(),
            )
        }),
        capture_proof: None,
    }
}

fn failure_evidence(code: &str) -> NativePostconditionEvidence {
    NativePostconditionEvidence {
        evidence_kind: "calendar_event_error",
        operation_succeeded: false,
        verified: false,
        bounded_count: None,
        truncated: None,
        native_result_code: Some(code.to_string()),
        durable_operation_binding: None,
        capture_proof: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_calendar_task_tools_map_to_native_receipts() {
        for operation in [
            "create_system_calendar_event",
            "create_conflict_free_calendar_event",
            "create_release_recovery_calendar_event",
        ] {
            assert_eq!(
                operation_spec(operation),
                Some((AppleCapability::Calendar, NativeActionClass::Write))
            );
        }
        assert_eq!(operation_spec("create_file"), None);
    }

    #[test]
    fn calendar_write_evidence_requires_the_verified_native_event_id() {
        let verified = Ok(ExecuteCommandResponse {
            operation: "create_system_calendar_event".to_string(),
            status: CommandStatus::Completed,
            message: serde_json::json!({
                "verified": true,
                "exists": true,
                "eventId": "native-event-1"
            })
            .to_string(),
            metrics: None,
            claims: Vec::new(),
            verified: true,
            model_used: None,
        });
        let verified_evidence = evidence(&verified);
        assert!(verified_evidence.verified);
        assert_eq!(
            verified_evidence
                .durable_operation_binding
                .as_deref()
                .map(str::len),
            Some(64)
        );

        let narrated = Ok(ExecuteCommandResponse {
            message: "The event was created.".to_string(),
            ..verified.unwrap()
        });
        let narrated_evidence = evidence(&narrated);
        assert!(!narrated_evidence.verified);
        assert!(narrated_evidence.durable_operation_binding.is_none());
    }
}
