use super::*;
use crate::native_app_ports::{
    LocalApplicationMailPort, LocalMailFuture, LocalMailReceipt, MailDraftPostconditionRequest,
    MailDraftRequest, MailSendRequest,
};
use tauri::Manager;

const DRAFT_SYSTEM_EMAIL_TOOL_NAME: &str = "draft_system_email";
const SEND_SYSTEM_EMAIL_TOOL_NAME: &str = "send_system_email";
pub(super) const DEFAULT_SYSTEM_EMAIL_READ_LIMIT: u32 = 20;
pub(super) const MAX_SYSTEM_EMAIL_READ_LIMIT: u32 = 20;

pub(super) fn is_system_mail_read_tool(server_name: &str, tool_name: &str) -> bool {
    server_name
        .trim()
        .eq_ignore_ascii_case(MACOS_APPLESCRIPT_SERVER_NAME)
        && tool_name
            .trim()
            .eq_ignore_ascii_case(READ_SYSTEM_EMAILS_TOOL_NAME)
}

pub(super) fn bounded_system_email_limit(max_messages: Option<u32>) -> u32 {
    max_messages
        .unwrap_or(DEFAULT_SYSTEM_EMAIL_READ_LIMIT)
        .clamp(1, MAX_SYSTEM_EMAIL_READ_LIMIT)
}

pub(super) fn accepted_user_prompt_for_turn(
    persistence: &PersistenceEngine,
    turn_context: Option<&ChatTurnPersistenceContext>,
) -> Result<String, McpClientError> {
    let turn_context = turn_context.ok_or_else(|| {
        McpClientError::permission(
            "Mail access requires the immutable context of an accepted chat turn.".to_string(),
        )
    })?;
    validate_mcp_chat_turn(persistence, Some(turn_context))?;
    let messages = persistence
        .select_chat_messages(&turn_context.session_id)
        .map_err(|error| {
            McpClientError::permission(format!(
                "Mail access could not verify its accepted user request: {error}"
            ))
        })?;
    messages
        .into_iter()
        .find(|message| {
            message.role.eq_ignore_ascii_case("user")
                && message.metadata_json.as_deref().is_some_and(|metadata| {
                    serde_json::from_str::<Value>(metadata)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("turnId")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .as_deref()
                        == Some(turn_context.turn_id.as_str())
                })
        })
        .map(|message| message.content)
        .ok_or_else(|| {
            McpClientError::permission(
                "Mail access requires a durable user request bound to this exact chat turn."
                    .to_string(),
            )
        })
}

pub(super) fn bounded_mail_read_arguments_for_prompt(
    prompt: &str,
    arguments: &Value,
) -> Result<Value, McpClientError> {
    validate_tool_arguments(arguments)?;
    let object = arguments.as_object().ok_or_else(|| {
        McpClientError::protocol("Mail read arguments must be a JSON object.".to_string())
    })?;
    let requested_max = object
        .get("max_messages")
        .or_else(|| object.get("maxMessages"))
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let requested_unread = object
        .get("unread_only")
        .or_else(|| object.get("unreadOnly"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let normalized = prompt.trim().to_ascii_lowercase();
    let tokens = normalized
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let explicitly_unread = tokens.contains(&"unread");
    let today_scope = [
        "today",
        "earlier today",
        "from today",
        "this morning",
        "this afternoon",
        "this evening",
        "tonight",
    ]
    .iter()
    .filter_map(|marker| normalized.find(marker).map(|index| (index, marker.len())))
    .min_by_key(|(index, _)| *index);
    let unread_or_today = normalized.find("unread").zip(today_scope).is_some_and(
        |(unread_index, (today_index, today_marker_len))| {
            let (start, end) = if unread_index < today_index {
                (unread_index + "unread".len(), today_index)
            } else {
                (today_index + today_marker_len, unread_index)
            };
            normalized[start..end]
                .split(|character: char| !character.is_alphanumeric())
                .any(|token| token == "or")
        },
    );
    Ok(serde_json::json!({
        "max_messages": bounded_system_email_limit(requested_max),
        "unread_only": requested_unread || (explicitly_unread && !unread_or_today),
    }))
}

pub(super) fn validate_turn_bound_mail_read_prompt(prompt: &str) -> Result<(), String> {
    let normalized = prompt.trim().to_ascii_lowercase();
    if crate::local_app_intent::private_app_data_kind(prompt) == Some("mail")
        && crate::local_app_intent::is_focused_local_app_shortcut_request(prompt, "mail")
        && crate::agentic_loop::is_direct_private_app_read_objective(prompt, &normalized)
    {
        return Ok(());
    }
    Err(
        "Mail access is limited to one focused, read-only request in the accepted chat turn."
            .to_string(),
    )
}

pub(super) async fn execute_turn_bound_mail_read(
    arguments: Value,
    turn_context: Option<&ChatTurnPersistenceContext>,
    registry: &McpClientRegistry,
    persistence: &PersistenceEngine,
    app: &tauri::AppHandle,
) -> Result<McpToolCallResult, String> {
    let prompt =
        accepted_user_prompt_for_turn(persistence, turn_context).map_err(|error| error.message)?;
    validate_turn_bound_mail_read_prompt(&prompt)?;
    let arguments = bounded_mail_read_arguments_for_prompt(&prompt, &arguments)
        .map_err(|error| error.message)?;
    ensure_trusted_builtin_mcp_server(registry, app, MACOS_APPLESCRIPT_SERVER_NAME).await?;
    registry
        .get_tool_details(MACOS_APPLESCRIPT_SERVER_NAME, READ_SYSTEM_EMAILS_TOOL_NAME)
        .await
        .map_err(|error| error.message)?;
    validate_mcp_chat_turn(persistence, turn_context).map_err(|error| error.message)?;
    registry
        .execute_tool(
            MACOS_APPLESCRIPT_SERVER_NAME,
            READ_SYSTEM_EMAILS_TOOL_NAME,
            arguments,
        )
        .await
        .map_err(|error| error.message)
}

impl McpClientRegistry {
    /// Executes a trusted Mail mutation from the fixed draft/send allowlist
    /// after its enclosing registered Task has crossed native Shield. This is
    /// deliberately narrower than MCP approval: it cannot name another server
    /// or tool, reach a remote transport, or reuse renderer-supplied authority.
    async fn execute_trusted_mail_mutation_after_native_shield(
        &self,
        tool_name: &'static str,
        arguments: Value,
    ) -> Result<McpToolCallResult, McpClientError> {
        validate_tool_arguments(&arguments)?;
        let server_name = MACOS_APPLESCRIPT_SERVER_NAME;
        if !matches!(
            tool_name,
            DRAFT_SYSTEM_EMAIL_TOOL_NAME | SEND_SYSTEM_EMAIL_TOOL_NAME
        ) {
            return Err(McpClientError::permission(
                "The Shield-approved Mail bridge refused an unregistered mutation.".to_string(),
            ));
        }
        let session = self.session(server_name).await?;
        if !matches!(&session.transport, McpTransportConfig::Stdio) {
            return Err(McpClientError::permission(
                "The Shield-approved Mail bridge refused a non-local MCP transport.".to_string(),
            ));
        }

        let trusted_config = self
            .trusted_builtin_configs
            .lock()
            .await
            .get(server_name)
            .cloned()
            .ok_or_else(|| {
                McpClientError::permission(
                    "The Shield-approved Mail bridge is not a trusted built-in server.".to_string(),
                )
            })?;
        let registered_config = self
            .configs
            .lock()
            .await
            .get(server_name)
            .cloned()
            .ok_or_else(|| {
                McpClientError::permission(
                    "The Shield-approved Mail bridge has no registered built-in binding."
                        .to_string(),
                )
            })?;
        if !matches!(&trusted_config.transport, McpTransportConfig::Stdio)
            || mcp_config_binding(&trusted_config) != mcp_config_binding(&registered_config)
            || !session.has_trusted_internal_activation_for(&trusted_config)
        {
            return Err(McpClientError::permission(
                "The Shield-approved Mail bridge failed its trusted built-in binding check."
                    .to_string(),
            ));
        }

        let (tool, remote_authority) = self
            .tool_and_remote_authority_for_session(&session, tool_name)
            .await?;
        if remote_authority.is_some() {
            return Err(McpClientError::permission(
                "The Shield-approved Mail bridge cannot execute a remote MCP tool.".to_string(),
            ));
        }
        let verified = VerifiedMcpToolExecution {
            session,
            tool_definition_binding: tool_definition_binding(&tool),
            remote_authority: None,
            audit_id: None,
            approval_scope_kinds: vec!["once".to_string()],
            chat_session_approved: false,
            public_search_turn_binding: None,
        };
        self.execute_tool_on_verified_session(server_name, tool_name, arguments, &verified, &|| {
            Ok(())
        })
        .await
    }

    pub(super) async fn execute_trusted_mail_draft_after_native_shield(
        &self,
        arguments: Value,
    ) -> Result<McpToolCallResult, McpClientError> {
        self.execute_trusted_mail_mutation_after_native_shield(
            DRAFT_SYSTEM_EMAIL_TOOL_NAME,
            arguments,
        )
        .await
    }

    pub(super) async fn execute_trusted_mail_send_after_native_shield(
        &self,
        arguments: Value,
    ) -> Result<McpToolCallResult, McpClientError> {
        self.execute_trusted_mail_mutation_after_native_shield(
            SEND_SYSTEM_EMAIL_TOOL_NAME,
            arguments,
        )
        .await
    }
}

fn local_mail_receipt(result: McpToolCallResult) -> LocalMailReceipt {
    LocalMailReceipt {
        is_error: result.is_error,
        structured_content: result.structured_content,
    }
}

fn insert_mail_bridge_flag(arguments: &mut Value, flag: &'static str) -> Result<(), String> {
    let object = arguments
        .as_object_mut()
        .ok_or_else(|| "The Mail bridge requires validated object arguments.".to_string())?;
    object.insert(flag.to_string(), Value::Bool(true));
    Ok(())
}

/// Concrete adapter for the neutral local-application Mail port. The caller
/// can select only a typed Mail operation; this adapter alone selects the
/// trusted built-in MCP server and enforces its local transport binding.
impl LocalApplicationMailPort for tauri::AppHandle {
    fn create_mail_draft<'a>(&'a self, request: MailDraftRequest) -> LocalMailFuture<'a> {
        Box::pin(async move {
            let mut arguments =
                serde_json::to_value(request.content).map_err(|error| error.to_string())?;
            if request.reuse_existing_matching {
                insert_mail_bridge_flag(&mut arguments, "reuse_existing_matching")?;
            }
            let registry = self.state::<McpClientRegistry>();
            ensure_trusted_builtin_mcp_server(
                registry.inner(),
                self,
                MACOS_APPLESCRIPT_SERVER_NAME,
            )
            .await?;
            registry
                .execute_trusted_mail_draft_after_native_shield(arguments)
                .await
                .map(local_mail_receipt)
                .map_err(|error| error.message)
        })
    }

    fn verify_mail_draft<'a>(
        &'a self,
        request: MailDraftPostconditionRequest,
    ) -> LocalMailFuture<'a> {
        Box::pin(async move {
            let mut arguments =
                serde_json::to_value(request.content).map_err(|error| error.to_string())?;
            insert_mail_bridge_flag(&mut arguments, "verify_existing_only")?;
            let registry = self.state::<McpClientRegistry>();
            ensure_trusted_builtin_mcp_server(
                registry.inner(),
                self,
                MACOS_APPLESCRIPT_SERVER_NAME,
            )
            .await?;
            registry
                .execute_trusted_mail_draft_after_native_shield(arguments)
                .await
                .map(local_mail_receipt)
                .map_err(|error| error.message)
        })
    }

    fn send_mail<'a>(&'a self, request: MailSendRequest) -> LocalMailFuture<'a> {
        Box::pin(async move {
            let arguments = serde_json::to_value(request).map_err(|error| error.to_string())?;
            let registry = self.state::<McpClientRegistry>();
            ensure_trusted_builtin_mcp_server(
                registry.inner(),
                self,
                MACOS_APPLESCRIPT_SERVER_NAME,
            )
            .await?;
            registry
                .execute_trusted_mail_send_after_native_shield(arguments)
                .await
                .map(local_mail_receipt)
                .map_err(|error| error.message)
        })
    }
}
