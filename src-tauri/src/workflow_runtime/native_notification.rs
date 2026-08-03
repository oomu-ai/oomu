use super::WorkflowRuntimeError;
use crate::tools::native_operation_receipt::{
    AppleCapability, NativeActionClass, NativeOperationAttempt, NativePostconditionEvidence,
};
use serde_json::{json, Value};
use tauri::Manager;

pub(super) fn is_tool(server_name: &str, tool_name: &str) -> bool {
    server_name.trim().eq_ignore_ascii_case("macos_applescript")
        && tool_name
            .trim()
            .eq_ignore_ascii_case("trigger_system_notification")
}

pub(super) fn execute(
    app: &tauri::AppHandle,
    execution_id: &str,
    arguments: &Value,
    human_approved: bool,
) -> Result<Value, WorkflowRuntimeError> {
    if !human_approved {
        return Err(WorkflowRuntimeError::execution(
            "This notification still needs your approval. Nothing was shown.".to_string(),
        ));
    }
    let (title, subtitle, body) = validated_text(arguments)?;
    let persistence = app.state::<crate::db::PersistenceEngine>().inner().clone();
    let result_copy = localized_result(&persistence)?;
    let binding = crate::foundation::digest::sha256_hex(
        serde_json::to_vec(arguments)
            .map_err(WorkflowRuntimeError::serialization)?
            .as_slice(),
    );
    let attempt = block_on(NativeOperationAttempt::begin_for_workflow_execution(
        AppleCapability::Notifications,
        NativeActionClass::Notify,
        human_approved,
        binding,
        &persistence,
        execution_id,
        "macos_applescript",
        "trigger_system_notification",
        arguments,
    ))
    .ok_or_else(|| {
        WorkflowRuntimeError::execution(
            "OOMU could not verify which approved request owns this notification. Nothing was shown."
                .to_string(),
        )
    })?;
    let result = crate::native_capability_adapters::deliver_notification_and_verify_blocking(
        &title, &subtitle, &body,
    );
    let evidence = match result.as_ref() {
        Ok(delivery) => NativePostconditionEvidence {
            evidence_kind: "delivered_notification",
            operation_succeeded: true,
            verified: delivery.verified(),
            bounded_count: Some(1),
            truncated: Some(false),
            native_result_code: Some("delivered".to_string()),
            durable_operation_binding: None,
            capture_proof: None,
        },
        Err(code) => NativePostconditionEvidence {
            evidence_kind: "notification_delivery_error",
            operation_succeeded: false,
            verified: false,
            bounded_count: None,
            truncated: None,
            native_result_code: Some(code.chars().take(80).collect()),
            durable_operation_binding: None,
            capture_proof: None,
        },
    };
    block_on(attempt.finish(evidence));
    let delivery = result.map_err(|_| WorkflowRuntimeError::notification_unavailable())?;
    Ok(json!({
        "content": [{ "type": "text", "text": result_copy }],
        "structuredContent": {
            "status": "delivered",
            "verified": delivery.verified(),
            "notificationId": delivery.notification_id,
            "submitted": delivery.submitted,
            "delivered": delivery.delivered
        },
        "isError": false
    }))
}

fn localized_result(
    persistence: &crate::db::PersistenceEngine,
) -> Result<String, WorkflowRuntimeError> {
    crate::settings::locale_state_for_engine(persistence, None)
        .map_err(|_| WorkflowRuntimeError::execution("notification_copy_unavailable".to_string()))?
        .translations
        .pointer("/sprint_301/notification/result")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| WorkflowRuntimeError::execution("notification_copy_unavailable".to_string()))
}

fn validated_text(arguments: &Value) -> Result<(String, String, String), WorkflowRuntimeError> {
    let object = arguments.as_object().ok_or_else(|| {
        WorkflowRuntimeError::execution(
            "The notification step did not receive valid details.".to_string(),
        )
    })?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "title_text"
                | "titleText"
                | "subtitle_text"
                | "subtitleText"
                | "body_text"
                | "bodyText"
        )
    }) {
        return Err(WorkflowRuntimeError::execution(
            "The notification step contains details OOMU does not recognize.".to_string(),
        ));
    }
    let text = |snake: &str, camel: &str, maximum: usize| {
        let value = object
            .get(snake)
            .or_else(|| object.get(camel))
            .and_then(Value::as_str)
            .unwrap_or_default();
        (value.chars().count() <= maximum)
            .then(|| value.to_string())
            .ok_or_else(|| {
                WorkflowRuntimeError::execution(
                    "The notification text is too long to show safely.".to_string(),
                )
            })
    };
    let values = (
        text("title_text", "titleText", 256)?,
        text("subtitle_text", "subtitleText", 256)?,
        text("body_text", "bodyText", 1_024)?,
    );
    if values.2.trim().is_empty() {
        return Err(WorkflowRuntimeError::execution(
            "The notification needs a message to show.".to_string(),
        ));
    }
    Ok(values)
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(future),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("native notification runtime")
            .block_on(future),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_arguments_are_strict_and_require_a_body() {
        assert!(validated_text(&json!({"bodyText":"Ready"})).is_ok());
        assert!(validated_text(&json!({"bodyText":""})).is_err());
        assert!(validated_text(&json!({"bodyText":"Ready","send":true})).is_err());
        assert!(validated_text(&json!({"bodyText":"x".repeat(1_025)})).is_err());
    }
}
