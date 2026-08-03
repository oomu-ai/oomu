use super::{elapsed_ms, SovereignSearchResponse, SovereignSearchResult};
use encoding_rs::{Encoding, UTF_8};
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, CACHE_CONTROL, CONTENT_TYPE};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

pub(super) const SEARCH_NOT_AUTHORIZED: &str = "search_not_authorized";
pub(super) const SEARCH_QUERY_INVALID: &str = "search_query_invalid";
pub(super) const SEARCH_PROVIDER_CHALLENGE: &str = "search_provider_challenge";
pub(super) const SEARCH_PROVIDER_UNAVAILABLE: &str = "search_provider_unavailable";
pub(super) const SEARCH_RETRIEVAL_TIMEOUT: &str = "search_retrieval_timeout";
pub(super) const SEARCH_NO_RESULTS: &str = "search_no_results";
pub(super) const SEARCH_DOM_FAILED: &str = "search_dom_failed";
pub(super) const SEARCH_CANCELLED: &str = "search_cancelled";
pub(super) const SEARCH_UNAVAILABLE: &str = "search_unavailable";
pub(super) const OVERALL_SEARCH_TIMEOUT: Duration = Duration::from_secs(50);

static SEARCH_RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static ACTIVE_SEARCHES: OnceLock<Mutex<HashMap<SearchRunOwner, Weak<SearchRunSignal>>>> =
    OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SearchRunOwner {
    session_id: String,
    turn_id: String,
    generation_token: String,
}

impl SearchRunOwner {
    fn from_parts(
        session_id: Option<&str>,
        turn_id: Option<&str>,
        generation_token: Option<&str>,
    ) -> Option<Self> {
        Some(Self {
            session_id: bounded_owner_id(session_id?)?,
            turn_id: bounded_owner_id(turn_id?)?,
            generation_token: bounded_owner_id(generation_token?)?,
        })
    }
}

#[derive(Debug)]
struct SearchRunSignal {
    cancelled: AtomicBool,
    changed: tokio::sync::Notify,
}

impl SearchRunSignal {
    fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            self.changed.notify_waiters();
        }
    }

    async fn cancelled(&self) {
        loop {
            let changed = self.changed.notified();
            if self.cancelled.load(Ordering::Acquire) {
                return;
            }
            changed.await;
        }
    }
}

/// Owns one bounded search run. Starting a new run for the same durable session
/// and exact immutable turn generation supersedes only that prior run. A second
/// turn in the same session cannot cancel work it does not own.
pub(super) struct SearchRunLease {
    correlation_id: String,
    owner: Option<SearchRunOwner>,
    signal: Arc<SearchRunSignal>,
}

impl SearchRunLease {
    pub(super) fn begin(
        session_id: Option<&str>,
        turn_id: Option<&str>,
        generation_token: Option<&str>,
    ) -> Self {
        let sequence = SEARCH_RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let signal = Arc::new(SearchRunSignal {
            cancelled: AtomicBool::new(false),
            changed: tokio::sync::Notify::new(),
        });
        let owner = SearchRunOwner::from_parts(session_id, turn_id, generation_token);

        if let Some(key) = owner.as_ref() {
            let registry = ACTIVE_SEARCHES.get_or_init(|| Mutex::new(HashMap::new()));
            if let Ok(mut active) = registry.lock() {
                active.retain(|_, existing| existing.strong_count() > 0);
                if let Some(previous) = active.insert(key.clone(), Arc::downgrade(&signal)) {
                    if let Some(previous) = previous.upgrade() {
                        previous.cancel();
                    }
                }
            }
        }

        Self {
            correlation_id: format!("search-{sequence}"),
            owner,
            signal,
        }
    }

    pub(super) async fn cancelled(&self) {
        self.signal.cancelled().await;
    }

    pub(super) fn correlation_id(&self) -> &str {
        &self.correlation_id
    }
}

pub(super) fn cancel_owned_search(session_id: &str, turn_id: &str, generation_token: &str) -> bool {
    let Some(key) =
        SearchRunOwner::from_parts(Some(session_id), Some(turn_id), Some(generation_token))
    else {
        return false;
    };
    let Some(registry) = ACTIVE_SEARCHES.get() else {
        return false;
    };
    let current = registry
        .lock()
        .ok()
        .and_then(|active| active.get(&key).cloned())
        .and_then(|active| active.upgrade());
    if let Some(current) = current {
        current.cancel();
        true
    } else {
        false
    }
}

impl Drop for SearchRunLease {
    fn drop(&mut self) {
        let Some(key) = self.owner.as_ref() else {
            return;
        };
        let Some(registry) = ACTIVE_SEARCHES.get() else {
            return;
        };
        if let Ok(mut active) = registry.lock() {
            let owns_entry = active
                .get(key)
                .and_then(Weak::upgrade)
                .is_some_and(|current| Arc::ptr_eq(&current, &self.signal));
            if owns_entry {
                active.remove(key);
            }
        }
    }
}

fn bounded_owner_id(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.chars().count() <= 256 && !value.chars().any(char::is_control))
        .then(|| value.to_string())
}

#[derive(Debug)]
pub(super) enum ProviderAttempt {
    Results(Vec<SovereignSearchResult>),
    Challenge,
    Unavailable,
    TimedOut,
    NoResults,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ProviderRequestFailure {
    TimedOut,
    Unavailable,
}

impl ProviderRequestFailure {
    fn from_reqwest(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::TimedOut
        } else {
            Self::Unavailable
        }
    }

    pub(super) fn observability_code(self) -> &'static str {
        match self {
            Self::TimedOut => "timeout",
            Self::Unavailable => "unavailable",
        }
    }

    pub(super) fn provider_attempt(self) -> ProviderAttempt {
        match self {
            Self::TimedOut => ProviderAttempt::TimedOut,
            Self::Unavailable => ProviderAttempt::Unavailable,
        }
    }
}

pub(super) async fn fetch_duckduckgo_lite_html(
    client: &reqwest::Client,
    query: &str,
) -> Result<String, ProviderRequestFailure> {
    let response = client
        .post(super::SEARCH_ENDPOINT)
        .header(ACCEPT, "text/html,application/xhtml+xml")
        .header(ACCEPT_LANGUAGE, "en-US,en;q=0.8")
        .header(CACHE_CONTROL, "no-store")
        .header("DNT", "1")
        .form(&[("q", query)])
        .send()
        .await
        .map_err(|error| ProviderRequestFailure::from_reqwest(&error))?;
    validate_provider_response(response).await
}

pub(super) async fn fetch_bing_html(
    client: &reqwest::Client,
    query: &str,
) -> Result<String, ProviderRequestFailure> {
    let response = client
        .get(super::BING_SEARCH_ENDPOINT)
        .header(ACCEPT, "text/html,application/xhtml+xml")
        .header(ACCEPT_LANGUAGE, "en-US,en;q=0.8")
        .header(CACHE_CONTROL, "no-store")
        .header("DNT", "1")
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|error| ProviderRequestFailure::from_reqwest(&error))?;
    validate_provider_response(response).await
}

async fn validate_provider_response(
    response: reqwest::Response,
) -> Result<String, ProviderRequestFailure> {
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > super::MAX_RESPONSE_BYTES as u64)
    {
        return Err(ProviderRequestFailure::Unavailable);
    }
    read_capped_text_response(response).await
}

async fn read_capped_text_response(
    response: reqwest::Response,
) -> Result<String, ProviderRequestFailure> {
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| ProviderRequestFailure::from_reqwest(&error))?;
        if body.len().saturating_add(chunk.len()) > super::MAX_RESPONSE_BYTES {
            return Err(ProviderRequestFailure::Unavailable);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(decode_response_body(&content_type, &body))
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

pub(super) fn provider_terminal_code(
    primary: &ProviderAttempt,
    fallback: &ProviderAttempt,
) -> &'static str {
    if matches!(primary, ProviderAttempt::NoResults)
        || matches!(fallback, ProviderAttempt::NoResults)
    {
        SEARCH_NO_RESULTS
    } else if matches!(primary, ProviderAttempt::Challenge)
        || matches!(fallback, ProviderAttempt::Challenge)
    {
        SEARCH_PROVIDER_CHALLENGE
    } else if matches!(primary, ProviderAttempt::TimedOut)
        || matches!(fallback, ProviderAttempt::TimedOut)
    {
        SEARCH_RETRIEVAL_TIMEOUT
    } else {
        SEARCH_PROVIDER_UNAVAILABLE
    }
}

pub(super) fn terminal_stage(response: &SovereignSearchResponse) -> &'static str {
    match response.error_code.as_deref() {
        None => "complete",
        Some(SEARCH_NOT_AUTHORIZED) => "authorization",
        Some(SEARCH_QUERY_INVALID) => "query_validation",
        Some(SEARCH_PROVIDER_CHALLENGE | SEARCH_PROVIDER_UNAVAILABLE | SEARCH_NO_RESULTS) => {
            "provider"
        }
        Some(SEARCH_RETRIEVAL_TIMEOUT) => "deadline",
        Some(SEARCH_DOM_FAILED) => "dom",
        Some(SEARCH_CANCELLED) => "cancellation",
        _ => "runtime",
    }
}

pub(super) fn observe_terminal(
    correlation_id: &str,
    started_at: Instant,
    response: &SovereignSearchResponse,
) {
    let code = response.error_code.as_deref().unwrap_or("search_succeeded");
    eprintln!(
        "SOVEREIGN_SEARCH_TERMINAL correlation_id={} stage={} duration_ms={} engine={} result_count={} dom_page_count={} headless_count={} code={}",
        correlation_id,
        terminal_stage(response),
        elapsed_ms(started_at),
        response.engine,
        response.result_count,
        response.dom_page_count,
        response.headless_fallback_count,
        code
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_terminal_codes_are_stable_and_distinct() {
        assert_eq!(
            provider_terminal_code(&ProviderAttempt::Challenge, &ProviderAttempt::Unavailable),
            SEARCH_PROVIDER_CHALLENGE
        );
        assert_eq!(
            provider_terminal_code(&ProviderAttempt::TimedOut, &ProviderAttempt::Unavailable),
            SEARCH_RETRIEVAL_TIMEOUT
        );
        assert_eq!(
            provider_terminal_code(&ProviderAttempt::NoResults, &ProviderAttempt::Challenge),
            SEARCH_NO_RESULTS
        );
        assert_eq!(
            provider_terminal_code(&ProviderAttempt::Unavailable, &ProviderAttempt::Unavailable),
            SEARCH_PROVIDER_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn an_exact_owner_replacement_cancels_only_its_prior_run() {
        let first = SearchRunLease::begin(Some("session-a"), Some("turn-a"), Some("generation-a"));
        let unrelated =
            SearchRunLease::begin(Some("session-a"), Some("turn-b"), Some("generation-b"));
        let _replacement =
            SearchRunLease::begin(Some("session-a"), Some("turn-a"), Some("generation-a"));

        tokio::time::timeout(Duration::from_millis(50), first.cancelled())
            .await
            .expect("the superseded run should be notified");
        assert!(
            tokio::time::timeout(Duration::from_millis(5), unrelated.cancelled())
                .await
                .is_err(),
            "a different turn generation in the same session must retain its run"
        );
    }

    #[tokio::test]
    async fn explicit_cancellation_requires_the_exact_owner() {
        let active = SearchRunLease::begin(
            Some("session-explicit-cancel"),
            Some("turn-explicit-cancel"),
            Some("generation-explicit-cancel"),
        );
        assert!(!cancel_owned_search(
            "session-explicit-cancel",
            "another-turn",
            "generation-explicit-cancel"
        ));
        assert!(cancel_owned_search(
            "session-explicit-cancel",
            "turn-explicit-cancel",
            "generation-explicit-cancel"
        ));
        tokio::time::timeout(Duration::from_millis(50), active.cancelled())
            .await
            .expect("the active run should observe explicit cancellation");
        assert!(!cancel_owned_search(
            "missing-session",
            "turn-explicit-cancel",
            "generation-explicit-cancel"
        ));
    }
}
