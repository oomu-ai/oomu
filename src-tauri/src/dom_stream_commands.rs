use crate::db::PersistenceEngine;
use crate::dom_streaming::{
    evaluate_active_dom, stream_public_url_with_subresources, ActiveDomStreamRequest,
    DomStreamRequest, DomStreamResponse, DomStreamSecurity,
};
use crate::network_policy::{
    resolve_destination, validate_browser_navigation_blocking, DestinationTransport,
};
use std::time::Instant;
use tauri::Manager;

const DOM_STREAM_NOT_AUTHORIZED: &str = "dom_stream_not_authorized";
const DOM_STREAM_UNAVAILABLE: &str = "dom_stream_unavailable";

#[tauri::command(rename_all = "camelCase")]
pub async fn stream_dom_to_context(
    request: DomStreamRequest,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<DomStreamResponse, String> {
    let started_at = Instant::now();
    let allowed_subresource_hosts =
        authorize_dom_stream_request(&request, &app, persistence.inner().clone(), false).await?;
    let outcome =
        stream_public_url_with_subresources(&request.url, Some(&app), &allowed_subresource_hosts)
            .await
            .map_err(|error| structured_dom_stream_error(DOM_STREAM_UNAVAILABLE, &error))?;
    let context_json = serde_json::to_string(&outcome.context)
        .map_err(|error| structured_dom_stream_error(DOM_STREAM_UNAVAILABLE, &error.to_string()))?;
    Ok(DomStreamResponse {
        context: outcome.context,
        context_json,
        retrieval_elapsed_ms: elapsed_ms(started_at),
        used_headless_browser: outcome.used_headless_browser,
        security: DomStreamSecurity {
            cookies_enabled: false,
            incognito: true,
            proxy_environment_enabled: false,
            visible_browser_opened: false,
            public_https_only: true,
        },
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn scrape_active_page_content(
    request: ActiveDomStreamRequest,
    app: tauri::AppHandle,
    manager: tauri::State<'_, crate::native_browser::NativeBrowserManager>,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<DomStreamResponse, String> {
    let started_at = Instant::now();
    let destination = manager
        .active()
        .map_err(|error| structured_dom_stream_error(DOM_STREAM_UNAVAILABLE, &error))?;
    let webview = app
        .get_webview(crate::native_browser::BROWSER_WEBVIEW_LABEL)
        .ok_or_else(|| {
            structured_dom_stream_error(
                DOM_STREAM_UNAVAILABLE,
                "The active browser page is no longer open.",
            )
        })?;
    let current_url = webview
        .url()
        .map_err(|error| structured_dom_stream_error(DOM_STREAM_UNAVAILABLE, &error.to_string()))?;
    validate_browser_navigation_blocking(&destination, current_url.as_str())
        .map_err(|error| structured_dom_stream_error(DOM_STREAM_NOT_AUTHORIZED, &error.message))?;
    let authorization = DomStreamRequest {
        url: current_url.to_string(),
        originating_utterance: request.originating_utterance,
        session_id: request.session_id,
        mod_id: request.mod_id,
    };
    let _ = authorize_dom_stream_request(&authorization, &app, persistence.inner().clone(), true)
        .await?;
    let mut context = evaluate_active_dom(&webview)
        .await
        .map_err(|error| structured_dom_stream_error(DOM_STREAM_UNAVAILABLE, &error))?;
    context.extraction_method = "active_browser".to_string();
    let context_json = serde_json::to_string(&context)
        .map_err(|error| structured_dom_stream_error(DOM_STREAM_UNAVAILABLE, &error.to_string()))?;
    Ok(DomStreamResponse {
        context,
        context_json,
        retrieval_elapsed_ms: elapsed_ms(started_at),
        used_headless_browser: false,
        security: DomStreamSecurity {
            cookies_enabled: false,
            incognito: true,
            proxy_environment_enabled: false,
            visible_browser_opened: false,
            public_https_only: true,
        },
    })
}

async fn authorize_dom_stream_request(
    request: &DomStreamRequest,
    app: &tauri::AppHandle,
    persistence: PersistenceEngine,
    allow_active_page_reference: bool,
) -> Result<Vec<String>, String> {
    if request.originating_utterance.trim().is_empty()
        || crate::local_app_intent::has_private_app_data_intent(&request.originating_utterance)
    {
        return Err(structured_dom_stream_error(
            DOM_STREAM_NOT_AUTHORIZED,
            "Public page reading requires a direct public request.",
        ));
    }

    if let Some(mod_id) = request
        .mod_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let mod_id = mod_id.to_string();
        let utterance = request.originating_utterance.clone();
        let url = request.url.clone();
        let authorized = tauri::async_runtime::spawn_blocking(move || {
            crate::security::mods::authorize_active_network_mod_command(
                &persistence,
                &mod_id,
                &utterance,
            )
        })
        .await
        .map_err(|error| structured_dom_stream_error(DOM_STREAM_UNAVAILABLE, &error.to_string()))?
        .map_err(|error| structured_dom_stream_error(DOM_STREAM_NOT_AUTHORIZED, &error))?;
        if !authorized.allows_url(&url) {
            return Err(structured_dom_stream_error(
                DOM_STREAM_NOT_AUTHORIZED,
                "The active mod did not declare this public host.",
            ));
        }
        return Ok(authorized.allowed_hosts);
    }

    let global_enabled = crate::settings::automated_web_grounding_enabled(app)
        .map_err(|error| structured_dom_stream_error(DOM_STREAM_UNAVAILABLE, &error))?;
    let session_override =
        session_web_grounding_override(request.session_id.as_deref(), persistence.clone())
            .await
            .map_err(|error| structured_dom_stream_error(DOM_STREAM_UNAVAILABLE, &error))?;
    if !session_override.unwrap_or(global_enabled) {
        return Err(structured_dom_stream_error(
            DOM_STREAM_NOT_AUTHORIZED,
            "Search must be enabled before OOMU can read a public page.",
        ));
    }

    let destination = resolve_destination(&request.url, DestinationTransport::NativeBrowser, None)
        .await
        .map_err(|error| structured_dom_stream_error(DOM_STREAM_NOT_AUTHORIZED, &error.message))?;
    let utterance = request.originating_utterance.to_ascii_lowercase();
    let host = destination.host().to_ascii_lowercase();
    let has_direct_target = utterance.contains(&host)
        || utterance.contains(&destination.canonical_url().to_ascii_lowercase());
    let has_read_intent = [
        "browse",
        "open",
        "read",
        "inspect",
        "research",
        "check",
        "look at",
        "summarize",
        "review",
        "extract",
        "what is on",
    ]
    .iter()
    .any(|phrase| utterance.contains(phrase));
    let has_active_page_reference = allow_active_page_reference
        && [
            "active page",
            "current page",
            "this page",
            "open page",
            "active webpage",
            "current webpage",
            "this webpage",
        ]
        .iter()
        .any(|phrase| utterance.contains(phrase));
    if (!has_direct_target && !has_active_page_reference) || !has_read_intent {
        return Err(structured_dom_stream_error(
            DOM_STREAM_NOT_AUTHORIZED,
            "Public page reading requires the user to name the page and ask OOMU to read it.",
        ));
    }
    Ok(Vec::new())
}

async fn session_web_grounding_override(
    session_id: Option<&str>,
    persistence: PersistenceEngine,
) -> Result<Option<bool>, String> {
    let Some(session_id) = session_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let session_id = session_id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        persistence
            .select_chat_session_by_id(&session_id)
            .map(|session| session.web_grounding_override)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                error => Err(error),
            })
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn structured_dom_stream_error(code: &str, message: &str) -> String {
    serde_json::json!({
        "code": code,
        "message": message,
    })
    .to_string()
}
