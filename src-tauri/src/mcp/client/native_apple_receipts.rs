use super::*;
use crate::tools::native_operation_receipt::{
    evidence_from_mcp_result, AppleCapability, NativeActionClass, NativeOperationAttempt,
    NativePostconditionEvidence,
};
use std::future::Future;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReceiptSpec {
    capability: AppleCapability,
    action: NativeActionClass,
    action_binding: String,
}

pub(super) fn spec_for(
    server_name: &str,
    tool_name: &str,
    arguments: &Value,
) -> Option<ReceiptSpec> {
    let server = server_name.trim().to_ascii_lowercase();
    let tool = tool_name.trim().to_ascii_lowercase();
    if server == "local_filesystem" {
        let action = match tool.as_str() {
            "list_directory" | "read_file" => NativeActionClass::Read,
            "write_file" => NativeActionClass::Write,
            "delete_file" => NativeActionClass::Delete,
            _ => return None,
        };
        return Some(ReceiptSpec {
            capability: AppleCapability::FilesAndFolders,
            action,
            action_binding: argument_binding(arguments),
        });
    }
    if server != MACOS_APPLESCRIPT_SERVER_NAME {
        return None;
    }
    let (capability, action) = match tool.as_str() {
        "read_system_calendar" => (AppleCapability::Calendar, NativeActionClass::Read),
        "read_system_contacts" => (AppleCapability::Contacts, NativeActionClass::Read),
        "read_system_photos" => (AppleCapability::Photos, NativeActionClass::Read),
        "read_system_music" => (AppleCapability::Music, NativeActionClass::Read),
        "read_system_emails" => (AppleCapability::Mail, NativeActionClass::Read),
        "read_system_notes" => (AppleCapability::Notes, NativeActionClass::Read),
        "read_system_reminders" => (AppleCapability::Reminders, NativeActionClass::Read),
        "add_system_reminder" => (AppleCapability::Reminders, NativeActionClass::Write),
        "create_system_note" => (AppleCapability::Notes, NativeActionClass::Write),
        "prepare_system_message" => (AppleCapability::Messages, NativeActionClass::Draft),
        "draft_system_email" => (AppleCapability::Mail, NativeActionClass::Draft),
        "send_system_email" => (AppleCapability::Mail, NativeActionClass::Send),
        "trigger_system_notification" => {
            (AppleCapability::Notifications, NativeActionClass::Notify)
        }
        "read_apple_app_ui" => (ui_read_capability(arguments), NativeActionClass::Read),
        _ => return None,
    };
    Some(ReceiptSpec {
        capability,
        action,
        action_binding: argument_binding(arguments),
    })
}

pub(super) async fn execute<F>(
    spec: Option<ReceiptSpec>,
    turn: Option<&ChatTurnPersistenceContext>,
    persistence: &PersistenceEngine,
    action_approved: bool,
    future: F,
) -> Result<McpToolCallResult, String>
where
    F: Future<Output = Result<McpToolCallResult, String>>,
{
    if spec.is_some() && turn.is_none() {
        return Err("native_apple_operation_requires_accepted_turn".to_string());
    }
    let attempt = match spec.as_ref() {
        Some(spec) => {
            NativeOperationAttempt::begin_with_persistence(
                spec.capability,
                spec.action,
                action_approved,
                spec.action_binding.clone(),
                turn,
                persistence,
            )
            .await
        }
        None => None,
    };
    let mut result = future.await;
    if let (Some(attempt), Some(spec)) = (attempt, spec.as_ref()) {
        let evidence = match result.as_ref() {
            Ok(result) => evidence_from_mcp_result(spec.action, result),
            Err(_) => NativePostconditionEvidence {
                evidence_kind: "native_call_error",
                operation_succeeded: false,
                verified: false,
                bounded_count: None,
                truncated: None,
                native_result_code: Some("execution_failed".to_string()),
                durable_operation_binding: None,
                capture_proof: None,
            },
        };
        let receipt = attempt.finish(evidence).await;
        if let Ok(tool_result) = result.as_mut() {
            receipt.attach_to_mcp_result(tool_result);
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_for_workflow<F>(
    spec: Option<ReceiptSpec>,
    persistence: &PersistenceEngine,
    execution_id: &str,
    server_name: &str,
    tool_name: &str,
    arguments: &Value,
    action_approved: bool,
    future: F,
) -> Result<McpToolCallResult, String>
where
    F: Future<Output = Result<McpToolCallResult, String>>,
{
    let attempt = match spec.as_ref() {
        Some(spec) => Some(
            NativeOperationAttempt::begin_for_workflow_execution(
                spec.capability,
                spec.action,
                action_approved,
                spec.action_binding.clone(),
                persistence,
                execution_id,
                server_name,
                tool_name,
                arguments,
            )
            .await
            .ok_or_else(|| "native_apple_workflow_operation_missing_authority".to_string())?,
        ),
        None => None,
    };
    let mut result = future.await;
    if let (Some(attempt), Some(spec)) = (attempt, spec.as_ref()) {
        let evidence = match result.as_ref() {
            Ok(result) => evidence_from_mcp_result(spec.action, result),
            Err(_) => NativePostconditionEvidence {
                evidence_kind: "native_call_error",
                operation_succeeded: false,
                verified: false,
                bounded_count: None,
                truncated: None,
                native_result_code: Some("execution_failed".to_string()),
                durable_operation_binding: None,
                capture_proof: None,
            },
        };
        let receipt = attempt.finish(evidence).await;
        if let Ok(tool_result) = result.as_mut() {
            receipt.attach_to_mcp_result(tool_result);
            if !receipt.verified_success() {
                tool_result.is_error = true;
            }
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_workflow_tool(
    registry: &McpClientRegistry,
    persistence: &PersistenceEngine,
    execution_id: &str,
    node_id: &str,
    server_name: &str,
    tool_name: &str,
    arguments: Value,
    approval: Option<McpToolApproval>,
    action_approved: bool,
) -> Result<McpToolCallResult, String> {
    let spec = spec_for(server_name, tool_name, &arguments);
    if spec.is_some() {
        bind_workflow_node(persistence, execution_id, node_id)?;
    }
    let execution_arguments = arguments.clone();
    execute_for_workflow(
        spec,
        persistence,
        execution_id,
        server_name,
        tool_name,
        &arguments,
        action_approved,
        async move {
            registry
                .execute_tool_with_approval(server_name, tool_name, execution_arguments, approval)
                .await
                .map_err(|error| error.message)
        },
    )
    .await
}

fn bind_workflow_node(
    persistence: &PersistenceEngine,
    execution_id: &str,
    node_id: &str,
) -> Result<(), String> {
    let execution_id = execution_id.trim();
    let node_id = node_id.trim();
    if execution_id.is_empty() || node_id.is_empty() {
        return Err("native_apple_workflow_operation_missing_authority".to_string());
    }
    let changed = persistence
        .open_connection()
        .map_err(|_| "native_apple_workflow_operation_missing_authority".to_string())?
        .execute(
            "UPDATE execution_instances SET active_node_id=?2,updated_at_ms=?3 WHERE id=?1 AND status IN ('Running','AwaitingApproval')",
            rusqlite::params![execution_id, node_id, crate::foundation::clock::unix_time_ms_i64()],
        )
        .map_err(|_| "native_apple_workflow_operation_missing_authority".to_string())?;
    (changed == 1)
        .then_some(())
        .ok_or_else(|| "native_apple_workflow_operation_missing_authority".to_string())
}

fn ui_read_capability(arguments: &Value) -> AppleCapability {
    let requested = arguments
        .get("app_name")
        .or_else(|| arguments.get("appName"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match requested.as_str() {
        "messages" => AppleCapability::Messages,
        "finder" => AppleCapability::Finder,
        _ => AppleCapability::SystemEvents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{AcceptChatTurnRequest, CreateChatSessionRequest},
        tools::native_operation_receipt::consume_chat_turn_receipt,
    };

    fn persistence(
        name: &str,
    ) -> (
        std::path::PathBuf,
        PersistenceEngine,
        ChatTurnPersistenceContext,
    ) {
        let root = std::env::temp_dir().join(format!(
            "oomu-native-receipt-{name}-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = persistence
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: format!("agent-{name}"),
                provider_id: "local".to_string(),
                model_id: "model".to_string(),
                title: Some(format!("Native receipt {name}")),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        let request = AcceptChatTurnRequest {
            turn_id: format!("turn-{name}"),
            generation_token: format!("generation-{name}"),
            parent_turn_id: None,
            root_turn_id: format!("turn-{name}"),
            turn_kind: "root".to_string(),
            session_id: session.id,
            agent_id: session.agent_id,
            provider_id: "local".to_string(),
            model_id: "model".to_string(),
            message: format!("Read the calendar for {name}."),
        };
        persistence.accept_chat_turn(request.clone()).unwrap();
        (root, persistence, request.persistence_context())
    }

    #[test]
    fn maps_only_real_builtin_apple_and_filesystem_tools() {
        let calendar = spec_for(
            MACOS_APPLESCRIPT_SERVER_NAME,
            "read_system_calendar",
            &serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(calendar.capability, AppleCapability::Calendar);
        assert_eq!(calendar.action, NativeActionClass::Read);

        let messages = spec_for(
            MACOS_APPLESCRIPT_SERVER_NAME,
            "read_apple_app_ui",
            &serde_json::json!({"app_name":"Messages"}),
        )
        .unwrap();
        assert_eq!(messages.capability, AppleCapability::Messages);

        let prepared = spec_for(
            MACOS_APPLESCRIPT_SERVER_NAME,
            "prepare_system_message",
            &serde_json::json!({"recipient":"+15555550123","body":"Test"}),
        )
        .unwrap();
        assert_eq!(prepared.capability, AppleCapability::Messages);
        assert_eq!(prepared.action, NativeActionClass::Draft);

        assert!(spec_for("remote", "read_system_calendar", &Value::Null).is_none());
        assert!(spec_for("local_filesystem", "write_file", &Value::Null).is_some());
    }

    #[tokio::test]
    async fn mapped_native_operation_fails_closed_without_an_accepted_turn() {
        let spec = spec_for(
            MACOS_APPLESCRIPT_SERVER_NAME,
            "read_system_notes",
            &serde_json::json!({}),
        );
        let root = std::env::temp_dir().join(format!(
            "oomu-native-receipt-no-turn-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let persistence = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let result = execute(spec, None, &persistence, true, async {
            panic!("native operation must not run without accepted turn evidence")
        })
        .await;
        assert_eq!(
            result.unwrap_err(),
            "native_apple_operation_requires_accepted_turn"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn completed_tool_result_carries_the_native_receipt_projection() {
        let spec = spec_for(
            MACOS_APPLESCRIPT_SERVER_NAME,
            "read_system_calendar",
            &serde_json::json!({}),
        );
        let (root, persistence, turn) = persistence("success");

        let result = execute(spec, Some(&turn), &persistence, true, async {
            Ok(McpToolCallResult {
                content: Vec::new(),
                structured_content: Some(serde_json::json!({
                    "code": "calendar_read_ok",
                    "events": []
                })),
                is_error: false,
                meta: Some(serde_json::json!({"transport": "native"})),
                raw: None,
            })
        })
        .await
        .unwrap();

        let meta = result.meta.unwrap();
        assert_eq!(meta["transport"], "native");
        let receipt = &meta["oomuNativeExecutionReceipt"];
        assert_eq!(receipt["capabilityId"], "calendar");
        assert_eq!(receipt["actionClass"], "read");
        assert_eq!(receipt["postcondition"]["operationSucceeded"], true);
        assert_eq!(receipt["postcondition"]["verified"], true);
        assert_eq!(
            receipt["executionBindingSha256"].as_str().map(str::len),
            Some(64)
        );
        let receipt_id = receipt["receiptId"].as_str().unwrap();
        let consumed = consume_chat_turn_receipt(receipt_id, &turn).unwrap();
        assert_eq!(consumed.receipt_id, receipt_id);
        assert_eq!(consumed.capability_id, "calendar");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn typed_permission_denial_carries_an_unverified_native_receipt() {
        let spec = spec_for(
            MACOS_APPLESCRIPT_SERVER_NAME,
            "read_system_calendar",
            &serde_json::json!({}),
        );
        let (root, persistence, turn) = persistence("denial");

        let result = execute(spec, Some(&turn), &persistence, true, async {
            Ok(McpToolCallResult {
                content: Vec::new(),
                structured_content: Some(serde_json::json!({
                    "code": "calendar_permission_denied"
                })),
                is_error: true,
                meta: None,
                raw: None,
            })
        })
        .await
        .unwrap();

        assert!(result.is_error);
        let meta = result.meta.unwrap();
        let receipt = &meta["oomuNativeExecutionReceipt"];
        assert_ne!(receipt["outcome"], "succeeded");
        assert_eq!(receipt["verified"], false);
        assert_eq!(receipt["postcondition"]["operationSucceeded"], false);
        assert_eq!(
            receipt["postcondition"]["nativeResultCode"],
            "calendar_permission_denied"
        );
        let receipt_id = receipt["receiptId"].as_str().unwrap();
        let consumed = consume_chat_turn_receipt(receipt_id, &turn).unwrap();
        assert!(!consumed.verified_success);
        assert_ne!(
            consumed.outcome,
            crate::tools::native_operation_receipt::NativeOperationOutcome::Succeeded
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn scheduled_mail_read_carries_workflow_bound_native_evidence() {
        let path = std::env::temp_dir().join(format!(
            "oomu-native-workflow-mail-{}-{}.db",
            std::process::id(),
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let engine = PersistenceEngine::initialize_for_integration_test(path.clone())
            .expect("isolated scheduled Mail receipt database");
        let connection = engine.open_connection().unwrap();
        connection
            .execute(
                "INSERT INTO workflow_blueprints (workflow_id,version,name,visual_state_json,is_active,created_at_ms,updated_at_ms) VALUES ('workflow-mail',1,'Mail workflow','{}',1,1,1)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO execution_instances (id,workflow_id,workflow_version,status,active_node_id,created_at_ms,updated_at_ms) VALUES ('mail-execution','workflow-mail',1,'Running','read-mail',1,1)",
                [],
            )
            .unwrap();
        drop(connection);
        let arguments = serde_json::json!({"max_messages": 20, "unread_only": true});
        let spec = spec_for(
            MACOS_APPLESCRIPT_SERVER_NAME,
            READ_SYSTEM_EMAILS_TOOL_NAME,
            &arguments,
        );

        let result = execute_for_workflow(
            spec,
            &engine,
            "mail-execution",
            MACOS_APPLESCRIPT_SERVER_NAME,
            READ_SYSTEM_EMAILS_TOOL_NAME,
            &arguments,
            false,
            async {
                Ok(McpToolCallResult {
                    content: Vec::new(),
                    structured_content: Some(serde_json::json!({
                        "code": "mail_read_ok",
                        "emails": []
                    })),
                    is_error: false,
                    meta: None,
                    raw: None,
                })
            },
        )
        .await
        .unwrap();

        let receipt = &result.meta.unwrap()["oomuNativeExecutionReceipt"];
        assert_eq!(receipt["capabilityId"], "mail");
        assert_eq!(receipt["actionClass"], "read");
        assert_eq!(receipt["postcondition"]["operationSucceeded"], true);
        let receipt_id = receipt["receiptId"].as_str().unwrap();
        let (turn_root, _, turn) = persistence("workflow-receipt-consumption");
        assert_eq!(
            consume_chat_turn_receipt(receipt_id, &turn),
            Err(crate::tools::native_operation_receipt::NativeReceiptConsumptionError::WorkflowReceipt)
        );
        let _ = std::fs::remove_dir_all(turn_root);
        let _ = std::fs::remove_file(path);
    }
}
