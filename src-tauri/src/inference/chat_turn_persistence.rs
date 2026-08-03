use super::{
    approved_file_receipts::project_verified_native_execution_receipt, public_grounding_provenance,
    ChatAttachment, InferenceError, DYNAMIC_ROUTE_ID,
};
use crate::db::{ChatTurnPersistenceContext, CompleteClaimedChatTurnRequest, PersistenceEngine};
use serde_json::{Map, Value};

pub(super) fn project_assistant_turn_metadata(
    metadata: &mut Map<String, Value>,
    context: &ChatTurnPersistenceContext,
    validation_retries: usize,
    attachments: &[ChatAttachment],
    verified_native_execution_receipt: bool,
    secure_memory_available: bool,
    context_condensation: Option<(usize, bool)>,
) {
    if validation_retries > 0 {
        metadata.insert(
            "dataValidationRetries".to_string(),
            Value::from(validation_retries as u64),
        );
    }
    metadata.insert("turnId".to_string(), Value::String(context.turn_id.clone()));
    metadata.insert(
        "generationToken".to_string(),
        Value::String(context.generation_token.clone()),
    );
    metadata.insert(
        "sessionId".to_string(),
        Value::String(context.session_id.clone()),
    );
    metadata.insert(
        "agentId".to_string(),
        Value::String(context.agent_id.clone()),
    );
    metadata.insert(
        "rootTurnId".to_string(),
        Value::String(context.root_turn_id.clone()),
    );
    metadata.insert(
        "parentTurnId".to_string(),
        context
            .parent_turn_id
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    metadata.insert(
        "turnKind".to_string(),
        Value::String(context.turn_kind.clone()),
    );
    public_grounding_provenance::project_metadata(attachments, metadata);
    project_verified_native_execution_receipt(metadata, verified_native_execution_receipt);
    if !secure_memory_available {
        metadata.insert(
            "secureMemoryStatus".to_string(),
            Value::String("unavailable".to_string()),
        );
    }
    if let Some((budget_tokens, sources_preserved)) = context_condensation {
        metadata.insert("contextCondensed".to_string(), Value::Bool(true));
        metadata.insert(
            "contextBudgetTokens".to_string(),
            Value::from(budget_tokens as u64),
        );
        metadata.insert(
            "contextSourcesPreserved".to_string(),
            Value::Bool(sources_preserved),
        );
    }
}

pub(super) struct ChatTurnPersistenceGuard {
    persistence: PersistenceEngine,
    context: ChatTurnPersistenceContext,
    session_provider_id: String,
    session_model_id: String,
    terminal: bool,
}

pub(super) struct PreClaimAcceptedTurnGuard {
    persistence: PersistenceEngine,
    context: ChatTurnPersistenceContext,
    armed: bool,
}

impl PreClaimAcceptedTurnGuard {
    pub(super) fn new(persistence: PersistenceEngine, context: ChatTurnPersistenceContext) -> Self {
        Self {
            persistence,
            context,
            armed: true,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PreClaimAcceptedTurnGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        match self.persistence.interrupt_accepted_chat_turn(&self.context) {
            Ok(Some(_)) => {}
            Ok(None) => crate::diagnostic_output::write_diagnostic_line(format_args!(
                "AUTO_ROUTE_ACCEPTED_TURN_INTERRUPT_SKIPPED turn={}",
                crate::foundation::digest::sha256_hex(self.context.turn_id.as_bytes())
            )),
            Err(error) => crate::diagnostic_output::write_diagnostic_line(format_args!(
                "AUTO_ROUTE_ACCEPTED_TURN_INTERRUPT_FAILED turn={} error={}",
                crate::foundation::digest::sha256_hex(self.context.turn_id.as_bytes()),
                crate::redaction::redacted_log_text(&error.to_string())
            )),
        }
    }
}

impl ChatTurnPersistenceGuard {
    pub(super) fn new(
        persistence: PersistenceEngine,
        context: ChatTurnPersistenceContext,
        preserve_dynamic_session_binding: bool,
    ) -> Self {
        let session_provider_id = if preserve_dynamic_session_binding {
            DYNAMIC_ROUTE_ID.to_string()
        } else {
            context.provider_id.clone()
        };
        let session_model_id = if preserve_dynamic_session_binding {
            DYNAMIC_ROUTE_ID.to_string()
        } else {
            context.model_id.clone()
        };
        Self {
            persistence,
            context,
            session_provider_id,
            session_model_id,
            terminal: false,
        }
    }

    pub(super) fn finish(&mut self, status: &str) -> rusqlite::Result<()> {
        self.finish_with_error(status, None)
    }

    pub(super) fn finish_inference_error(
        &mut self,
        error: &InferenceError,
    ) -> rusqlite::Result<()> {
        if error.code == "private_egress_confirmation_required" {
            if !self
                .persistence
                .release_chat_turn_response_claim(&self.context)?
            {
                return Err(rusqlite::Error::InvalidParameterName(
                    "private egress consent could not release the exact response claim".to_string(),
                ));
            }
            self.terminal = true;
            return Ok(());
        }
        if error.code == "local_inference_cancelled"
            && self
                .persistence
                .interrupt_accepted_chat_turn(&self.context)?
                .is_some()
        {
            self.terminal = true;
            return Ok(());
        }
        let status = if error.code == "local_inference_cancelled" {
            "cancelled"
        } else {
            "failed"
        };
        self.finish_with_error(status, Some(error))
    }

    fn finish_with_error(
        &mut self,
        status: &str,
        error: Option<&InferenceError>,
    ) -> rusqlite::Result<()> {
        let content = if status == "cancelled" {
            "Generation stopped."
        } else {
            "OOMU couldn't finish this reply. Nothing was changed. Try again."
        };
        let mut metadata = serde_json::json!({
            "turnId": self.context.turn_id.as_str(),
            "generationToken": self.context.generation_token.as_str(),
        });
        if let (Some(error), Some(metadata)) = (error, metadata.as_object_mut()) {
            metadata.insert(
                "terminalErrorCode".to_string(),
                Value::String(error.code.clone()),
            );
            metadata.insert(
                "terminalErrorBoundary".to_string(),
                Value::String(error.boundary.clone()),
            );
        }
        self.persistence
            .complete_claimed_chat_turn(CompleteClaimedChatTurnRequest {
                context: self.context.clone(),
                role: "system".to_string(),
                content: content.to_string(),
                message_provider_id: self.context.provider_id.clone(),
                message_model_id: self.context.model_id.clone(),
                metadata,
                session_title: None,
                session_provider_id: self.session_provider_id.clone(),
                session_model_id: self.session_model_id.clone(),
                status: status.to_string(),
            })?;
        self.terminal = true;
        Ok(())
    }

    pub(super) fn mark_terminal(&mut self) {
        self.terminal = true;
    }
}

impl Drop for ChatTurnPersistenceGuard {
    fn drop(&mut self) {
        if !self.terminal {
            let _ = self.finish("failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{
        AcceptChatTurnRequest, CreateChatSessionRequest, FinalizeAcceptedChatTurnRequest,
    };

    fn accepted_request(
        session: &crate::db::ChatSessionRecord,
        boundary: &str,
    ) -> AcceptChatTurnRequest {
        AcceptChatTurnRequest {
            turn_id: format!("turn-preclaim-{boundary}"),
            generation_token: format!("generation-preclaim-{boundary}"),
            parent_turn_id: None,
            root_turn_id: format!("turn-preclaim-{boundary}"),
            turn_kind: "root".to_string(),
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            provider_id: DYNAMIC_ROUTE_ID.to_string(),
            model_id: DYNAMIC_ROUTE_ID.to_string(),
            message: format!("Preserve the {boundary} turn exactly once."),
        }
    }

    #[test]
    fn every_post_accept_preclaim_failure_is_interrupted_and_resumes_once_after_restart() {
        for boundary in [
            "policy_freeze",
            "route_resolution",
            "provider_audit",
            "project_policy",
            "response_claim",
        ] {
            let root = std::env::temp_dir().join(format!(
                "oomu-preclaim-{boundary}-{}-{}",
                std::process::id(),
                crate::foundation::clock::unix_time_ns_u128()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let path = root.join("state.sqlite");
            let engine = PersistenceEngine::initialize_at(path.clone()).unwrap();
            let session = engine
                .ensure_chat_session(CreateChatSessionRequest {
                    agent_id: format!("agent-{boundary}"),
                    provider_id: DYNAMIC_ROUTE_ID.to_string(),
                    model_id: DYNAMIC_ROUTE_ID.to_string(),
                    title: Some(format!("Preclaim {boundary}")),
                    dynamic_routing_override: Some(true),
                    workspace_id: None,
                })
                .unwrap();
            let request = accepted_request(&session, boundary);
            engine.accept_chat_turn(request.clone()).unwrap();
            drop(PreClaimAcceptedTurnGuard::new(
                engine.clone(),
                request.persistence_context(),
            ));
            drop(engine);

            let restarted = PersistenceEngine::initialize_at(path).unwrap();
            let state: (String, Option<i64>, Option<i64>, String) = restarted
                .open_connection()
                .unwrap()
                .query_row(
                    "SELECT turns.status,turns.completed_at_ms,turns.response_claimed_at_ms,
                            json_extract(message.metadata_json,'$.turnState')
                     FROM chat_turns turns JOIN chat_messages message
                       ON message.session_id=turns.session_id AND message.role='user'
                      AND json_extract(message.metadata_json,'$.turnId')=turns.turn_id
                     WHERE turns.turn_id=?1",
                    rusqlite::params![request.turn_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap();
            assert_eq!(state.0, "failed", "{boundary}");
            assert!(state.1.is_some(), "{boundary}");
            assert!(state.2.is_none(), "{boundary}");
            assert_eq!(state.3, "interrupted", "{boundary}");

            restarted
                .resume_interrupted_chat_turn(request.clone())
                .unwrap();
            assert!(restarted
                .resume_interrupted_chat_turn(request.clone())
                .is_err());
            let context = request.persistence_context();
            let mut guard = PreClaimAcceptedTurnGuard::new(restarted.clone(), context.clone());
            restarted
                .begin_or_claim_chat_turn_response(&context)
                .unwrap();
            guard.disarm();
            let completion = CompleteClaimedChatTurnRequest {
                context,
                role: "system".to_string(),
                content: "The retried turn completed once.".to_string(),
                message_provider_id: DYNAMIC_ROUTE_ID.to_string(),
                message_model_id: DYNAMIC_ROUTE_ID.to_string(),
                metadata: serde_json::json!({}),
                session_title: None,
                session_provider_id: DYNAMIC_ROUTE_ID.to_string(),
                session_model_id: DYNAMIC_ROUTE_ID.to_string(),
                status: "completed".to_string(),
            };
            let first = restarted
                .complete_claimed_chat_turn(completion.clone())
                .unwrap();
            let repeated = restarted.complete_claimed_chat_turn(completion).unwrap();
            assert_eq!(first, repeated, "{boundary}");
            let terminal_count: i64 = restarted
                .open_connection()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM chat_messages
                     WHERE session_id=?1
                       AND json_extract(metadata_json,'$.terminalResultForTurnId')=?2",
                    rusqlite::params![session.id, request.turn_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(terminal_count, 1, "{boundary}");
            drop(restarted);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn sprint_304_private_egress_consent_releases_claim_for_retry_or_cancel() {
        let root = std::env::temp_dir().join(format!(
            "oomu-private-egress-retry-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ns_u128()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let session = engine
            .ensure_chat_session(CreateChatSessionRequest {
                agent_id: "agent-private-egress".to_string(),
                provider_id: DYNAMIC_ROUTE_ID.to_string(),
                model_id: DYNAMIC_ROUTE_ID.to_string(),
                title: Some("Private egress retry".to_string()),
                dynamic_routing_override: Some(true),
                workspace_id: None,
            })
            .unwrap();
        let request = accepted_request(&session, "private-egress");
        engine.accept_chat_turn(request.clone()).unwrap();
        let mut claimed = request.persistence_context();
        claimed.provider_id = "google-ai-studio".to_string();
        claimed.model_id = "gemini-private-egress".to_string();
        let consent_error = InferenceError {
            code: "private_egress_confirmation_required".to_string(),
            boundary: "PrivateEgressBoundary".to_string(),
            message: "Choose whether to send this private information.".to_string(),
        };

        engine.begin_or_claim_chat_turn_response(&claimed).unwrap();
        let mut first_attempt =
            ChatTurnPersistenceGuard::new(engine.clone(), claimed.clone(), true);
        first_attempt
            .finish_inference_error(&consent_error)
            .unwrap();
        drop(first_attempt);

        let first_state: (String, Option<i64>) = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT status, response_claimed_at_ms FROM chat_turns WHERE turn_id=?1",
                rusqlite::params![claimed.turn_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(first_state, ("running".to_string(), None));

        engine.begin_or_claim_chat_turn_response(&claimed).unwrap();
        let mut second_attempt =
            ChatTurnPersistenceGuard::new(engine.clone(), claimed.clone(), true);
        second_attempt
            .finish_inference_error(&consent_error)
            .unwrap();
        drop(second_attempt);

        let terminal_id = engine
            .finalize_accepted_chat_turn(FinalizeAcceptedChatTurnRequest {
                turn_id: request.turn_id.clone(),
                generation_token: request.generation_token.clone(),
                parent_turn_id: request.parent_turn_id.clone(),
                root_turn_id: request.root_turn_id.clone(),
                turn_kind: request.turn_kind.clone(),
                session_id: request.session_id.clone(),
                agent_id: request.agent_id.clone(),
                provider_id: request.provider_id.clone(),
                model_id: request.model_id.clone(),
                role: "system".to_string(),
                content: "Your private information stayed on this Mac.".to_string(),
                status: "cancelled".to_string(),
            })
            .unwrap();
        assert!(terminal_id > 0);
        let final_state: (String, Option<i64>) = engine
            .open_connection()
            .unwrap()
            .query_row(
                "SELECT status, response_claimed_at_ms FROM chat_turns WHERE turn_id=?1",
                rusqlite::params![request.turn_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(final_state, ("cancelled".to_string(), None));
        let messages = engine.select_chat_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[1].content,
            "Your private information stayed on this Mac."
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
