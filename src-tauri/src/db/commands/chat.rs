use super::super::chat_turn_acceptance::CancelSavedChatTurnRequest;
use super::super::*;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarkChatSessionCompletionUnreadRequest {
    pub session_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionAttentionReceipt {
    pub session_id: String,
    pub unread_count: i64,
    pub banner_delivered: bool,
    pub newly_recorded: bool,
}

#[tauri::command]
pub async fn list_chat_sessions(
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<ChatSessionRecord>, AgenticLoopError> {
    let engine = persistence.inner().clone();
    let sessions = tauri::async_runtime::spawn_blocking(move || engine.select_chat_sessions())
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
    let unread_count = sessions
        .iter()
        .filter(|session| session.unread_completion)
        .count() as i64;
    crate::chat_attention::set_dock_unread_count(&app, unread_count)
        .map_err(AgenticLoopError::from_persistence)?;
    Ok(sessions)
}

#[tauri::command]
pub async fn mark_chat_session_completion_unread(
    request: MarkChatSessionCompletionUnreadRequest,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ChatSessionAttentionReceipt, AgenticLoopError> {
    let session_id = request.session_id.trim().to_string();
    let engine = persistence.inner().clone();
    let turn_id = request.turn_id.trim().to_string();
    let session_id_for_write = session_id.clone();
    let (unread_count, newly_recorded) = tauri::async_runtime::spawn_blocking(move || {
        engine.record_chat_completion_attention(&session_id_for_write, &turn_id)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
    crate::chat_attention::set_dock_unread_count(&app, unread_count)
        .map_err(AgenticLoopError::from_persistence)?;
    let banner_delivered =
        newly_recorded && crate::chat_attention::show_background_completion(&app).is_ok();
    Ok(ChatSessionAttentionReceipt {
        session_id,
        unread_count,
        banner_delivered,
        newly_recorded,
    })
}

#[tauri::command]
pub async fn mark_chat_session_read(
    session_id: String,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ChatSessionAttentionReceipt, AgenticLoopError> {
    let session_id = session_id.trim().to_string();
    let engine = persistence.inner().clone();
    let session_id_for_write = session_id.clone();
    let unread_count = tauri::async_runtime::spawn_blocking(move || {
        engine.set_chat_session_unread_completion(&session_id_for_write, false)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
    crate::chat_attention::set_dock_unread_count(&app, unread_count)
        .map_err(AgenticLoopError::from_persistence)?;
    crate::chat_attention::clear_delivered_chat_notifications();
    Ok(ChatSessionAttentionReceipt {
        session_id,
        unread_count,
        banner_delivered: false,
        newly_recorded: false,
    })
}

#[tauri::command]
pub async fn create_chat_session(
    request: CreateChatSessionRequest,
    auto_route_baseline: Option<AutoRouteSessionBaselineRequest>,
    project_id: Option<String>,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
    agent_manager: tauri::State<'_, crate::agent_manager::AgentManager>,
) -> Result<ChatSessionRecord, AgenticLoopError> {
    let model_root = auto_route_baseline
        .as_ref()
        .map(|_| crate::settings::resolved_local_model_directory(&app))
        .transpose()
        .map_err(AgenticLoopError::from_persistence)?;
    let engine = persistence.inner().clone();
    let agent_manager = agent_manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(project_id) = project_id.as_deref() {
            crate::projects::repository::validate_user_project(&engine, project_id)
                .map_err(|message| AgenticLoopError::from_persistence(message))?;
        }
        let dynamic_binding =
            request.provider_id.trim() == "dynamic" && request.model_id.trim() == "dynamic";
        let session = match (dynamic_binding, auto_route_baseline) {
            (true, Some(baseline)) => {
                let _provider_guard = agent_manager.lock_writes();
                let verified = super::auto_route::verify_baseline_locked(
                    &agent_manager,
                    &baseline,
                    model_root.as_deref().ok_or_else(|| {
                        super::auto_route::domain_error(
                            "auto_route_local_model_store_unavailable",
                            "auto_route_provider_identity",
                            "OOMU could not verify the selected on-device model.",
                        )
                    })?,
                )?;
                let session = engine
                    .ensure_chat_session_with_auto_route_baseline(
                        request,
                        verified.clone(),
                        model_root.as_deref().ok_or_else(|| {
                            super::auto_route::domain_error(
                                "auto_route_local_model_store_unavailable",
                                "auto_route_provider_identity",
                                "OOMU could not verify the selected on-device model.",
                            )
                        })?,
                    )
                    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
                let current =
                    super::super::auto_route::read_persisted_auto_route_state(&engine, &session.id)
                        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
                super::super::auto_route::emit_auto_route_receipt(&AutoRouteActivationReceipt {
                    kind: "auto_route_session_created",
                    receipt_id: format!("auto-route-{}-1", session.id),
                    session_id: session.id.clone(),
                    provider_config_id: Some(verified.provider_config_id),
                    provider_type: Some(verified.provider_type),
                    model_id: Some(verified.model_id),
                    provenance: Some(verified.provenance),
                    previous_route_generation: RouteGeneration::UNVERIFIED,
                    current_route_generation: current.route_generation,
                    previous_state_digest:
                        super::super::auto_route::missing_auto_route_state_digest(&session.id),
                    current_state_digest: current.state_digest,
                    dynamic_routing_enabled: current.dynamic_routing_enabled,
                    changed: true,
                    committed: true,
                    rolled_back: false,
                    retryable: false,
                    error_code: None,
                });
                session
            }
            (true, None) => {
                return Err(super::auto_route::domain_error(
                    "auto_route_baseline_incomplete",
                    "auto_route_provider_identity",
                    "Choose an on-device model before starting an Auto-route chat.",
                ))
            }
            (false, Some(_)) => {
                return Err(super::auto_route::domain_error(
                    "auto_route_baseline_out_of_scope",
                    "auto_route_provider_identity",
                    "A manual chat cannot save an Auto-route model baseline.",
                ))
            }
            (false, None) => engine
                .ensure_chat_session(request)
                .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?,
        };
        if let Some(project_id) = project_id {
            crate::projects::repository::bind_record(
                &engine,
                crate::projects::BindProjectRecordRequest {
                    project_id: Some(project_id),
                    record_kind: "chat_session".to_string(),
                    record_id: session.id.clone(),
                },
            )
            .map_err(AgenticLoopError::from_persistence)?;
            return engine
                .select_chat_session_by_id(&session.id)
                .map_err(|error| AgenticLoopError::from_persistence(error.to_string()));
        }
        Ok(session)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
}

#[tauri::command]
pub async fn update_chat_session_web_grounding_override(
    session_id: String,
    web_grounding_override: Option<bool>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ChatSessionRecord, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.update_chat_session_web_grounding_override(&session_id, web_grounding_override)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn rename_chat_session(
    request: RenameChatSessionRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ChatSessionRecord, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.rename_chat_session(&request.session_id, &request.title)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn delete_chat_session(
    session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<bool, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.delete_chat_session_by_id(&session_id))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn stage_chat_session_deletion(
    session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<bool, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.stage_chat_session_deletion_by_id(&session_id)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn undo_chat_session_deletion(
    session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<bool, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.undo_chat_session_deletion_by_id(&session_id)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn commit_chat_session_deletion(
    session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<bool, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.commit_chat_session_deletion_by_id(&session_id)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn list_chat_messages(
    session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<ChatMessageRecord>, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.select_chat_messages(&session_id))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn get_agent_execution_recovery_states(
    session_id: String,
    execution_ids: Vec<String>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<
    Vec<super::super::agent_execution_recovery_state::AgentExecutionRecoveryStateRecord>,
    AgenticLoopError,
> {
    let (session_id, execution_ids) =
        super::super::agent_execution_recovery_state::clean_agent_execution_recovery_state_query(
            session_id,
            execution_ids,
        )?;
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.select_agent_execution_recovery_states(&session_id, &execution_ids)
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn accept_chat_turn(
    request: AcceptChatTurnRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
    voice_capture: tauri::State<'_, crate::mac_speech::VoiceCaptureManager>,
) -> Result<AcceptedChatTurn, AgenticLoopError> {
    let engine = persistence.inner().clone();
    let voice_persistence = persistence.inner().clone();
    let receipt_request = request.clone();
    let voice_request = request.clone();
    let accepted = tauri::async_runtime::spawn_blocking(move || {
        let accepted = engine.accept_chat_turn(request);
        if let Ok(result) = accepted.as_ref() {
            let receipt = accepted_chat_turn_native_receipt(&receipt_request, result);
            crate::diagnostic_output::write_functional_acceptance_receipt(&receipt);
        }
        accepted
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?;
    if let Some(capture) = voice_capture.take_matching_capture(&voice_request.message) {
        record_voice_capture_receipts(&voice_request, capture, &voice_persistence).await;
    }
    Ok(accepted)
}

#[tauri::command]
pub async fn resume_interrupted_chat_turn(
    request: AcceptChatTurnRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<AcceptedChatTurn, AgenticLoopError> {
    let engine = persistence.inner().clone();
    let receipt_request = request.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let resumed = engine.resume_interrupted_chat_turn(request);
        if let Ok(result) = resumed.as_ref() {
            let receipt = resumed_chat_turn_native_receipt(&receipt_request, result);
            crate::diagnostic_output::write_functional_acceptance_receipt(&receipt);
        }
        resumed
    })
    .await
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
    .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn cancel_permission_recovery_turn(
    request: super::super::permission_turn_continuation::CancelPermissionTurnRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<super::super::permission_turn_continuation::CancelPermissionTurnResult, AgenticLoopError>
{
    persistence
        .cancel_permission_turn(request)
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

fn accepted_chat_turn_native_receipt(
    request: &AcceptChatTurnRequest,
    result: &AcceptedChatTurn,
) -> Value {
    serde_json::json!({
        "kind": "accepted_chat_turn", "sessionId": request.session_id,
        "turnId": request.turn_id, "generationToken": request.generation_token,
        "rootTurnId": request.root_turn_id, "turnKind": request.turn_kind,
        "providerId": request.provider_id, "modelId": request.model_id,
        "messageSha256": crate::foundation::digest::sha256_hex(request.message.as_bytes()),
        "messageId": result.message_id,
        "sessionWasEmptyBeforeAcceptance": result.session_was_empty_before_acceptance,
    })
}

async fn record_voice_capture_receipts(
    request: &AcceptChatTurnRequest,
    capture: crate::mac_speech::PendingVoiceCapture,
    persistence: &PersistenceEngine,
) {
    let context = request.persistence_context();
    let transcript_digest = crate::foundation::digest::sha256_hex(capture.transcript.as_bytes());
    let binding = format!(
        "voice:{}",
        crate::foundation::digest::sha256_hex(
            format!(
                "{}:{}:{}:{}:{}",
                capture.capture_id,
                transcript_digest,
                request.session_id,
                request.turn_id,
                crate::foundation::digest::sha256_hex(request.message.as_bytes())
            )
            .as_bytes(),
        )
    );
    let count = capture.transcript.chars().count() as u64;
    let durable_binding = format!("voice-turn:{binding}");
    let microphone = crate::tools::native_operation_receipt::NativePostconditionEvidence {
        evidence_kind: "voice_audio_capture",
        operation_succeeded: true,
        verified: true,
        bounded_count: Some(count),
        truncated: Some(false),
        native_result_code: Some(if capture.final_seen {
            "final_transcript_bound".to_string()
        } else {
            "partial_transcript_bound".to_string()
        }),
        durable_operation_binding: Some(format!("{durable_binding}:microphone")),
        capture_proof: None,
    };
    finish_voice_receipt(
        crate::tools::native_operation_receipt::AppleCapability::Microphone,
        binding.clone(),
        &context,
        persistence,
        microphone,
    )
    .await;
    let speech = crate::tools::native_operation_receipt::NativePostconditionEvidence {
        evidence_kind: "voice_transcript_bound",
        operation_succeeded: true,
        verified: true,
        bounded_count: Some(count),
        truncated: Some(false),
        native_result_code: Some("accepted_turn_contains_transcript".to_string()),
        durable_operation_binding: Some(format!("{durable_binding}:speech")),
        capture_proof: None,
    };
    finish_voice_receipt(
        crate::tools::native_operation_receipt::AppleCapability::SpeechRecognition,
        binding,
        &context,
        persistence,
        speech,
    )
    .await;
}

async fn finish_voice_receipt(
    capability: crate::tools::native_operation_receipt::AppleCapability,
    binding: String,
    context: &ChatTurnPersistenceContext,
    persistence: &PersistenceEngine,
    evidence: crate::tools::native_operation_receipt::NativePostconditionEvidence,
) {
    if let Some(attempt) =
        crate::tools::native_operation_receipt::NativeOperationAttempt::begin_with_persistence(
            capability,
            crate::tools::native_operation_receipt::NativeActionClass::Observe,
            true,
            binding,
            Some(context),
            persistence,
        )
        .await
    {
        let _ = attempt.finish(evidence).await;
    }
}

fn resumed_chat_turn_native_receipt(
    request: &AcceptChatTurnRequest,
    result: &AcceptedChatTurn,
) -> Value {
    serde_json::json!({
        "kind": "resumed_interrupted_chat_turn", "sessionId": request.session_id,
        "turnId": request.turn_id, "generationToken": request.generation_token,
        "rootTurnId": request.root_turn_id, "turnKind": request.turn_kind,
        "providerId": request.provider_id, "modelId": request.model_id,
        "messageSha256": crate::foundation::digest::sha256_hex(request.message.as_bytes()),
        "messageId": result.message_id, "fromTurnState": "interrupted",
        "toTurnState": "accepted", "reusedMessage": true, "responseClaimed": false,
    })
}

#[cfg(test)]
mod accepted_chat_turn_native_receipt_tests {
    use super::*;

    #[test]
    fn receipt_exposes_atomic_fresh_session_and_requested_route_identity() {
        let request = AcceptChatTurnRequest {
            turn_id: "turn-294".to_string(),
            generation_token: "generation-294".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-294".to_string(),
            turn_kind: "root".to_string(),
            session_id: "session-294".to_string(),
            agent_id: "agent-294".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            message: "Search separately".to_string(),
        };
        let receipt = accepted_chat_turn_native_receipt(
            &request,
            &AcceptedChatTurn {
                turn_id: request.turn_id.clone(),
                message_id: 1,
                accepted: true,
                session_was_empty_before_acceptance: true,
            },
        );

        assert_eq!(receipt["providerId"], "dynamic");
        assert_eq!(receipt["modelId"], "dynamic");
        assert_eq!(receipt["sessionWasEmptyBeforeAcceptance"], true);
        assert_eq!(receipt["generationToken"], "generation-294");
        assert_eq!(receipt["messageId"], 1);
        assert_eq!(
            receipt["messageSha256"],
            crate::foundation::digest::sha256_hex(request.message.as_bytes())
        );
    }

    #[test]
    fn resume_receipt_proves_exact_identity_and_reused_message() {
        let request = AcceptChatTurnRequest {
            turn_id: "turn-301".to_string(),
            generation_token: "generation-301".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-301".to_string(),
            turn_kind: "root".to_string(),
            session_id: "session-301".to_string(),
            agent_id: "agent-301".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            message: "Resume this exact turn.".to_string(),
        };
        let receipt = resumed_chat_turn_native_receipt(
            &request,
            &AcceptedChatTurn {
                turn_id: request.turn_id.clone(),
                message_id: 301,
                accepted: true,
                session_was_empty_before_acceptance: false,
            },
        );

        assert_eq!(receipt["kind"], "resumed_interrupted_chat_turn");
        assert_eq!(receipt["generationToken"], "generation-301");
        assert_eq!(receipt["messageId"], 301);
        assert_eq!(receipt["fromTurnState"], "interrupted");
        assert_eq!(receipt["toTurnState"], "accepted");
        assert_eq!(receipt["reusedMessage"], true);
        assert_eq!(receipt["responseClaimed"], false);
    }

    #[test]
    fn acceptance_and_resume_receipts_bind_the_same_message_row() {
        let request = AcceptChatTurnRequest {
            turn_id: "turn-bound".to_string(),
            generation_token: "generation-bound".to_string(),
            parent_turn_id: None,
            root_turn_id: "turn-bound".to_string(),
            turn_kind: "root".to_string(),
            session_id: "session-bound".to_string(),
            agent_id: "agent-bound".to_string(),
            provider_id: "dynamic".to_string(),
            model_id: "dynamic".to_string(),
            message: "Keep this exact row.".to_string(),
        };
        let result = AcceptedChatTurn {
            turn_id: request.turn_id.clone(),
            message_id: 901,
            accepted: true,
            session_was_empty_before_acceptance: true,
        };
        let accepted = accepted_chat_turn_native_receipt(&request, &result);
        let resumed = resumed_chat_turn_native_receipt(&request, &result);

        assert_eq!(accepted["messageId"], resumed["messageId"]);
        assert_eq!(accepted["messageSha256"], resumed["messageSha256"]);
        assert_eq!(accepted["turnId"], resumed["turnId"]);
        assert_eq!(accepted["generationToken"], resumed["generationToken"]);
    }
}

#[tauri::command]
pub async fn finalize_accepted_chat_turn(
    request: FinalizeAcceptedChatTurnRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<i64, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.finalize_accepted_chat_turn(request))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn abandon_accepted_chat_turn(
    request: AbandonAcceptedChatTurnRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Option<i64>, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.abandon_accepted_chat_turn(request))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn cancel_saved_chat_turn(
    request: CancelSavedChatTurnRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<i64, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.cancel_saved_chat_turn(request))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn get_sovereign_ledger_stats(
    since_ms: Option<i64>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SovereignLedgerStats, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.sovereign_ledger_stats(since_ms))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn reset_sovereign_ledger_stats(
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<(), AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.reset_sovereign_ledger_stats())
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn get_session_context_status(
    session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ContextHorizonStatus, AgenticLoopError> {
    let session_id = clean_session_config_id(session_id)?;
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.session_context_status(&session_id))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn execute_semantic_compaction(
    session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SemanticCompactionResponse, AgenticLoopError> {
    let session_id = clean_session_config_id(session_id)?;
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.compact_session_messages(&session_id))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn save_session_context_policy(
    mut request: SaveSessionContextPolicyRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SessionContextPolicyRecord, AgenticLoopError> {
    request.session_id = clean_session_config_id(request.session_id)?;
    if !(50..=90).contains(&request.auto_compaction_threshold_percent) {
        return Err(AgenticLoopError::from_persistence(
            "Choose an automatic compaction point from 50% through 90%.".to_string(),
        ));
    }
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.save_session_context_policy(&request))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn compact_chat_session(
    mut request: CompactChatSessionRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<ContextCompactionResult, AgenticLoopError> {
    request.session_id = clean_session_config_id(request.session_id)?;
    if request
        .target_percent
        .is_some_and(|value| !(50..=90).contains(&value))
    {
        return Err(AgenticLoopError::from_persistence(
            "Choose a compaction target from 50% through 90%.".to_string(),
        ));
    }
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.compact_chat_session(&request))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn queue_message(
    request: QueueMessageRequest,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<QueuedMessageRecord, AgenticLoopError> {
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.insert_queued_message(request))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}

#[tauri::command]
pub async fn get_queued_messages(
    session_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<Vec<QueuedMessageRecord>, AgenticLoopError> {
    let session_id = clean_session_config_id(session_id)?;
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.select_queued_messages(&session_id))
        .await
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))?
        .map_err(|error| AgenticLoopError::from_persistence(error.to_string()))
}
