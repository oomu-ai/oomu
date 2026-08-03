use super::{verified_sources, SovereignSearchExecutionRequest, SovereignSearchResponse};
use crate::db::PersistenceEngine;
use serde::Serialize;
use tauri::Emitter;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SovereignSearchProgressEvent {
    session_id: String,
    turn_id: String,
    generation_token: String,
    stage: &'static str,
    query_digest: String,
}

pub(super) fn emit_progress(
    app: Option<&tauri::AppHandle>,
    persistence: Option<&PersistenceEngine>,
    request: &SovereignSearchExecutionRequest,
    stage: &'static str,
) {
    let query_digest = crate::foundation::digest::sha256_hex(request.query.as_bytes());
    if let (Some(session_id), Some(turn_id), Some(generation_token)) = (
        request.session_id.as_deref(),
        request.origin_turn_id.as_deref(),
        request.origin_generation_token.as_deref(),
    ) {
        crate::diagnostic_output::write_functional_acceptance_receipt(&serde_json::json!({
            "kind": "sovereign_search_progress",
            "sessionId": session_id,
            "turnId": turn_id,
            "generationToken": generation_token,
            "stage": stage,
            "queryDigest": query_digest,
        }));
    }
    let (Some(app), Some(session_id), Some(turn_id), Some(generation_token)) = (
        app,
        request.session_id.as_deref(),
        request.origin_turn_id.as_deref(),
        request.origin_generation_token.as_deref(),
    ) else {
        persist_progress(persistence, request, stage);
        return;
    };
    let _ = app.emit(
        "sovereign-search://progress",
        SovereignSearchProgressEvent {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            generation_token: generation_token.to_string(),
            stage,
            query_digest,
        },
    );
    persist_progress(persistence, request, stage);
}

fn persist_progress(
    persistence: Option<&PersistenceEngine>,
    request: &SovereignSearchExecutionRequest,
    stage: &'static str,
) {
    let (Some(persistence), Some(session_id), Some(turn_id), Some(generation_token)) = (
        persistence,
        request.session_id.as_deref(),
        request.origin_turn_id.as_deref(),
        request.origin_generation_token.as_deref(),
    ) else {
        return;
    };
    let Some(context) = persistence.select_chat_turn_context(turn_id).ok().flatten() else {
        return;
    };
    if context.session_id != session_id || context.generation_token != generation_token {
        return;
    }
    let query_digest = crate::foundation::digest::sha256_hex(request.query.as_bytes());
    let receipt_digest = crate::foundation::digest::sha256_hex(
        format!("{session_id}:{turn_id}:{generation_token}:{stage}:{query_digest}").as_bytes(),
    );
    let metadata = serde_json::json!({
        "eventKind": "sovereign_search_progress", "checkpointKind": "sovereign_search_progress",
        "checkpointForTurnId": turn_id, "uiOnlyCheckpoint": true, "sessionId": session_id,
        "turnId": turn_id, "generationToken": generation_token, "searchStage": stage,
        "queryDigest": query_digest, "progressReceiptDigest": receipt_digest,
    });
    let _ = persistence.insert_chat_message_with_metadata(
        session_id,
        &context.agent_id,
        "system",
        "sovereign_search_progress",
        Some(&context.provider_id),
        Some(&context.model_id),
        Some(&metadata),
    );
}

pub(super) fn persist(
    persistence: &PersistenceEngine,
    request: &SovereignSearchExecutionRequest,
    response: &SovereignSearchResponse,
) -> Option<(String, usize)> {
    let session_id = request.session_id.as_deref()?;
    let turn_id = request.origin_turn_id.as_deref()?;
    let generation_token = request.origin_generation_token.as_deref()?;
    let context = persistence.select_chat_turn_context(turn_id).ok()??;
    if context.session_id != session_id || context.generation_token != generation_token {
        return None;
    }
    let invocation_index = persistence
        .select_chat_messages(session_id)
        .ok()?
        .iter()
        .filter(|message| {
            message
                .metadata_json
                .as_deref()
                .and_then(|metadata| serde_json::from_str::<serde_json::Value>(metadata).ok())
                .is_some_and(|metadata| {
                    metadata
                        .get("checkpointKind")
                        .and_then(serde_json::Value::as_str)
                        == Some("verified_sovereign_search")
                        && metadata
                            .get("checkpointForTurnId")
                            .and_then(serde_json::Value::as_str)
                            == Some(turn_id)
                })
        })
        .count()
        + 1;
    let result_urls = response
        .results
        .iter()
        .map(|result| result.url.as_str())
        .collect::<Vec<_>>();
    let source_urls = verified_sources::from_context_json(&response.context_json)
        .into_iter()
        .map(|source| source.url)
        .collect::<Vec<_>>();
    if source_urls.is_empty() {
        return None;
    }
    let context_digest = crate::foundation::digest::sha256_hex(response.context_json.as_bytes());
    let receipt_payload = serde_json::json!({
        "sessionId": session_id, "turnId": turn_id, "generationToken": generation_token,
        "queryDigest": crate::foundation::digest::sha256_hex(request.query.as_bytes()),
        "contextDigest": context_digest,
        "engine": response.engine, "resultCount": response.result_count,
        "retrievedPageCount": source_urls.len(), "sourceUrls": source_urls,
        "resultUrls": result_urls,
        "accessedAtUtc": response.accessed_at_utc, "invocationIndex": invocation_index,
    });
    let receipt_digest = crate::foundation::digest::sha256_hex(
        serde_json::to_string(&receipt_payload).ok()?.as_bytes(),
    );
    let metadata = serde_json::json!({
        "eventKind": "sovereign_search_receipt", "checkpointKind": "verified_sovereign_search",
        "checkpointForTurnId": turn_id, "uiOnlyCheckpoint": true, "sessionId": session_id,
        "turnId": turn_id, "generationToken": generation_token,
        "searchReceiptDigest": receipt_digest, "searchInvocationIndex": invocation_index,
        "queryDigest": receipt_payload["queryDigest"],
        "contextDigest": receipt_payload["contextDigest"], "searchEngine": response.engine,
        "resultCount": response.result_count,
        "retrievedPageCount": receipt_payload["retrievedPageCount"],
        "sourceUrls": receipt_payload["sourceUrls"], "resultUrls": receipt_payload["resultUrls"],
        "accessedAtUtc": response.accessed_at_utc,
    });
    persistence
        .insert_chat_message_with_metadata(
            session_id,
            &context.agent_id,
            "system",
            "verified_sovereign_search",
            Some(&context.provider_id),
            Some(&context.model_id),
            Some(&metadata),
        )
        .ok()?;
    crate::diagnostic_output::write_functional_acceptance_receipt(&serde_json::json!({
        "kind": "verified_sovereign_search",
        "sessionId": session_id,
        "turnId": turn_id,
        "generationToken": generation_token,
        "receiptDigest": receipt_digest,
        "invocationIndex": invocation_index,
        "queryDigest": receipt_payload["queryDigest"],
        "contextDigest": receipt_payload["contextDigest"],
        "engine": response.engine,
        "resultCount": response.result_count,
        "retrievedPageCount": receipt_payload["retrievedPageCount"],
        "sourceUrls": receipt_payload["sourceUrls"],
        "resultUrls": receipt_payload["resultUrls"],
        "accessedAtUtc": response.accessed_at_utc,
    }));
    Some((receipt_digest, invocation_index))
}
