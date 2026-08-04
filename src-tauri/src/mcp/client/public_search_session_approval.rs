use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct PublicSearchChatSessionGrant {
    session_id: String,
    agent_id: String,
    trusted_config_binding: String,
    tool_definition_binding: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PublicSearchApprovalTurnBinding {
    turn_id: String,
    generation_token: String,
    pub(super) session_id: String,
    agent_id: String,
    provider_id: String,
    model_id: String,
    parent_turn_id: Option<String>,
    root_turn_id: String,
    turn_kind: String,
}

impl PublicSearchApprovalTurnBinding {
    pub(super) fn from_turn_context(turn_context: &ChatTurnPersistenceContext) -> Self {
        Self {
            turn_id: turn_context.turn_id.clone(),
            generation_token: turn_context.generation_token.clone(),
            session_id: turn_context.session_id.clone(),
            agent_id: turn_context.agent_id.clone(),
            provider_id: turn_context.provider_id.clone(),
            model_id: turn_context.model_id.clone(),
            parent_turn_id: turn_context.parent_turn_id.clone(),
            root_turn_id: turn_context.root_turn_id.clone(),
            turn_kind: turn_context.turn_kind.clone(),
        }
    }

    pub(super) fn matches(&self, turn_context: &ChatTurnPersistenceContext) -> bool {
        self == &Self::from_turn_context(turn_context)
    }
}

impl McpClientRegistry {
    pub(super) async fn configure_public_search_chat_session_approval(
        &self,
        server_name: &str,
        tool_name: &str,
        turn_context: Option<&ChatTurnPersistenceContext>,
        prepared: &mut PreparedMcpToolApproval,
    ) {
        if !native_public_search_execution::is_supported_tool(server_name, tool_name) {
            return;
        }
        let Some(turn_context) = turn_context else {
            return;
        };
        prepared.public_search_turn_binding = Some(
            PublicSearchApprovalTurnBinding::from_turn_context(turn_context),
        );
        prepared.request.approval_scope_kinds =
            vec!["once".to_string(), "chat_session".to_string()];
        let Some(trusted_config_binding) = prepared
            .session
            .as_ref()
            .and_then(|session| session.trusted_internal_config_binding.as_deref())
        else {
            return;
        };
        prepared.request.chat_session_approved = self
            .public_search_chat_session_grant_covers(
                turn_context,
                trusted_config_binding,
                &prepared.request.tool_definition_binding,
            )
            .await;
    }

    pub(super) async fn public_search_chat_session_grant_covers(
        &self,
        turn_context: &ChatTurnPersistenceContext,
        trusted_config_binding: &str,
        tool_definition_binding: &str,
    ) -> bool {
        let Some(grant) = public_search_chat_session_grant(
            turn_context,
            trusted_config_binding,
            tool_definition_binding,
        ) else {
            return false;
        };
        self.public_search_chat_session_grants
            .lock()
            .await
            .contains(&grant)
    }

    pub(super) async fn grant_public_search_for_chat_session(
        &self,
        turn_context: &ChatTurnPersistenceContext,
        trusted_config_binding: &str,
        tool_definition_binding: &str,
    ) -> Result<(), McpClientError> {
        let grant = public_search_chat_session_grant(
            turn_context,
            trusted_config_binding,
            tool_definition_binding,
        )
        .ok_or_else(|| {
            McpClientError::permission(
                "Public search session approval requires an exact chat, agent, service, and tool binding."
                    .to_string(),
            )
        })?;
        self.public_search_chat_session_grants
            .lock()
            .await
            .insert(grant);
        Ok(())
    }

    pub(crate) async fn revoke_public_search_chat_session_authority(
        &self,
        session_id: &str,
    ) -> usize {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return 0;
        }
        let mut revoked = 0usize;
        {
            let mut grants = self.public_search_chat_session_grants.lock().await;
            let before = grants.len();
            grants.retain(|grant| grant.session_id != session_id);
            revoked = revoked.saturating_add(before.saturating_sub(grants.len()));
        }
        {
            let mut pending = self.pending_tool_approvals.lock().await;
            let before = pending.len();
            pending.retain(|_, approval| {
                approval
                    .public_search_turn_binding
                    .as_ref()
                    .is_none_or(|binding| binding.session_id != session_id)
            });
            revoked = revoked.saturating_add(before.saturating_sub(pending.len()));
        }
        revoked
    }

    pub(super) async fn activate_prepared_tool_approval_with_postcondition(
        &self,
        prepared: PreparedMcpToolApproval,
        native_shield_approved: bool,
        postcondition: impl FnOnce() -> Result<(), McpClientError>,
    ) -> Result<McpToolApprovalRequest, McpClientError> {
        let activated = self
            .activate_prepared_tool_approval(prepared, native_shield_approved)
            .await?;
        if let Err(error) = postcondition() {
            let _ = self.reject_tool_approval(&activated.approval_token).await;
            return Err(error);
        }
        Ok(activated)
    }
}

fn public_search_chat_session_grant(
    turn_context: &ChatTurnPersistenceContext,
    trusted_config_binding: &str,
    tool_definition_binding: &str,
) -> Option<PublicSearchChatSessionGrant> {
    let session_id = turn_context.session_id.trim();
    let agent_id = turn_context.agent_id.trim();
    let trusted_config_binding = trusted_config_binding.trim();
    let tool_definition_binding = tool_definition_binding.trim();
    if session_id.is_empty()
        || agent_id.is_empty()
        || trusted_config_binding.is_empty()
        || tool_definition_binding.is_empty()
    {
        return None;
    }
    Some(PublicSearchChatSessionGrant {
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        trusted_config_binding: trusted_config_binding.to_string(),
        tool_definition_binding: tool_definition_binding.to_string(),
    })
}

impl From<McpChatTurnContext> for ChatTurnPersistenceContext {
    fn from(context: McpChatTurnContext) -> Self {
        Self {
            turn_id: context.turn_id,
            generation_token: context.generation_token,
            session_id: context.session_id,
            agent_id: context.agent_id,
            provider_id: context.provider_id,
            model_id: context.model_id,
            parent_turn_id: context.parent_turn_id,
            root_turn_id: context.root_turn_id,
            turn_kind: context.turn_kind,
        }
    }
}

pub(super) fn validate_mcp_chat_turn(
    persistence: &PersistenceEngine,
    turn_context: Option<&ChatTurnPersistenceContext>,
) -> Result<(), McpClientError> {
    let Some(turn_context) = turn_context else {
        return Ok(());
    };
    persistence
        .ensure_chat_turn_for_native_action(turn_context)
        .map_err(|error| {
            McpClientError::permission(format!(
                "MCP execution blocked because its originating chat turn is stale: {error}"
            ))
        })
}

pub(super) async fn prepare_tool_approval(
    server_name: String,
    tool_name: String,
    arguments: serde_json::Value,
    turn_context: Option<McpChatTurnContext>,
    registry: tauri::State<'_, McpClientRegistry>,
    persistence: tauri::State<'_, PersistenceEngine>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    app: tauri::AppHandle,
) -> Result<Option<McpToolApprovalRequest>, String> {
    let turn_context = turn_context.map(ChatTurnPersistenceContext::from);
    validate_mcp_chat_turn(persistence.inner(), turn_context.as_ref())
        .map_err(|error| error.message)?;
    let prepared = registry
        .prepare_tool_approval_candidate(&server_name, &tool_name, arguments)
        .await
        .map_err(|error| error.message)?;
    let Some(mut prepared) = prepared else {
        return Ok(None);
    };
    registry
        .configure_public_search_chat_session_approval(
            &server_name,
            &tool_name,
            turn_context.as_ref(),
            &mut prepared,
        )
        .await;
    if !prepared.requires_native_shield {
        return registry
            .activate_prepared_tool_approval_with_postcondition(prepared, false, || {
                validate_mcp_chat_turn(persistence.inner(), turn_context.as_ref())
            })
            .await
            .map(Some)
            .map_err(|error| error.message);
    }

    let action = remote_mcp_tool_shield_action(&prepared).map_err(|error| error.message)?;
    let shield_request = shield_gate::build_shield_approval_request(&action).ok_or_else(|| {
        "Shield Gate did not classify the remote MCP tool call as consent-required.".to_string()
    })?;
    eprintln!(
        "MCP_TOOL_SECURITY_EVENT audit_id={} server={} tool={} phase=native_shield_requested",
        prepared.request.audit_id,
        crate::redaction::redacted_log_text(&prepared.request.server_name),
        crate::redaction::redacted_log_text(&prepared.request.tool_name),
    );
    if let Err(error) =
        shield_gate::request_user_approval(&app, approvals.inner(), shield_request).await
    {
        eprintln!(
            "MCP_TOOL_SECURITY_EVENT audit_id={} server={} tool={} phase=native_shield_denied code={}",
            prepared.request.audit_id,
            crate::redaction::redacted_log_text(&prepared.request.server_name),
            crate::redaction::redacted_log_text(&prepared.request.tool_name),
            error.code,
        );
        return Err(
            "Remote MCP tool call was not approved by the native Shield boundary.".to_string(),
        );
    }
    registry
        .activate_prepared_tool_approval_with_postcondition(prepared, true, || {
            validate_mcp_chat_turn(persistence.inner(), turn_context.as_ref())
        })
        .await
        .map(Some)
        .map_err(|error| error.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn rejects_incomplete_chat_session_grant_bindings() {
        let mut turn_context = ChatTurnPersistenceContext {
            turn_id: "turn".to_string(),
            generation_token: "generation".to_string(),
            session_id: "session".to_string(),
            agent_id: "agent".to_string(),
            provider_id: "provider".to_string(),
            model_id: "model".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn".to_string(),
            turn_kind: "user".to_string(),
        };
        assert!(public_search_chat_session_grant(&turn_context, "config", "tool").is_some());
        turn_context.agent_id.clear();
        assert!(public_search_chat_session_grant(&turn_context, "config", "tool").is_none());
    }

    #[test]
    fn turn_binding_matches_only_its_exact_turn_context() {
        let context = ChatTurnPersistenceContext {
            turn_id: "turn".to_string(),
            generation_token: "generation".to_string(),
            session_id: "session".to_string(),
            agent_id: "agent".to_string(),
            provider_id: "provider".to_string(),
            model_id: "model".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn".to_string(),
            turn_kind: "user".to_string(),
        };
        let binding = PublicSearchApprovalTurnBinding::from_turn_context(&context);
        assert!(binding.matches(&context));
        let mut other = context;
        other.generation_token = "other-generation".to_string();
        assert!(!binding.matches(&other));
    }

    #[test]
    fn grant_set_keeps_exact_agent_and_session_identity() {
        let grant = PublicSearchChatSessionGrant {
            session_id: "session".to_string(),
            agent_id: "agent".to_string(),
            trusted_config_binding: "config".to_string(),
            tool_definition_binding: "tool".to_string(),
        };
        let mut grants = HashSet::new();
        assert!(grants.insert(grant.clone()));
        assert!(!grants.insert(grant));
    }
}
