use super::{canonical_search_query, SovereignSearchExecutionRequest, SovereignSearchResponse};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const VERIFIED_CONTEXT_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_VERIFIED_CONTEXTS: usize = 64;
const MAX_VERIFIED_CONTEXT_BYTES: usize = 512_000;

static VERIFIED_CONTEXTS: OnceLock<Mutex<VecDeque<VerifiedSearchContext>>> = OnceLock::new();

#[derive(Debug, Clone)]
pub(super) struct VerifiedSearchOrigin {
    session_id: String,
    turn_id: String,
    generation_token: String,
}

impl VerifiedSearchOrigin {
    pub(super) fn from_request(request: &SovereignSearchExecutionRequest) -> Option<Self> {
        Some(Self {
            session_id: required_id(request.session_id.as_deref()?)?,
            turn_id: required_id(request.origin_turn_id.as_deref()?)?,
            generation_token: required_id(request.origin_generation_token.as_deref()?)?,
        })
    }
}

#[derive(Debug, Clone)]
struct VerifiedSearchContext {
    origin: VerifiedSearchOrigin,
    canonical_query: String,
    context_json: String,
    context_digest: String,
    engine: String,
    created_at: Instant,
}

pub(super) struct ConsumedVerifiedSearchContext {
    origin: VerifiedSearchOrigin,
    canonical_query: String,
    pub(super) context_json: String,
    pub(super) context_digest: String,
    pub(super) engine: String,
}

pub(super) fn register_success(
    origin: Option<VerifiedSearchOrigin>,
    query: &str,
    response: &SovereignSearchResponse,
) {
    let Some(origin) = origin else { return };
    if response.degraded
        || response.dom_page_count == 0
        || response.context_json == "[]"
        || response.context_json.len() > MAX_VERIFIED_CONTEXT_BYTES
    {
        return;
    }
    let canonical_query = canonical_search_query(query);
    if canonical_query.is_empty() {
        return;
    }
    let context = VerifiedSearchContext {
        origin,
        canonical_query,
        context_digest: crate::foundation::digest::sha256_hex(response.context_json.as_bytes()),
        context_json: response.context_json.clone(),
        engine: response.engine.clone(),
        created_at: Instant::now(),
    };
    let registry = VERIFIED_CONTEXTS.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Ok(mut contexts) = registry.lock() {
        contexts.retain(|existing| existing.created_at.elapsed() < VERIFIED_CONTEXT_TTL);
        contexts.retain(|existing| {
            existing.origin.session_id != context.origin.session_id
                || existing.origin.turn_id != context.origin.turn_id
                || existing.origin.generation_token != context.origin.generation_token
                || existing.canonical_query != context.canonical_query
        });
        contexts.push_back(context);
        while contexts.len() > MAX_VERIFIED_CONTEXTS {
            contexts.pop_front();
        }
    }
}

pub(super) fn consume(
    session_id: &str,
    turn_id: &str,
    generation_token: &str,
    query: &str,
    context_json: &str,
) -> Result<ConsumedVerifiedSearchContext, &'static str> {
    let session_id = required_id(session_id).ok_or("search_continuation_invalid")?;
    let turn_id = required_id(turn_id).ok_or("search_continuation_invalid")?;
    let generation_token = required_id(generation_token).ok_or("search_continuation_invalid")?;
    let canonical_query = canonical_search_query(query);
    if canonical_query.is_empty()
        || context_json.is_empty()
        || context_json.len() > MAX_VERIFIED_CONTEXT_BYTES
    {
        return Err("search_continuation_invalid");
    }
    let digest = crate::foundation::digest::sha256_hex(context_json.as_bytes());
    let registry = VERIFIED_CONTEXTS.get_or_init(|| Mutex::new(VecDeque::new()));
    let mut contexts = registry
        .lock()
        .map_err(|_| "search_continuation_unavailable")?;
    contexts.retain(|existing| existing.created_at.elapsed() < VERIFIED_CONTEXT_TTL);
    let position = contexts.iter().position(|existing| {
        existing.origin.session_id == session_id
            && existing.origin.turn_id == turn_id
            && existing.origin.generation_token == generation_token
            && existing.canonical_query == canonical_query
    });
    let Some(position) = position else {
        return Err("search_continuation_stale");
    };
    if contexts[position].context_digest != digest
        || contexts[position].context_json != context_json
    {
        return Err("search_continuation_mismatch");
    }
    let verified = contexts
        .remove(position)
        .ok_or("search_continuation_stale")?;
    Ok(ConsumedVerifiedSearchContext {
        origin: verified.origin,
        canonical_query: verified.canonical_query,
        context_json: verified.context_json,
        context_digest: verified.context_digest,
        engine: verified.engine,
    })
}

pub(super) fn restore(consumed: ConsumedVerifiedSearchContext) {
    let context = VerifiedSearchContext {
        origin: consumed.origin,
        canonical_query: consumed.canonical_query,
        context_json: consumed.context_json,
        context_digest: consumed.context_digest,
        engine: consumed.engine,
        created_at: Instant::now(),
    };
    let registry = VERIFIED_CONTEXTS.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Ok(mut contexts) = registry.lock() {
        contexts.retain(|existing| existing.created_at.elapsed() < VERIFIED_CONTEXT_TTL);
        contexts.retain(|existing| {
            existing.origin.session_id != context.origin.session_id
                || existing.origin.turn_id != context.origin.turn_id
                || existing.origin.generation_token != context.origin.generation_token
                || existing.canonical_query != context.canonical_query
        });
        contexts.push_back(context);
        while contexts.len() > MAX_VERIFIED_CONTEXTS {
            contexts.pop_front();
        }
    }
}

fn required_id(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.chars().count() <= 256 && !value.chars().any(char::is_control))
        .then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verified_response(context_json: &str) -> SovereignSearchResponse {
        SovereignSearchResponse {
            query: "current FAA guidance".to_string(),
            engine: "duckduckgo_lite_static".to_string(),
            result_count: 1,
            results: Vec::new(),
            context_json: context_json.to_string(),
            accessed_at_utc: "2026-07-23T12:00:00.000Z".to_string(),
            retrieval_elapsed_ms: 10,
            dom_page_count: 1,
            headless_fallback_count: 0,
            degraded: false,
            error_code: None,
            error: None,
            receipt_digest: None,
            invocation_index: None,
            security: super::super::SovereignSearchSecurity {
                api_key_required: false,
                cookies_enabled: false,
                browser_automation_enabled: true,
                visible_browser_opened: false,
                proxy_environment_enabled: false,
                endpoint_allowlist: Vec::new(),
            },
        }
    }

    #[test]
    fn verified_context_is_exactly_bound_and_single_use() {
        let origin = VerifiedSearchOrigin {
            session_id: "context-session-a".to_string(),
            turn_id: "context-turn-a".to_string(),
            generation_token: "context-generation-a".to_string(),
        };
        let context_json = r#"{"pages":[{"url":"https://www.faa.gov/"}]}"#;
        register_success(
            Some(origin),
            "current FAA guidance",
            &verified_response(context_json),
        );

        assert_eq!(
            consume(
                "context-session-a",
                "context-turn-a",
                "wrong-generation",
                "current FAA guidance",
                context_json,
            )
            .err(),
            Some("search_continuation_stale")
        );
        let consumed = consume(
            "context-session-a",
            "context-turn-a",
            "context-generation-a",
            "current FAA guidance",
            context_json,
        )
        .expect("the exact native context binding should be consumable");
        assert_eq!(consumed.context_json, context_json);
        assert_eq!(
            consume(
                "context-session-a",
                "context-turn-a",
                "context-generation-a",
                "current FAA guidance",
                context_json,
            )
            .err(),
            Some("search_continuation_stale")
        );
    }

    #[test]
    fn failed_dispatch_can_restore_the_same_verified_context() {
        let origin = VerifiedSearchOrigin {
            session_id: "context-session-restore".to_string(),
            turn_id: "context-turn-restore".to_string(),
            generation_token: "context-generation-restore".to_string(),
        };
        let context_json = r#"{"pages":[{"url":"https://www.transportation.gov/"}]}"#;
        register_success(
            Some(origin),
            "current freight guidance",
            &verified_response(context_json),
        );
        let consumed = consume(
            "context-session-restore",
            "context-turn-restore",
            "context-generation-restore",
            "current freight guidance",
            context_json,
        )
        .expect("the registered context should be available");
        restore(consumed);
        assert!(consume(
            "context-session-restore",
            "context-turn-restore",
            "context-generation-restore",
            "current freight guidance",
            context_json,
        )
        .is_ok());
    }

    #[test]
    fn concurrent_duplicate_consumers_have_one_winner() {
        let origin = VerifiedSearchOrigin {
            session_id: "context-session-race".to_string(),
            turn_id: "context-turn-race".to_string(),
            generation_token: "context-generation-race".to_string(),
        };
        let context_json = r#"{"pages":[{"url":"https://www.faa.gov/race"}]}"#;
        register_success(
            Some(origin),
            "FAA race guidance",
            &verified_response(context_json),
        );
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let handles = (0..2)
            .map(|_| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    consume(
                        "context-session-race",
                        "context-turn-race",
                        "context-generation-race",
                        "FAA race guidance",
                        context_json,
                    )
                    .is_ok()
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        assert_eq!(
            handles
                .into_iter()
                .map(|handle| handle.join().expect("consumer thread should finish"))
                .filter(|consumed| *consumed)
                .count(),
            1
        );
    }

    #[test]
    fn independent_sessions_consume_independent_contexts() {
        for suffix in ["one", "two"] {
            register_success(
                Some(VerifiedSearchOrigin {
                    session_id: format!("context-session-{suffix}"),
                    turn_id: format!("context-turn-{suffix}"),
                    generation_token: format!("context-generation-{suffix}"),
                }),
                &format!("guidance {suffix}"),
                &verified_response(&format!(
                    r#"{{"pages":[{{"url":"https://example.com/{suffix}"}}]}}"#
                )),
            );
        }
        for suffix in ["one", "two"] {
            assert!(consume(
                &format!("context-session-{suffix}"),
                &format!("context-turn-{suffix}"),
                &format!("context-generation-{suffix}"),
                &format!("guidance {suffix}"),
                &format!(r#"{{"pages":[{{"url":"https://example.com/{suffix}"}}]}}"#),
            )
            .is_ok());
        }
    }
}
