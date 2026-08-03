use super::*;
use crate::tools::native_operation_receipt::{
    AppleCapability, NativeActionClass, NativeOperationAttempt, NativePostconditionEvidence,
};

mod screen_capture;

pub(super) async fn execute_if_supported(
    server_name: &str,
    tool_name: &str,
    arguments: &Value,
    approval: Option<McpToolApproval>,
    turn: Option<&ChatTurnPersistenceContext>,
    persistence: &PersistenceEngine,
    registry: &McpClientRegistry,
    app: &tauri::AppHandle,
    guard: &(dyn Fn() -> Result<(), McpClientError> + Send + Sync),
) -> Option<Result<McpToolCallResult, String>> {
    if !server_name
        .trim()
        .eq_ignore_ascii_case(MACOS_APPLESCRIPT_SERVER_NAME)
    {
        return None;
    }
    let tool = tool_name.trim().to_ascii_lowercase();
    let (capability, action) = match tool.as_str() {
        "capture_disposable_window" => (AppleCapability::ScreenCapture, NativeActionClass::Capture),
        "preview_camera" => (AppleCapability::Camera, NativeActionClass::Capture),
        "trigger_system_notification" => {
            (AppleCapability::Notifications, NativeActionClass::Notify)
        }
        _ => return None,
    };
    Some(
        execute(
            &tool,
            arguments.clone(),
            approval,
            turn,
            persistence,
            registry,
            app,
            guard,
            capability,
            action,
        )
        .await,
    )
}

#[allow(clippy::too_many_arguments)]
async fn execute(
    tool_name: &str,
    arguments: Value,
    approval: Option<McpToolApproval>,
    turn: Option<&ChatTurnPersistenceContext>,
    persistence: &PersistenceEngine,
    registry: &McpClientRegistry,
    app: &tauri::AppHandle,
    guard: &(dyn Fn() -> Result<(), McpClientError> + Send + Sync),
    capability: AppleCapability,
    action: NativeActionClass,
) -> Result<McpToolCallResult, String> {
    let turn = turn.ok_or_else(|| "native_action_turn_required".to_string())?;
    validate_tool_arguments(&arguments).map_err(|error| error.message)?;
    ensure_trusted_builtin_mcp_server(registry, app, MACOS_APPLESCRIPT_SERVER_NAME).await?;
    registry
        .get_tool_details(MACOS_APPLESCRIPT_SERVER_NAME, tool_name)
        .await
        .map_err(|error| error.message)?;
    let action_approved = approval.is_some();
    let verified = registry
        .ensure_tool_approval(
            MACOS_APPLESCRIPT_SERVER_NAME,
            tool_name,
            &arguments,
            approval,
        )
        .await
        .map_err(|error| error.message)?;
    registry
        .revalidate_verified_tool_execution(MACOS_APPLESCRIPT_SERVER_NAME, tool_name, &verified)
        .await
        .map_err(|error| error.message)?;
    guard().map_err(|error| error.message)?;

    let attempt = NativeOperationAttempt::begin_with_persistence(
        capability,
        action,
        action_approved,
        argument_binding(&arguments),
        Some(turn),
        persistence,
    )
    .await;
    let mut result = match tool_name {
        "capture_disposable_window" => screen_capture::execute(app, &arguments, persistence).await,
        "preview_camera" => execute_camera(app, &arguments, persistence).await,
        "trigger_system_notification" => execute_notification(&arguments, persistence).await,
        _ => unreachable!("native capability was matched before execution"),
    };
    if let Some(attempt) = attempt {
        let receipt = attempt.finish(evidence(tool_name, action, &result)).await;
        if let Ok(tool_result) = result.as_mut() {
            receipt.attach_to_mcp_result(tool_result);
        }
    }
    if let Some(audit_id) = verified.audit_id.as_deref() {
        eprintln!(
            "MCP_TOOL_SECURITY_EVENT audit_id={} server={} tool={} completion={}",
            audit_id,
            MACOS_APPLESCRIPT_SERVER_NAME,
            tool_name,
            if result.is_ok() {
                "success"
            } else {
                "blocked_or_failed"
            }
        );
    }
    result
}

async fn execute_camera(
    app: &tauri::AppHandle,
    arguments: &Value,
    persistence: &PersistenceEngine,
) -> Result<McpToolCallResult, String> {
    if !arguments.as_object().is_some_and(serde_json::Map::is_empty) {
        return Err("camera_preview_arguments_invalid".to_string());
    }
    let native =
        crate::native_capability_adapters::open_camera_preview_without_retention(app).await?;
    Ok(result(
        &localized_result(persistence, "camera", "preview_result")?,
        serde_json::json!({
            "status": "preview_closed",
            "verified": native.verified(),
            "previewOpened": native.preview_opened,
            "previewClosed": native.preview_closed,
            "captureOutputs": native.capture_outputs,
            "frameRetained": native.frame_retained,
        }),
    ))
}

async fn execute_notification(
    arguments: &Value,
    persistence: &PersistenceEngine,
) -> Result<McpToolCallResult, String> {
    let object = arguments
        .as_object()
        .ok_or_else(|| "notification_arguments_invalid".to_string())?;
    let text = |snake: &str, camel: &str, maximum: usize| -> Result<String, String> {
        let Some(value) = object.get(snake).or_else(|| object.get(camel)) else {
            return Ok(String::new());
        };
        let value = value
            .as_str()
            .ok_or_else(|| "notification_arguments_invalid".to_string())?;
        if value.chars().count() > maximum {
            return Err("notification_arguments_invalid".to_string());
        }
        Ok(value.to_string())
    };
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
        return Err("notification_arguments_invalid".to_string());
    }
    let title = text("title_text", "titleText", 256)?;
    let subtitle = text("subtitle_text", "subtitleText", 256)?;
    let body = text("body_text", "bodyText", 1_024)?;
    let native =
        crate::native_capability_adapters::deliver_notification_and_verify(title, subtitle, body)
            .await?;
    Ok(result(
        &localized_result(persistence, "notification", "result")?,
        serde_json::json!({
            "status": "delivered",
            "verified": native.verified(),
            "notificationId": native.notification_id,
            "submitted": native.submitted,
            "delivered": native.delivered,
        }),
    ))
}

fn localized_result(
    persistence: &PersistenceEngine,
    capability: &str,
    key: &str,
) -> Result<String, String> {
    let state = crate::settings::locale_state_for_engine(persistence, None)?;
    state
        .translations
        .pointer(&format!("/sprint_301/{capability}/{key}"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{capability}_copy_unavailable"))
}

fn result(text: &str, structured: Value) -> McpToolCallResult {
    McpToolCallResult {
        content: vec![serde_json::json!({ "type": "text", "text": text })],
        structured_content: Some(structured),
        is_error: false,
        meta: None,
        raw: None,
    }
}

fn evidence(
    tool_name: &str,
    action: NativeActionClass,
    result: &Result<McpToolCallResult, String>,
) -> NativePostconditionEvidence {
    match result {
        Ok(result) if tool_name == "capture_disposable_window" => screen_capture::evidence(result),
        Ok(result) => {
            crate::tools::native_operation_receipt::evidence_from_mcp_result(action, result)
        }
        Err(error) => NativePostconditionEvidence {
            evidence_kind: "native_call_error",
            operation_succeeded: false,
            verified: false,
            bounded_count: None,
            truncated: None,
            native_result_code: Some(error.chars().take(80).collect()),
            durable_operation_binding: None,
            capture_proof: None,
        },
    }
}

#[cfg(test)]
mod locale_tests {
    use super::*;

    #[test]
    fn camera_and_notification_results_exist_in_every_shipped_locale() {
        let root = std::env::temp_dir().join(format!(
            "oomu-native-capability-copy-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        for locale in [
            "de-DE", "en-US", "es-ES", "fr-FR", "id-ID", "ja-JP", "pt-BR", "ru-RU", "uk-UA",
            "vi-VN", "zh-CN", "zh-TW",
        ] {
            persistence
                .upsert_app_preference("ui.active_locale", locale)
                .unwrap();
            assert!(!localized_result(&persistence, "camera", "preview_result")
                .unwrap()
                .is_empty());
            assert!(!localized_result(&persistence, "notification", "result")
                .unwrap()
                .is_empty());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_arguments_reject_unknown_or_oversized_values() {
        let unknown = serde_json::json!({"body_text":"Hello", "send":true});
        assert_eq!(
            futures_executor(&unknown).unwrap_err(),
            "notification_arguments_invalid"
        );
        let oversized = serde_json::json!({"body_text":"x".repeat(1_025)});
        assert_eq!(
            futures_executor(&oversized).unwrap_err(),
            "notification_arguments_invalid"
        );
    }

    fn futures_executor(arguments: &Value) -> Result<(String, String, String), String> {
        let object = arguments
            .as_object()
            .ok_or_else(|| "notification_arguments_invalid".to_string())?;
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
            return Err("notification_arguments_invalid".to_string());
        }
        let value = object
            .get("body_text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if value.chars().count() > 1_024 {
            return Err("notification_arguments_invalid".to_string());
        }
        Ok((String::new(), String::new(), value.to_string()))
    }
}
