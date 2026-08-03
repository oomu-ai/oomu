use crate::browser_proxy::{start_hidden_browser_connect_proxy, BrowserProxyHandle};
use crate::network_policy::{
    resolve_destination, validate_browser_navigation_blocking, validate_connected_peer,
    validate_redirect_destination, CanonicalDestination, DestinationTransport,
};
use encoding_rs::{Encoding, UTF_8};
use futures_util::StreamExt;
use regex::Regex;
use reqwest::header::{
    ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, CACHE_CONTROL, CONTENT_TYPE, LOCATION,
};
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::webview::PageLoadEvent;
mod headless_script;
use headless_script::headless_dom_script;

const MAX_RESPONSE_BYTES: usize = 4_000_000;
const MAX_VISIBLE_TEXT_CHARS: usize = 36_000;
const MAX_TEXT_BLOCKS: usize = 240;
const MAX_INPUTS: usize = 80;
const MAX_BUTTONS: usize = 100;
const MAX_LINKS: usize = 120;
const MAX_TABLES: usize = 12;
const MAX_TABLE_ROWS: usize = 40;
const MAX_TABLE_COLUMNS: usize = 16;
const MAX_TEMPORAL_EVIDENCE: usize = 24;
const MAX_FIELD_CHARS: usize = 320;
const MAX_URL_CHARS: usize = 2_048;
const MAX_REDIRECTS: usize = 3;
const STATIC_CONNECT_TIMEOUT: Duration = Duration::from_millis(1_500);
const STATIC_FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const HEADLESS_PAGE_TIMEOUT: Duration = Duration::from_secs(9);
const HEADLESS_EVALUATION_TIMEOUT: Duration = Duration::from_secs(3);
const HEADLESS_SETTLE_TIME: Duration = Duration::from_millis(350);
const HEADLESS_EVIDENCE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const HEADLESS_EVIDENCE_POLL_TIMEOUT: Duration = Duration::from_secs(3);
const TRANSIENT_SEARCH_RETRY_DELAY: Duration = Duration::from_millis(300);
const TRANSIENT_SEARCH_RETRY_PAGE_ATTEMPTS: usize = 2;
const STATIC_CONTENT_SUFFICIENT_CHARS: usize = 600;
static HEADLESS_WINDOW_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static HEADLESS_BROWSER_SEMAPHORE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomStreamRequest {
    pub url: String,
    pub originating_utterance: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub mod_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveDomStreamRequest {
    pub originating_utterance: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub mod_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomStreamResponse {
    pub context: DomContext,
    pub context_json: String,
    pub retrieval_elapsed_ms: u64,
    pub used_headless_browser: bool,
    pub security: DomStreamSecurity,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomStreamSecurity {
    pub cookies_enabled: bool,
    pub incognito: bool,
    pub proxy_environment_enabled: bool,
    pub visible_browser_opened: bool,
    pub public_https_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DomContext {
    pub url: String,
    pub title: String,
    pub visible_text: String,
    pub inputs: Vec<DomInput>,
    pub buttons: Vec<String>,
    pub links: Vec<DomLink>,
    pub tables: Vec<DomTable>,
    #[serde(default)]
    pub temporal_evidence: Vec<DomTemporalEvidence>,
    pub extraction_method: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DomInput {
    pub input_type: String,
    pub name: String,
    pub label: String,
    pub placeholder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DomLink {
    pub text: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DomTable {
    #[serde(default)]
    pub label: String,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DomTemporalEvidence {
    pub value: String,
    pub evidence_type: String,
    pub label: String,
}

pub(crate) struct DomStreamOutcome {
    pub(crate) context: DomContext,
    pub(crate) used_headless_browser: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DomSearchFailureKind {
    Challenge,
    Timeout,
    Cancelled,
    Irrelevant,
    Unavailable,
}

pub(crate) struct DomSearchBatch {
    pub(crate) contexts: Vec<DomContext>,
    pub(crate) attempted_count: usize,
    pub(crate) failures: Vec<DomSearchFailureKind>,
}

pub(crate) const MAX_SEARCH_PAGE_ATTEMPTS: usize = 5;

impl DomSearchBatch {
    pub(crate) fn terminal_error_code(&self) -> &'static str {
        if self
            .failures
            .iter()
            .any(|failure| *failure == DomSearchFailureKind::Cancelled)
        {
            "search_cancelled"
        } else if self
            .failures
            .iter()
            .any(|failure| *failure == DomSearchFailureKind::Timeout)
        {
            "search_retrieval_timeout"
        } else {
            "search_dom_failed"
        }
    }

    fn all_attempted_pages_failed_transiently(&self) -> bool {
        self.contexts.is_empty()
            && self.attempted_count > 0
            && self.failures.len() == self.attempted_count
            && self.failures.iter().all(|failure| {
                matches!(
                    failure,
                    DomSearchFailureKind::Timeout | DomSearchFailureKind::Unavailable
                )
            })
    }
}

#[derive(Debug)]
struct DomSearchFailure {
    kind: DomSearchFailureKind,
    detail: String,
}

impl DomSearchFailure {
    fn classified(detail: String) -> Self {
        let lowered = detail.to_ascii_lowercase();
        let kind = if lowered.contains("challenge")
            || lowered.contains("captcha")
            || lowered.contains("unusual traffic")
        {
            DomSearchFailureKind::Challenge
        } else if lowered.contains("timed out") || lowered.contains("timeout") {
            DomSearchFailureKind::Timeout
        } else if lowered.contains("cancelled") || lowered.contains("canceled") {
            DomSearchFailureKind::Cancelled
        } else {
            DomSearchFailureKind::Unavailable
        };
        Self { kind, detail }
    }

    fn irrelevant(detail: &str) -> Self {
        Self {
            kind: DomSearchFailureKind::Irrelevant,
            detail: detail.to_string(),
        }
    }
}

pub(crate) async fn stream_search_results(
    urls: &[String],
    app: Option<&tauri::AppHandle>,
    required_content_tokens: &[String],
    minimum_content_matches: usize,
    required_content_patterns: &[Regex],
    allowed_subresource_hosts: &[String],
) -> DomSearchBatch {
    let attempted_count = urls.len().min(MAX_SEARCH_PAGE_ATTEMPTS);
    let futures = urls
        .iter()
        .take(MAX_SEARCH_PAGE_ATTEMPTS)
        .cloned()
        .map(|url| {
            let required_content_tokens = required_content_tokens.to_vec();
            let required_content_patterns = required_content_patterns.to_vec();
            let allowed_subresource_hosts = allowed_subresource_hosts.to_vec();
            async move {
                match stream_public_url_with_evidence(
                    &url,
                    app,
                    &required_content_tokens,
                    minimum_content_matches,
                    &required_content_patterns,
                    &allowed_subresource_hosts,
                )
                .await
                {
                    Ok(outcome) => Ok(outcome),
                    Err(error) => {
                        eprintln!(
                            "DOM_STREAM_PUBLIC_CONTEXT_UNAVAILABLE reason={}",
                            public_context_failure_reason(&error.detail)
                        );
                        #[cfg(test)]
                        eprintln!("DOM_STREAM_PUBLIC_CONTEXT_TEST_DETAIL {}", error.detail);
                        Err(error.kind)
                    }
                }
            }
        });
    let mut contexts = Vec::new();
    let mut failures = Vec::new();
    for outcome in futures_util::future::join_all(futures).await {
        match outcome {
            Ok(outcome) => contexts.push(compact_search_context(outcome.context)),
            Err(failure) => failures.push(failure),
        }
    }
    DomSearchBatch {
        contexts,
        attempted_count,
        failures,
    }
}

pub(crate) async fn stream_search_results_with_transient_retry(
    urls: &[String],
    app: Option<&tauri::AppHandle>,
    required_content_tokens: &[String],
    minimum_content_matches: usize,
    required_content_patterns: &[Regex],
    allowed_subresource_hosts: &[String],
) -> DomSearchBatch {
    let first = stream_search_results(
        urls,
        app,
        required_content_tokens,
        minimum_content_matches,
        required_content_patterns,
        allowed_subresource_hosts,
    )
    .await;
    if !first.all_attempted_pages_failed_transiently() {
        return first;
    }

    let retry_urls = urls
        .iter()
        .take(TRANSIENT_SEARCH_RETRY_PAGE_ATTEMPTS)
        .cloned()
        .collect::<Vec<_>>();
    eprintln!(
        "SOVEREIGN_SEARCH_DOM_TRANSIENT_RETRY first_attempts={} retry_attempts={}",
        first.attempted_count,
        retry_urls.len()
    );
    tokio::time::sleep(TRANSIENT_SEARCH_RETRY_DELAY).await;
    stream_search_results(
        &retry_urls,
        app,
        required_content_tokens,
        minimum_content_matches,
        required_content_patterns,
        allowed_subresource_hosts,
    )
    .await
}

fn public_context_failure_reason(error: &str) -> &'static str {
    let lowered = error.to_ascii_lowercase();
    if lowered.contains("resolve") || lowered.contains("dns") {
        "resolution"
    } else if lowered.contains("timed out") || lowered.contains("timeout") {
        "timeout"
    } else if lowered.contains("redirect") || lowered.contains("navigation") {
        "navigation"
    } else if lowered.contains("http") {
        "http_status"
    } else if lowered.contains("size limit") || lowered.contains("exceeded") {
        "size_limit"
    } else if lowered.contains("content") {
        "content_type"
    } else if lowered.contains("peer") || lowered.contains("destination") {
        "network_policy"
    } else if lowered.contains("browser") || lowered.contains("dom extraction") {
        "browser_extraction"
    } else {
        "retrieval"
    }
}

pub(crate) fn headless_context_count(contexts: &[DomContext]) -> usize {
    contexts
        .iter()
        .filter(|context| context.extraction_method == "headless_browser")
        .count()
}

pub(crate) async fn stream_public_url_with_subresources(
    url: &str,
    app: Option<&tauri::AppHandle>,
    allowed_subresource_hosts: &[String],
) -> Result<DomStreamOutcome, String> {
    stream_public_url_with_evidence(url, app, &[], 0, &[], allowed_subresource_hosts)
        .await
        .map_err(|error| error.detail)
}

async fn stream_public_url_with_evidence(
    url: &str,
    app: Option<&tauri::AppHandle>,
    required_content_tokens: &[String],
    minimum_content_matches: usize,
    required_content_patterns: &[Regex],
    allowed_subresource_hosts: &[String],
) -> Result<DomStreamOutcome, DomSearchFailure> {
    let static_result = fetch_static_dom_context(url).await;
    if let Ok(context) = static_result.as_ref() {
        let static_has_task_evidence = context_is_usable_search_evidence(
            context,
            required_content_tokens,
            minimum_content_matches,
            required_content_patterns,
        );
        // A large static shell can still be a generic landing page. For task-bound mod
        // retrieval, let the hidden renderer hydrate the page unless the static body itself
        // contains the requested evidence. The URL is deliberately not considered evidence.
        if static_has_task_evidence {
            return Ok(DomStreamOutcome {
                context: context.clone(),
                used_headless_browser: false,
            });
        }
        if context_is_challenge_or_search_page(context) {
            return Err(DomSearchFailure {
                kind: DomSearchFailureKind::Challenge,
                detail: "Public page returned a challenge or search-results page.".to_string(),
            });
        }
    }

    if let Some(app) = app {
        match extract_with_hidden_browser(
            app,
            url,
            required_content_tokens,
            minimum_content_matches,
            required_content_patterns,
            allowed_subresource_hosts,
        )
        .await
        {
            Ok(headless) => {
                if context_is_usable_search_evidence(
                    &headless,
                    required_content_tokens,
                    minimum_content_matches,
                    required_content_patterns,
                ) {
                    return Ok(DomStreamOutcome {
                        context: headless,
                        used_headless_browser: true,
                    });
                }
                if let Ok(static_context) = static_result.as_ref() {
                    if context_is_usable_search_evidence(
                        static_context,
                        required_content_tokens,
                        minimum_content_matches,
                        required_content_patterns,
                    ) {
                        return Ok(DomStreamOutcome {
                            context: static_context.clone(),
                            used_headless_browser: false,
                        });
                    }
                }
                if context_is_challenge_or_search_page(&headless) {
                    return Err(DomSearchFailure {
                        kind: DomSearchFailureKind::Challenge,
                        detail: "Headless page returned a challenge or search-results page."
                            .to_string(),
                    });
                }
                Err(DomSearchFailure::irrelevant(
                    "Public page did not contain usable evidence for the authorized query.",
                ))
            }
            Err(headless_error) => {
                if let Ok(context) = static_result.as_ref() {
                    if context_is_usable_search_evidence(
                        context,
                        required_content_tokens,
                        minimum_content_matches,
                        required_content_patterns,
                    ) {
                        return Ok(DomStreamOutcome {
                            context: context.clone(),
                            used_headless_browser: false,
                        });
                    }
                }
                Err(DomSearchFailure::classified(headless_error))
            }
        }
    } else {
        match static_result {
            Ok(context) => {
                if context_is_usable_search_evidence(
                    &context,
                    required_content_tokens,
                    minimum_content_matches,
                    required_content_patterns,
                ) {
                    return Ok(DomStreamOutcome {
                        context,
                        used_headless_browser: false,
                    });
                }
                Err(DomSearchFailure::irrelevant(
                    "Public page did not contain usable evidence for the authorized query.",
                ))
            }
            Err(error) => Err(DomSearchFailure::classified(error)),
        }
    }
}

#[cfg(test)]
pub(crate) fn context_matches_exact_tokens(
    context: &DomContext,
    required_tokens: &[String],
    minimum_matches: usize,
) -> bool {
    if required_tokens.is_empty() || minimum_matches == 0 {
        return true;
    }

    let factual_text = context_factual_text(context);
    text_matches_exact_tokens(&factual_text, required_tokens, minimum_matches)
}

fn text_matches_exact_tokens(
    factual_text: &str,
    required_tokens: &[String],
    minimum_matches: usize,
) -> bool {
    let mut evidence_tokens = HashSet::new();
    extend_exact_tokens(&mut evidence_tokens, factual_text);

    required_tokens
        .iter()
        .filter(|token| evidence_tokens.contains(token.as_str()))
        .take(minimum_matches)
        .count()
        >= minimum_matches
}

pub(crate) fn context_matches_search_evidence(
    context: &DomContext,
    required_tokens: &[String],
    minimum_matches: usize,
    required_patterns: &[Regex],
) -> bool {
    let factual_text = context_factual_text(context);
    if !text_matches_exact_tokens(&factual_text, required_tokens, minimum_matches) {
        return false;
    }
    required_patterns
        .iter()
        .all(|pattern| pattern.is_match(&factual_text))
}

pub(crate) fn context_is_usable_search_evidence(
    context: &DomContext,
    required_tokens: &[String],
    minimum_matches: usize,
    required_patterns: &[Regex],
) -> bool {
    if context_is_challenge_or_search_page(context) {
        return false;
    }
    if required_tokens.is_empty() && required_patterns.is_empty() {
        return context_is_sufficient(context);
    }
    context_has_bounded_search_evidence(context)
        && context_matches_search_evidence(
            context,
            required_tokens,
            minimum_matches,
            required_patterns,
        )
}

pub(crate) fn context_has_bounded_search_evidence(context: &DomContext) -> bool {
    !context_is_challenge_or_search_page(context)
        && (context.visible_text.chars().count() >= 160
            || context
                .tables
                .iter()
                .any(|table| table.rows.iter().any(|row| !row.is_empty())))
}

fn context_is_challenge_or_search_page(context: &DomContext) -> bool {
    let normalized = format!("{}\n{}", context.title, context.visible_text).to_ascii_lowercase();
    if [
        "captcha",
        "unusual traffic",
        "verify you are human",
        "verify that you are human",
        "solve the challenge",
        "access denied",
        "checking your browser",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return true;
    }

    let Ok(url) = url::Url::parse(&context.url) else {
        return true;
    };
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let path = url.path().to_ascii_lowercase();
    let search_host = matches!(
        host.as_str(),
        "bing.com"
            | "www.bing.com"
            | "duckduckgo.com"
            | "html.duckduckgo.com"
            | "lite.duckduckgo.com"
            | "google.com"
            | "www.google.com"
    );
    search_host
        && (path == "/search"
            || path.starts_with("/search/")
            || path == "/lite"
            || path.starts_with("/lite/"))
}

fn context_factual_text(context: &DomContext) -> String {
    let mut text = String::with_capacity(context.visible_text.len().saturating_add(512));
    let mut push = |value: &str| {
        if !value.trim().is_empty() {
            text.push_str(value);
            text.push('\n');
        }
    };
    push(&context.visible_text);
    for input in &context.inputs {
        push(&input.label);
        push(&input.placeholder);
    }
    for button in &context.buttons {
        push(button);
    }
    for link in &context.links {
        push(&link.text);
    }
    for table in &context.tables {
        push(&table.label);
        for row in &table.rows {
            for cell in row {
                push(cell);
            }
        }
    }
    text
}

fn extend_exact_tokens<'a>(tokens: &mut HashSet<String>, value: &'a str) {
    tokens.extend(
        value
            .split(|character: char| !character.is_alphanumeric())
            .filter(|token| !token.is_empty())
            .map(str::to_lowercase),
    );
}

async fn fetch_static_dom_context(url: &str) -> Result<DomContext, String> {
    let approved = resolve_destination(url, DestinationTransport::NativeBrowser, None)
        .await
        .map_err(|error| error.message)?;
    let client = static_client(&approved)?;
    let mut current = approved.clone();

    for redirect_index in 0..=MAX_REDIRECTS {
        let response = client
            .get(current.url().clone())
            .header(
                ACCEPT,
                "text/html,application/xhtml+xml;q=0.9,text/plain;q=0.5",
            )
            .header(ACCEPT_LANGUAGE, "en-US,en;q=0.8")
            .header(ACCEPT_ENCODING, "identity")
            .header(CACHE_CONTROL, "no-store")
            .header("DNT", "1")
            .send()
            .await
            .map_err(|error| {
                let reason = if error.is_timeout() {
                    "timeout"
                } else if error.is_connect() {
                    "connection"
                } else {
                    "transport"
                };
                format!("Public page request failed ({reason}).")
            })?;
        validate_connected_peer(&current, response.remote_addr()).map_err(|error| error.message)?;

        if response.status().is_redirection() {
            if redirect_index == MAX_REDIRECTS {
                return Err("Public page exceeded the redirect limit.".to_string());
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    "Public page redirect did not include a valid location.".to_string()
                })?;
            let redirect_url = current
                .url()
                .join(location)
                .map_err(|error| format!("Public page redirect was malformed: {error}"))?;
            current = validate_redirect_destination(&approved, redirect_url.as_str())
                .await
                .map_err(|error| error.message)?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("Public page returned HTTP {}.", response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err("Public page exceeded the DOM streaming size limit.".to_string());
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !content_type.is_empty()
            && !content_type.contains("text/html")
            && !content_type.contains("application/xhtml+xml")
            && !content_type.contains("text/plain")
        {
            return Err("Public URL did not return readable page content.".to_string());
        }
        let body = read_capped_response(response, &content_type).await?;
        return Ok(extract_dom_from_html(current.canonical_url(), &body));
    }

    Err("Public page could not be read.".to_string())
}

fn static_client(destination: &CanonicalDestination) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(STATIC_CONNECT_TIMEOUT)
        .timeout(STATIC_FETCH_TIMEOUT)
        .http1_only()
        .user_agent(native_dom_user_agent())
        .resolve_to_addrs(destination.host(), &destination.resolved_socket_addresses())
        .build()
        .map_err(|error| format!("Public page client could not start: {error}"))
}

async fn read_capped_response(
    response: reqwest::Response,
    content_type: &str,
) -> Result<String, String> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("Public page read failed: {error}"))?;
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("Public page exceeded the DOM streaming size limit.".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(decode_response_body(content_type, &body))
}

fn extract_dom_from_html(url: &str, document: &str) -> DomContext {
    let page = Html::parse_document(document);
    let title = page
        .select(&selector("title"))
        .next()
        .map(element_text)
        .unwrap_or_default();
    let mut visible_blocks = Vec::new();
    let mut seen_blocks = HashSet::new();
    let semantic_selector =
        selector("h1,h2,h3,h4,h5,h6,p,li,dt,dd,blockquote,pre,figcaption,address,[role='heading']");
    for element in page.select(&semantic_selector) {
        push_visible_text_block(
            &mut visible_blocks,
            &mut seen_blocks,
            element,
            element_text(element),
        );
        if visible_blocks.len() >= MAX_TEXT_BLOCKS {
            break;
        }
    }
    if visible_blocks.len() < MAX_TEXT_BLOCKS {
        for element in page.select(&selector("div,span")) {
            let direct_text = clean_text(
                &element
                    .children()
                    .filter_map(|child| child.value().as_text())
                    .map(|text| text.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            push_visible_text_block(&mut visible_blocks, &mut seen_blocks, element, direct_text);
            if visible_blocks.len() >= MAX_TEXT_BLOCKS {
                break;
            }
        }
    }
    let visible_text = truncate_chars(&visible_blocks.join("\n"), MAX_VISIBLE_TEXT_CHARS);

    let labels = static_input_labels(&page);
    let inputs = page
        .select(&selector("input,textarea,select"))
        .filter(|element| element_is_visible(*element))
        .take(MAX_INPUTS)
        .map(|element| {
            let input_type = element
                .value()
                .attr("type")
                .unwrap_or_else(|| element.value().name())
                .to_ascii_lowercase();
            let id = element.value().attr("id").unwrap_or("");
            let label = labels
                .get(id)
                .cloned()
                .or_else(|| {
                    element
                        .ancestors()
                        .filter_map(ElementRef::wrap)
                        .find(|ancestor| ancestor.value().name() == "label")
                        .map(element_text)
                })
                .unwrap_or_default();
            DomInput {
                input_type: truncate_chars(&input_type, 40),
                name: truncate_chars(element.value().attr("name").unwrap_or(""), 120),
                label: truncate_chars(&label, MAX_FIELD_CHARS),
                placeholder: truncate_chars(
                    element.value().attr("placeholder").unwrap_or(""),
                    MAX_FIELD_CHARS,
                ),
            }
        })
        .collect();

    let buttons = page
        .select(&selector(
            "button,[role='button'],input[type='submit'],input[type='button']",
        ))
        .filter(|element| element_is_visible(*element))
        .filter_map(|element| {
            let text = if element.value().name() == "input" {
                element.value().attr("value").unwrap_or("").to_string()
            } else {
                element_text(element)
            };
            (!text.is_empty()).then(|| truncate_chars(&text, MAX_FIELD_CHARS))
        })
        .take(MAX_BUTTONS)
        .collect();

    let links = page
        .select(&selector("a[href]"))
        .filter(|element| element_is_visible(*element))
        .filter_map(|element| {
            let text = element_text(element);
            let href = element.value().attr("href")?.trim();
            if text.is_empty() || href.is_empty() {
                return None;
            }
            let absolute = reqwest::Url::parse(url).ok()?.join(href).ok()?;
            matches!(absolute.scheme(), "http" | "https").then(|| DomLink {
                text: truncate_chars(&text, MAX_FIELD_CHARS),
                url: truncate_chars(absolute.as_str(), MAX_URL_CHARS),
            })
        })
        .take(MAX_LINKS)
        .collect();

    let row_selector = selector("tr");
    let cell_selector = selector("th,td");
    let tables = page
        .select(&selector("table"))
        .filter(|element| element_is_visible(*element))
        .take(MAX_TABLES)
        .filter_map(|table| {
            let label = table
                .select(&selector("caption"))
                .next()
                .map(element_text)
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    table
                        .value()
                        .attr("aria-label")
                        .map(clean_text)
                        .filter(|value| !value.is_empty())
                })
                .unwrap_or_default();
            let rows = table
                .select(&row_selector)
                .take(MAX_TABLE_ROWS)
                .filter_map(|row| {
                    let cells = row
                        .select(&cell_selector)
                        .take(MAX_TABLE_COLUMNS)
                        .map(element_text)
                        .filter(|text| !text.is_empty())
                        .map(|text| truncate_chars(&text, MAX_FIELD_CHARS))
                        .collect::<Vec<_>>();
                    (!cells.is_empty()).then_some(cells)
                })
                .collect::<Vec<_>>();
            (!rows.is_empty()).then_some(DomTable {
                label: truncate_chars(&label, MAX_FIELD_CHARS),
                rows,
            })
        })
        .collect();
    let temporal_evidence = extract_temporal_evidence(&page);

    DomContext {
        url: truncate_chars(url, MAX_URL_CHARS),
        title: truncate_chars(&title, MAX_FIELD_CHARS),
        visible_text,
        inputs,
        buttons,
        links,
        tables,
        temporal_evidence,
        extraction_method: "static_html".to_string(),
    }
}

fn extract_temporal_evidence(page: &Html) -> Vec<DomTemporalEvidence> {
    let mut evidence = Vec::new();
    let mut seen = HashSet::new();
    for element in page.select(&selector("meta[content]")) {
        let key = element
            .value()
            .attr("property")
            .or_else(|| element.value().attr("name"))
            .or_else(|| element.value().attr("itemprop"))
            .unwrap_or("")
            .to_ascii_lowercase();
        let evidence_type = if key.contains("published") || key.contains("publication") {
            Some("publicationDate")
        } else if key.contains("modified") || key.contains("updated") {
            Some("updatedDate")
        } else if key.contains("release") {
            Some("releaseDate")
        } else {
            None
        };
        if let Some(evidence_type) = evidence_type {
            push_temporal_evidence(
                &mut evidence,
                &mut seen,
                element.value().attr("content").unwrap_or(""),
                evidence_type,
                &key,
            );
        }
    }
    for element in page.select(&selector("time[datetime]")) {
        push_temporal_evidence(
            &mut evidence,
            &mut seen,
            element.value().attr("datetime").unwrap_or(""),
            "publicationDate",
            &element_text(element),
        );
    }
    for element in page.select(&selector("script[type='application/ld+json']")) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&element.inner_html()) {
            collect_json_ld_dates(&value, &mut evidence, &mut seen);
        }
        if evidence.len() >= MAX_TEMPORAL_EVIDENCE {
            break;
        }
    }
    evidence.truncate(MAX_TEMPORAL_EVIDENCE);
    evidence
}

fn collect_json_ld_dates(
    value: &serde_json::Value,
    evidence: &mut Vec<DomTemporalEvidence>,
    seen: &mut HashSet<String>,
) {
    if evidence.len() >= MAX_TEMPORAL_EVIDENCE {
        return;
    }
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                collect_json_ld_dates(value, evidence, seen);
            }
        }
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                let normalized = key.to_ascii_lowercase();
                let evidence_type = match normalized.as_str() {
                    "datepublished" | "uploaddate" => Some("publicationDate"),
                    "datemodified" => Some("updatedDate"),
                    "datereleased" => Some("releaseDate"),
                    _ => None,
                };
                if let (Some(evidence_type), Some(raw)) = (evidence_type, value.as_str()) {
                    push_temporal_evidence(evidence, seen, raw, evidence_type, key);
                } else if evidence_type.is_none() {
                    collect_json_ld_dates(value, evidence, seen);
                }
            }
        }
        _ => {}
    }
}

fn push_temporal_evidence(
    evidence: &mut Vec<DomTemporalEvidence>,
    seen: &mut HashSet<String>,
    value: &str,
    evidence_type: &str,
    label: &str,
) {
    let value = clean_text(value);
    if value.is_empty() || value.chars().count() > 160 {
        return;
    }
    let key = format!("{evidence_type}:{}", value.to_ascii_lowercase());
    if seen.insert(key) && evidence.len() < MAX_TEMPORAL_EVIDENCE {
        evidence.push(DomTemporalEvidence {
            value,
            evidence_type: evidence_type.to_string(),
            label: truncate_chars(&clean_text(label), 160),
        });
    }
}

fn push_visible_text_block(
    blocks: &mut Vec<String>,
    seen: &mut HashSet<String>,
    element: ElementRef<'_>,
    text: String,
) {
    if text.is_empty() || !element_is_visible(element) {
        return;
    }
    let text =
        crate::dom_sanitizer::semantic_markdown_block(element, &truncate_chars(&text, 1_200));
    if seen.insert(text.clone()) {
        blocks.push(text);
    }
}

fn static_input_labels(page: &Html) -> HashMap<String, String> {
    page.select(&selector("label[for]"))
        .filter_map(|label| {
            let target = label.value().attr("for")?.trim();
            let text = element_text(label);
            (!target.is_empty() && !text.is_empty()).then(|| (target.to_string(), text))
        })
        .collect()
}

fn element_is_visible(element: ElementRef<'_>) -> bool {
    !crate::dom_sanitizer::element_is_boilerplate(element)
        && !std::iter::once(element)
            .chain(element.ancestors().filter_map(ElementRef::wrap))
            .any(|ancestor| {
                let tag = ancestor.value().name();
                if matches!(
                    tag,
                    "script" | "style" | "noscript" | "template" | "svg" | "canvas" | "head"
                ) {
                    return true;
                }
                if ancestor.value().attr("hidden").is_some()
                    || ancestor
                        .value()
                        .attr("aria-hidden")
                        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                {
                    return true;
                }
                let style = ancestor
                    .value()
                    .attr("style")
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .replace(' ', "");
                style.contains("display:none")
                    || style.contains("visibility:hidden")
                    || style.contains("opacity:0")
            })
}

struct HiddenBrowserResources {
    window: tauri::WebviewWindow,
    _proxy: BrowserProxyHandle,
}

impl Drop for HiddenBrowserResources {
    fn drop(&mut self) {
        // `destroy` is deliberately owned by this guard rather than the happy path.
        // Dropping the search future on timeout, supersession, session change, or
        // caller cancellation therefore closes the native webview and aborts its
        // proxy task before any stale result can reattach.
        let _ = self.window.destroy();
    }
}

async fn extract_with_hidden_browser(
    app: &tauri::AppHandle,
    url: &str,
    required_content_tokens: &[String],
    minimum_content_matches: usize,
    required_content_patterns: &[Regex],
    allowed_subresource_hosts: &[String],
) -> Result<DomContext, String> {
    let permit = headless_browser_semaphore()
        .acquire_owned()
        .await
        .map_err(|_| "Headless browser capacity is unavailable.".to_string())?;
    let _permit = permit;
    let destination = resolve_destination(url, DestinationTransport::NativeBrowser, None)
        .await
        .map_err(|error| error.message)?;
    let proxy =
        start_hidden_browser_connect_proxy(destination.clone(), allowed_subresource_hosts).await?;
    let label = format!(
        "oomu-headless-dom-{}",
        HEADLESS_WINDOW_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let (loaded_sender, loaded_receiver) = tokio::sync::oneshot::channel();
    let loaded_sender = Arc::new(Mutex::new(Some(loaded_sender)));
    let page_loaded_sender = loaded_sender.clone();
    let navigation_binding = destination.clone();
    let window = tauri::WebviewWindowBuilder::new(
        app,
        &label,
        tauri::WebviewUrl::External(destination.url().clone()),
    )
    .visible(false)
    .focused(false)
    .focusable(false)
    .decorations(false)
    .skip_taskbar(true)
    .content_protected(true)
    .incognito(true)
    .proxy_url(proxy.proxy_url())
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .on_download(|_, _| false)
    .on_navigation(move |navigation_url| {
        validate_browser_navigation_blocking(&navigation_binding, navigation_url.as_str()).is_ok()
    })
    .on_page_load(move |_, payload| {
        if payload.event() == PageLoadEvent::Finished {
            if let Ok(mut sender) = page_loaded_sender.lock() {
                if let Some(sender) = sender.take() {
                    let _ = sender.send(payload.url().to_string());
                }
            }
        }
    })
    .build()
    .map_err(|error| format!("Headless browser could not start: {error}"))?;

    let resources = HiddenBrowserResources {
        window,
        _proxy: proxy,
    };
    async {
        let loaded_url = tokio::time::timeout(HEADLESS_PAGE_TIMEOUT, loaded_receiver)
            .await
            .map_err(|_| "Headless browser page load timed out.".to_string())?
            .map_err(|_| "Headless browser page load was cancelled.".to_string())?;
        validate_browser_navigation_blocking(&destination, &loaded_url)
            .map_err(|error| error.message)?;
        tokio::time::sleep(HEADLESS_SETTLE_TIME).await;
        evaluate_hidden_dom_until_ready(
            &resources.window,
            required_content_tokens,
            minimum_content_matches,
            required_content_patterns,
        )
        .await
    }
    .await
}

async fn evaluate_hidden_dom_until_ready(
    window: &tauri::WebviewWindow,
    required_content_tokens: &[String],
    minimum_content_matches: usize,
    required_content_patterns: &[Regex],
) -> Result<DomContext, String> {
    if (required_content_tokens.is_empty() || minimum_content_matches == 0)
        && required_content_patterns.is_empty()
    {
        return evaluate_hidden_dom(window).await;
    }

    let deadline = tokio::time::Instant::now() + HEADLESS_EVIDENCE_POLL_TIMEOUT;
    let mut richest_context: Option<DomContext> = None;
    let mut last_error = None;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, evaluate_hidden_dom(window)).await {
            Ok(Ok(context)) => {
                if context_matches_search_evidence(
                    &context,
                    required_content_tokens,
                    minimum_content_matches,
                    required_content_patterns,
                ) {
                    return Ok(context);
                }
                if richest_context
                    .as_ref()
                    .is_none_or(|richest| context_richness(&context) > context_richness(richest))
                {
                    richest_context = Some(context);
                }
            }
            Ok(Err(error)) => last_error = Some(error),
            Err(_) => break,
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(HEADLESS_EVIDENCE_POLL_INTERVAL.min(remaining)).await;
    }

    richest_context.ok_or_else(|| {
        last_error.unwrap_or_else(|| "Headless DOM extraction timed out.".to_string())
    })
}

async fn evaluate_hidden_dom(window: &tauri::WebviewWindow) -> Result<DomContext, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Mutex::new(Some(sender));
    window
        .eval_with_callback(headless_dom_script(), move |value| {
            if let Ok(mut guard) = sender.lock() {
                if let Some(sender) = guard.take() {
                    let _ = sender.send(value);
                }
            }
        })
        .map_err(|error| format!("Headless DOM extraction could not run: {error}"))?;
    let encoded = tokio::time::timeout(HEADLESS_EVALUATION_TIMEOUT, receiver)
        .await
        .map_err(|_| "Headless DOM extraction timed out.".to_string())?
        .map_err(|_| "Headless DOM extraction callback was cancelled.".to_string())?;
    let value: serde_json::Value = serde_json::from_str(&encoded)
        .map_err(|error| format!("Headless DOM extraction returned invalid JSON: {error}"))?;
    let value = match value {
        serde_json::Value::String(inner) => {
            serde_json::from_str(&inner).unwrap_or_else(|_| serde_json::Value::String(inner))
        }
        other => other,
    };
    let mut context: DomContext = serde_json::from_value(value)
        .map_err(|error| format!("Headless DOM extraction returned invalid context: {error}"))?;
    context.url = truncate_chars(&context.url, MAX_URL_CHARS);
    context.title = truncate_chars(&clean_text(&context.title), MAX_FIELD_CHARS);
    context.visible_text = truncate_chars(
        &clean_multiline_text(&context.visible_text),
        MAX_VISIBLE_TEXT_CHARS,
    );
    context.inputs.truncate(MAX_INPUTS);
    context.buttons.truncate(MAX_BUTTONS);
    context.links.truncate(MAX_LINKS);
    context.tables.truncate(MAX_TABLES);
    for table in &mut context.tables {
        table.label = truncate_chars(&clean_text(&table.label), MAX_FIELD_CHARS);
    }
    context.extraction_method = "headless_browser".to_string();
    Ok(context)
}

pub(crate) async fn evaluate_active_dom(webview: &tauri::Webview) -> Result<DomContext, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Mutex::new(Some(sender));
    webview
        .eval_with_callback(headless_dom_script(), move |value| {
            if let Ok(mut guard) = sender.lock() {
                if let Some(sender) = guard.take() {
                    let _ = sender.send(value);
                }
            }
        })
        .map_err(|error| format!("Active DOM extraction could not run: {error}"))?;
    let encoded = tokio::time::timeout(HEADLESS_EVALUATION_TIMEOUT, receiver)
        .await
        .map_err(|_| "Active DOM extraction timed out.".to_string())?
        .map_err(|_| "Active DOM extraction callback was cancelled.".to_string())?;
    let value: serde_json::Value = serde_json::from_str(&encoded)
        .map_err(|error| format!("Active DOM extraction returned invalid JSON: {error}"))?;
    let value = match value {
        serde_json::Value::String(inner) => {
            serde_json::from_str(&inner).unwrap_or_else(|_| serde_json::Value::String(inner))
        }
        other => other,
    };
    let mut context: DomContext = serde_json::from_value(value)
        .map_err(|error| format!("Active DOM extraction returned invalid context: {error}"))?;
    context.url = truncate_chars(&context.url, MAX_URL_CHARS);
    context.title = truncate_chars(&clean_text(&context.title), MAX_FIELD_CHARS);
    context.visible_text = truncate_chars(
        &clean_multiline_text(&context.visible_text),
        MAX_VISIBLE_TEXT_CHARS,
    );
    context.inputs.truncate(MAX_INPUTS);
    context.buttons.truncate(MAX_BUTTONS);
    context.links.truncate(MAX_LINKS);
    context.tables.truncate(MAX_TABLES);
    for table in &mut context.tables {
        table.label = truncate_chars(&clean_text(&table.label), MAX_FIELD_CHARS);
    }
    Ok(context)
}

fn headless_browser_semaphore() -> Arc<tokio::sync::Semaphore> {
    HEADLESS_BROWSER_SEMAPHORE
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1)))
        .clone()
}

pub(crate) fn context_is_sufficient(context: &DomContext) -> bool {
    context.visible_text.chars().count() >= STATIC_CONTENT_SUFFICIENT_CHARS
        || !context.inputs.is_empty()
        || !context.buttons.is_empty()
        || !context.tables.is_empty()
}

fn compact_search_context(mut context: DomContext) -> DomContext {
    context.visible_text = truncate_chars(&context.visible_text, 12_000);
    context.inputs.truncate(30);
    context.buttons.truncate(40);
    context.links.truncate(40);
    context.tables.truncate(6);
    context.temporal_evidence.truncate(MAX_TEMPORAL_EVIDENCE);
    for table in &mut context.tables {
        table.label = truncate_chars(&table.label, MAX_FIELD_CHARS);
        table.rows.truncate(20);
    }
    context
}

fn context_richness(context: &DomContext) -> usize {
    context.visible_text.chars().count()
        + context.inputs.len() * 60
        + context.buttons.len() * 30
        + context.links.len() * 20
        + context
            .tables
            .iter()
            .map(|table| table.rows.len() * 80)
            .sum::<usize>()
}

fn decode_response_body(content_type: &str, data: &[u8]) -> String {
    let encoding = content_type
        .split(';')
        .find_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| value.trim().trim_matches('"'))
        })
        .and_then(|charset| Encoding::for_label(charset.as_bytes()))
        .unwrap_or(UTF_8);
    let (decoded, _, _) = encoding.decode(data);
    decoded.into_owned()
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("static DOM selector is valid")
}

fn element_text(element: ElementRef<'_>) -> String {
    clean_text(&element.text().collect::<Vec<_>>().join(" "))
}

fn clean_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clean_multiline_text(value: &str) -> String {
    value
        .lines()
        .map(clean_text)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value
        .chars()
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string()
}

fn native_dom_user_agent() -> &'static str {
    concat!(
        "OOMU/",
        env!("CARGO_PKG_VERSION"),
        " native-public-dom-reader"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
      <html>
        <head>
          <title>Flight comparison</title>
          <style>.hidden { display: none; }</style>
          <script>window.secret = "RAW SCRIPT CANARY";</script>
        </head>
        <body>
          <nav><p>NAVIGATION CANARY</p></nav>
          <section class="cookie-consent"><p>COOKIE CANARY</p></section>
          <svg><text>SVG CANARY</text></svg>
          <h1>Best flights</h1>
          <p>Singapore Airlines · $1,120 round trip</p>
          <p hidden>HIDDEN CANARY</p>
          <label for="origin">From</label>
          <input id="origin" name="origin" placeholder="Airport" value="ROC">
          <button>Compare fares</button>
          <table>
            <caption>Current flight prices</caption>
            <tr><th>Airline</th><th>Price</th></tr>
            <tr><td>Singapore Airlines</td><td>$1,120</td></tr>
          </table>
        </body>
      </html>
    "#;

    #[test]
    fn static_dom_extraction_returns_only_structured_visible_context() {
        let context = extract_dom_from_html("https://example.com/flights", FIXTURE);
        assert_eq!(context.title, "Flight comparison");
        assert!(context.visible_text.contains("Best flights"));
        assert!(context.visible_text.contains("# Best flights"));
        assert!(context.visible_text.contains("$1,120 round trip"));
        assert!(!context.visible_text.contains("NAVIGATION CANARY"));
        assert!(!context.visible_text.contains("COOKIE CANARY"));
        assert!(!context.visible_text.contains("RAW SCRIPT CANARY"));
        assert!(!context.visible_text.contains("SVG CANARY"));
        assert!(!context.visible_text.contains("HIDDEN CANARY"));
        assert_eq!(
            context.inputs,
            vec![DomInput {
                input_type: "input".to_string(),
                name: "origin".to_string(),
                label: "From".to_string(),
                placeholder: "Airport".to_string(),
            }]
        );
        assert_eq!(context.buttons, vec!["Compare fares"]);
        assert_eq!(context.tables.len(), 1);
        assert_eq!(context.tables[0].label, "Current flight prices");
        assert_eq!(context.tables[0].rows.len(), 2);
        assert_eq!(context.extraction_method, "static_html");
    }

    #[test]
    fn static_dom_extraction_preserves_bounded_structured_temporal_evidence() {
        let context = extract_dom_from_html(
            "https://www.bts.gov/newsroom/freight",
            r#"<html><head>
                <meta property="article:published_time" content="2026-07-09T09:00:00-04:00">
                <script type="application/ld+json">{"dateModified":"2026-07-10"}</script>
              </head><body>
                <time datetime="2026-07-09">July 9, 2026</time>
                <p>The Freight Transportation Services Index changed in April.</p>
              </body></html>"#,
        );
        assert!(context.temporal_evidence.iter().any(|evidence| {
            evidence.value == "2026-07-09T09:00:00-04:00"
                && evidence.evidence_type == "publicationDate"
        }));
        assert!(context.temporal_evidence.iter().any(|evidence| {
            evidence.value == "2026-07-10" && evidence.evidence_type == "updatedDate"
        }));
    }

    #[test]
    fn static_dom_never_returns_input_values_or_inline_css() {
        let context = extract_dom_from_html("https://example.com/flights", FIXTURE);
        let encoded = serde_json::to_string(&context).expect("context serializes");
        assert!(!encoded.contains("\"ROC\""));
        assert!(!encoded.contains("display: none"));
        assert!(!encoded.contains("window.secret"));
    }

    #[test]
    fn search_context_is_compacted_before_entering_model_context() {
        let mut context = extract_dom_from_html("https://example.com/flights", FIXTURE);
        context.visible_text = "x".repeat(20_000);
        context.buttons = (0..80).map(|index| format!("Button {index}")).collect();
        context.links = (0..80)
            .map(|index| DomLink {
                text: format!("Link {index}"),
                url: format!("https://example.com/{index}"),
            })
            .collect();

        let compacted = compact_search_context(context);
        assert_eq!(compacted.visible_text.chars().count(), 12_000);
        assert_eq!(compacted.buttons.len(), 40);
        assert_eq!(compacted.links.len(), 40);
    }

    #[test]
    fn transient_all_page_failure_is_retryable() {
        let batch = DomSearchBatch {
            contexts: Vec::new(),
            attempted_count: 3,
            failures: vec![
                DomSearchFailureKind::Timeout,
                DomSearchFailureKind::Unavailable,
                DomSearchFailureKind::Unavailable,
            ],
        };

        assert!(batch.all_attempted_pages_failed_transiently());
    }

    #[test]
    fn semantic_or_partial_page_failure_is_not_retryable() {
        let irrelevant = DomSearchBatch {
            contexts: Vec::new(),
            attempted_count: 2,
            failures: vec![
                DomSearchFailureKind::Unavailable,
                DomSearchFailureKind::Irrelevant,
            ],
        };
        let partial = DomSearchBatch {
            contexts: vec![extract_dom_from_html(
                "https://example.com/release",
                "<html><body><p>Rust 1.97.1 release details.</p></body></html>",
            )],
            attempted_count: 2,
            failures: vec![DomSearchFailureKind::Unavailable],
        };

        assert!(!irrelevant.all_attempted_pages_failed_transiently());
        assert!(!partial.all_attempted_pages_failed_transiently());
    }

    #[test]
    fn task_evidence_uses_exact_visible_tokens_and_never_the_page_url() {
        let mut context = extract_dom_from_html(
            "https://travel.example/flights/ROC-SIN/2027-03-14",
            r#"<html><head><title>ROC to SIN | Flight search</title></head><body>
                 <p>Baggage fees may apply. Hacker Fares combine separate tickets.</p>
               </body></html>"#,
        );
        let route_tokens = vec!["roc".to_string(), "sin".to_string()];
        let listing_patterns = vec![
            Regex::new(r"\$\s*\d{2,}").unwrap(),
            Regex::new(r"(?i)\b(?:nonstop|\d+\s+stops?|\d{1,2}:\d{2}\s*(?:am|pm)?)\b").unwrap(),
        ];

        assert!(!context_matches_exact_tokens(&context, &route_tokens, 2));

        context.visible_text =
            "ROC to SIN. Baggage fees may apply. Hacker Fares combine separate tickets."
                .to_string();
        assert!(context_matches_exact_tokens(&context, &route_tokens, 2));
        assert!(!context_matches_search_evidence(
            &context,
            &route_tokens,
            2,
            &listing_patterns
        ));

        context.visible_text = "ROC to SIN · 6:10 pm · 1 stop · $850".to_string();
        assert!(context_matches_search_evidence(
            &context,
            &route_tokens,
            2,
            &listing_patterns
        ));
    }
}
