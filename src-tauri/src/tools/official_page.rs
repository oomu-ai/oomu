use crate::{
    foundation::digest::sha256_hex,
    network_policy::{
        resolve_destination, validate_connected_peer, validate_redirect_destination,
        CanonicalDestination, DestinationTransport, ResolvedDestinationClass,
    },
    p0_contracts::EvidenceClass,
    shield_gate::{CommandStatus, ExecuteCommandResponse},
    tools::{
        task_runtime::{record_event, require_agent_runtime_task},
        task_tool_runtime::{
            TaskToolApprovalTier, TaskToolExecutionContext, TaskToolFuture, TaskToolMetadata,
            TaskToolRegistration, TaskToolRiskTier, TaskToolValidation,
        },
    },
};
use chrono::{SecondsFormat, Utc};
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, CACHE_CONTROL, CONTENT_TYPE, LOCATION, USER_AGENT};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;

const OPERATION: &str = "fetch_official_page";
const MAX_REDIRECTS: usize = 4;
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const DEFAULT_CONTENT_CHARS: usize = 50_000;
const MAX_CONTENT_CHARS: usize = 80_000;
const MAX_FALLBACK_URLS: usize = 2;
/// Per-source ceiling when a general Workflow Agent consumes the receipt.
/// Specialized deterministic readers may use the larger fetch limit directly.
pub(crate) const MAX_AGENT_EVIDENCE_CHARS: usize = 3_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FetchOfficialPageRequest {
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) fallback_urls: Vec<String>,
    #[serde(default = "default_content_chars")]
    pub(crate) max_content_chars: usize,
}

fn default_content_chars() -> usize {
    DEFAULT_CONTENT_CHARS
}

pub(crate) fn register_task_tool() -> Result<(), String> {
    crate::tools::task_tool_runtime::register(TaskToolRegistration {
        operation: OPERATION,
        validate: validate_registration,
        validate_resolved: validate_registration,
        resolve: crate::tools::task_tool_runtime::identity_resolver,
        execute: execute_registration,
        planner_context: None,
        schema: input_schema,
        metadata: TaskToolMetadata {
            description: "Fetch one explicit public HTTPS source and return bounded readable content with its final URL, UTC access time, and SHA-256 evidence.",
            risk_tier: TaskToolRiskTier::Network,
            approval_tier: TaskToolApprovalTier::Background,
            agent_error_code: "official_page_fetch_failed",
            agent_error_boundary: "OfficialPageFetch",
            execution_path: "The native fetch_official_page tool resolved and pinned the public destination, bounded redirects and bytes, extracted readable content, and hashed the exact returned text.",
        },
    })
}

fn input_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "url":{"type":"string","minLength":8,"maxLength":8192,"description":"Exact primary or official public HTTPS page URL."},
            "fallbackUrls":{"type":"array","items":{"type":"string","minLength":8,"maxLength":8192},"maxItems":MAX_FALLBACK_URLS,"uniqueItems":true,"default":[]},
            "maxContentChars":{"type":"integer","minimum":1000,"maximum":MAX_CONTENT_CHARS,"default":DEFAULT_CONTENT_CHARS}
        },
        "required":["url"],
        "additionalProperties":false
    })
}

fn validate_registration(arguments: Value) -> Result<TaskToolValidation, String> {
    let mut request =
        serde_json::from_value::<FetchOfficialPageRequest>(arguments).map_err(|_| {
            "fetch_official_page arguments do not match the registered schema.".to_string()
        })?;
    if !(1_000..=MAX_CONTENT_CHARS).contains(&request.max_content_chars)
        || request.fallback_urls.len() > MAX_FALLBACK_URLS
    {
        return Err("fetch_official_page request is outside the bounded contract.".to_string());
    }
    request.url = normalize_public_https_url(&request.url)?;
    let mut seen = HashSet::from([request.url.clone()]);
    let mut fallbacks = Vec::with_capacity(request.fallback_urls.len());
    for fallback in request.fallback_urls {
        let fallback = normalize_public_https_url(&fallback)?;
        if seen.insert(fallback.clone()) {
            fallbacks.push(fallback);
        }
    }
    request.fallback_urls = fallbacks;
    Ok(TaskToolValidation {
        arguments: serde_json::to_value(request).map_err(|error| error.to_string())?,
        potentially_effectful: false,
    })
}

fn normalize_public_https_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let mut parsed = reqwest::Url::parse(raw)
        .map_err(|_| "fetch_official_page requires valid public HTTPS URLs.".to_string())?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || raw.len() > 8_192
    {
        return Err("fetch_official_page request is outside the bounded contract.".to_string());
    }
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

fn execute_registration<'a>(
    context: TaskToolExecutionContext<'a>,
    arguments: Value,
) -> TaskToolFuture<'a> {
    Box::pin(async move {
        let request =
            serde_json::from_value::<FetchOfficialPageRequest>(arguments).map_err(|_| {
                "fetch_official_page arguments do not match the registered schema.".to_string()
            })?;
        let execution_id = context
            .execution_id
            .ok_or_else(|| "Fetching an official page requires an active Task.".to_string())?;
        let task = require_agent_runtime_task(context.persistence, execution_id)?;
        let receipt = fetch_page(&request)
            .await
            .map_err(|error| encoded_fetch_error(&error))?;
        record_event(
            context.persistence,
            &task.task_run_id,
            "official_page.fetched",
            EvidenceClass::VerifiedPostcondition,
            json!({
                "requestedUrl":receipt.requested_url,
                "selectedUrl":receipt.selected_url,
                "attemptedUrls":receipt.attempted_urls,
                "fallbackUsed":receipt.fallback_used,
                "finalUrl":receipt.final_url,
                "accessedAtUtc":receipt.accessed_at_utc,
                "contentSha256":receipt.content_sha256,
                "contentBytes":receipt.content_bytes,
                "contentTruncated":receipt.content_truncated,
                "statusCode":receipt.status_code,
            }),
        )?;
        let message = serde_json::to_string(&receipt).map_err(|error| error.to_string())?;
        Ok(ExecuteCommandResponse {
            operation: OPERATION.to_string(),
            status: CommandStatus::Completed,
            message,
            metrics: None,
            claims: vec![format!(
                "CLAIM official_page_fetched=true final_url={} accessed_at_utc={} content_sha256={} content_bytes={}",
                receipt.final_url, receipt.accessed_at_utc, receipt.content_sha256, receipt.content_bytes
            )],
            verified: true,
            model_used: None,
        })
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfficialPageReceipt {
    pub(crate) requested_url: String,
    pub(crate) selected_url: String,
    pub(crate) attempted_urls: Vec<String>,
    pub(crate) fallback_used: bool,
    pub(crate) final_url: String,
    pub(crate) accessed_at_utc: String,
    pub(crate) status_code: u16,
    pub(crate) content_type: String,
    pub(crate) content: String,
    pub(crate) content_sha256: String,
    pub(crate) content_bytes: usize,
    pub(crate) content_truncated: bool,
}

pub(crate) async fn fetch_page(
    request: &FetchOfficialPageRequest,
) -> Result<OfficialPageReceipt, String> {
    let mut attempted_urls = Vec::with_capacity(1 + request.fallback_urls.len());
    let mut last_error = None;
    for candidate in std::iter::once(request.url.as_str())
        .chain(request.fallback_urls.iter().map(String::as_str))
    {
        attempted_urls.push(candidate.to_string());
        match fetch_candidate(candidate, request.max_content_chars).await {
            Ok(mut receipt) => {
                receipt.requested_url = request.url.clone();
                receipt.selected_url = candidate.to_string();
                receipt.fallback_used = candidate != request.url;
                receipt.attempted_urls = attempted_urls;
                return Ok(receipt);
            }
            Err(error) => last_error = Some(error),
        }
    }
    let last_error =
        last_error.unwrap_or_else(|| "The official page could not be read.".to_string());
    if request.fallback_urls.is_empty() {
        Err(last_error)
    } else {
        Err(format!(
            "None of the approved official sources could be read. {last_error}"
        ))
    }
}

async fn fetch_candidate(
    url: &str,
    max_content_chars: usize,
) -> Result<OfficialPageReceipt, String> {
    let approved = resolve_destination(url, DestinationTransport::NativeBrowser, None)
        .await
        .map_err(|error| error.message)?;
    if approved.destination_class() != ResolvedDestinationClass::Public {
        return Err("fetch_official_page requires a public HTTPS destination.".to_string());
    }
    let client = pinned_client(&approved)?;
    let mut current = approved.clone();
    for redirect_index in 0..=MAX_REDIRECTS {
        let response = client
            .get(current.url().clone())
            .header(ACCEPT, "text/html,application/xhtml+xml,text/plain;q=0.9")
            .header(ACCEPT_LANGUAGE, "en-US,en;q=0.8")
            .header(CACHE_CONTROL, "no-store")
            .header(USER_AGENT, "OOMU/1 official-source-reader")
            .send()
            .await
            .map_err(|error| bounded_fetch_error(&error))?;
        validate_connected_peer(&current, response.remote_addr()).map_err(|error| error.message)?;
        if response.status().is_redirection() {
            if redirect_index == MAX_REDIRECTS {
                return Err("The official page exceeded the redirect limit.".to_string());
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "The official page returned an invalid redirect.".to_string())?;
            let next = current
                .url()
                .join(location)
                .map_err(|_| "The official page returned an invalid redirect.".to_string())?;
            current = validate_redirect_destination(&approved, next.as_str())
                .await
                .map_err(|error| error.message)?;
            continue;
        }
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "The official page returned HTTP {}.",
                status.as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err("The official page exceeded the response size limit.".to_string());
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("text/plain")
            .to_ascii_lowercase();
        if !content_type.contains("text/html")
            && !content_type.contains("application/xhtml+xml")
            && !content_type.contains("text/plain")
        {
            return Err("The official URL did not return readable page content.".to_string());
        }
        let bytes = read_bounded_body(response).await?;
        let readable = readable_content(&content_type, &bytes);
        let (content, content_truncated) = truncate_chars(readable, max_content_chars);
        if content.trim().is_empty() {
            return Err("The official page did not expose readable content.".to_string());
        }
        let content_sha256 = sha256_hex(content.as_bytes());
        return Ok(OfficialPageReceipt {
            requested_url: url.to_string(),
            selected_url: url.to_string(),
            attempted_urls: vec![url.to_string()],
            fallback_used: false,
            final_url: current.canonical_url().to_string(),
            accessed_at_utc: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            status_code: status.as_u16(),
            content_type,
            content_bytes: content.len(),
            content,
            content_sha256,
            content_truncated,
        });
    }
    Err("The official page could not be read.".to_string())
}

fn pinned_client(destination: &CanonicalDestination) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(20))
        .resolve_to_addrs(destination.host(), &destination.resolved_socket_addresses())
        .build()
        .map_err(|_| "The official-page reader could not start.".to_string())
}

async fn read_bounded_body(response: reqwest::Response) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| bounded_fetch_error(&error))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("The official page exceeded the response size limit.".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn readable_content(content_type: &str, bytes: &[u8]) -> String {
    let decoded = String::from_utf8_lossy(bytes);
    if !content_type.contains("html") {
        return decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    let document = Html::parse_document(&decoded);
    let selector = Selector::parse("h1,h2,h3,h4,h5,h6,p,li,dt,dd,th,td,blockquote,figcaption")
        .expect("static readable-content selector");
    document
        .select(&selector)
        .map(|element| element.text().collect::<Vec<_>>().join(" "))
        .flat_map(|text| {
            text.split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_chars(content: String, max_chars: usize) -> (String, bool) {
    let mut end = 0;
    let mut chars = 0;
    for (index, character) in content.char_indices() {
        if chars == max_chars || index.saturating_add(character.len_utf8()) > max_chars {
            break;
        }
        chars += 1;
        end = index + character.len_utf8();
    }
    if end == content.len() {
        (content, false)
    } else {
        (content[..end].to_string(), true)
    }
}

fn bounded_fetch_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "The official page did not respond in time.".to_string()
    } else if error.is_connect() {
        "The official page could not be reached.".to_string()
    } else if error.is_request() || error.is_body() {
        "The network became unavailable while reading the official page.".to_string()
    } else {
        "The official page request failed.".to_string()
    }
}

fn encoded_fetch_error(message: &str) -> String {
    let lowered = message.to_ascii_lowercase();
    let code = if lowered.contains("dns resolution") {
        "dns_resolution_failed"
    } else if lowered.contains("in time")
        || lowered.contains("timeout")
        || lowered.contains("deadline")
    {
        "network_timeout"
    } else if lowered.contains("could not be reached") || lowered.contains("connect") {
        "connection_failed"
    } else if lowered.contains("network became unavailable")
        || lowered.contains("request failed")
        || [408, 425, 429]
            .into_iter()
            .chain(500..=599)
            .any(|status| lowered.contains(&format!("http {status}")))
    {
        "network_unavailable"
    } else {
        "official_page_fetch_failed"
    };
    json!({"taskToolError":{
        "code":code,
        "message":message,
        "context":{"changedState":false}
    }})
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_official_page_tool_is_bounded_and_read_only() {
        let _ = register_task_tool();
        let schema = crate::tools::task_tool_runtime::schema(OPERATION).unwrap();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            crate::tools::task_tool_runtime::risk_tier(OPERATION).unwrap(),
            TaskToolRiskTier::Network
        );
        assert_eq!(
            crate::tools::task_tool_runtime::approval_tier(OPERATION),
            Some(TaskToolApprovalTier::Background)
        );
        let validated =
            validate_registration(json!({"url":"https://www.energy.gov/news"})).unwrap();
        assert!(!validated.potentially_effectful);
        assert_eq!(
            validated.arguments["maxContentChars"],
            DEFAULT_CONTENT_CHARS
        );
        assert_eq!(validated.arguments["fallbackUrls"], json!([]));
        let validated = validate_registration(json!({
            "url":"https://ops.fhwa.dot.gov/freight/#overview",
            "fallbackUrls":[
                "https://www.fhwa.dot.gov/policyinformation/statistics.cfm",
                "https://www.fhwa.dot.gov/policyinformation/statistics.cfm#duplicate"
            ]
        }))
        .unwrap();
        assert_eq!(
            validated.arguments["url"],
            "https://ops.fhwa.dot.gov/freight/"
        );
        assert_eq!(
            validated.arguments["fallbackUrls"],
            json!(["https://www.fhwa.dot.gov/policyinformation/statistics.cfm"])
        );
        assert!(validate_registration(json!({"url":"http://127.0.0.1/private"})).is_err());
        assert!(validate_registration(json!({
            "url":"https://example.com",
            "fallbackUrls":["http://127.0.0.1/private"]
        }))
        .is_err());
        assert!(validate_registration(json!({
            "url":"https://example.com",
            "fallbackUrls":[
                "https://one.example.com",
                "https://two.example.com",
                "https://three.example.com"
            ]
        }))
        .is_err());
        assert!(validate_registration(json!({"url":"https://example.com","extra":true})).is_err());
    }

    #[test]
    fn readable_content_is_bounded_and_hashes_exact_returned_text() {
        let text = readable_content(
            "text/html",
            b"<html><script>secret()</script><h1>Official update</h1><p>Fuel index 42</p></html>",
        );
        assert_eq!(text, "Official update Fuel index 42");
        let (bounded, truncated) = truncate_chars(text, 15);
        assert!(truncated);
        assert_eq!(bounded.chars().count(), 15);
        assert_eq!(sha256_hex(bounded.as_bytes()).len(), 64);
        let (unicode, truncated) = truncate_chars("ééé".to_string(), 4);
        assert!(truncated);
        assert_eq!(unicode, "éé");
        assert_eq!(unicode.len(), 4);
    }

    #[test]
    fn transient_fetch_failures_are_typed_and_verified_unchanged() {
        for (message, code) in [
            (
                "Destination DNS resolution failed.",
                "dns_resolution_failed",
            ),
            (
                "The official page did not respond in time.",
                "network_timeout",
            ),
            (
                "The official page could not be reached.",
                "connection_failed",
            ),
            (
                "The network became unavailable while reading the official page.",
                "network_unavailable",
            ),
            (
                "The official page returned HTTP 429.",
                "network_unavailable",
            ),
            (
                "The official page returned HTTP 503.",
                "network_unavailable",
            ),
        ] {
            let normalized = crate::tools::task_tool_runtime::normalize_agent_error(
                OPERATION,
                &encoded_fetch_error(message),
            );
            let parsed = crate::tools::task_tool_runtime::parse_agent_error(&normalized).unwrap();
            assert_eq!(parsed.code, code);
            assert!(parsed.changed_state_verified);
            assert_eq!(
                parsed.changed_state,
                crate::tools::task_tool_runtime::TaskToolChangedState::None
            );
        }
    }
}
