use crate::db::PersistenceEngine;
use chrono::{SecondsFormat, Utc};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::{Duration, Instant};
mod authorization_policy;
pub(crate) mod continuation;
mod provider_selection;
mod query_binding;
mod result_url;
mod retrieval_runtime;
mod search_receipt;
mod verified_context;
pub(crate) use crate::foundation::public_web_sources as verified_sources;
use result_url::normalize_result_url;
use retrieval_runtime::*;

const SEARCH_ENDPOINT: &str = "https://lite.duckduckgo.com/lite/";
const BING_SEARCH_ENDPOINT: &str = "https://www.bing.com/search";
const DUCKDUCKGO_SEARCH_ENGINE: &str = "duckduckgo_lite_static";
const BING_SEARCH_ENGINE: &str = "bing_static_fallback";
const MOD_DECLARED_CONTEXT_ENGINE: &str = "mod_declared_public_context";
const POLICY_REGISTERED_CONTEXT_ENGINE: &str = "policy_registered_official_context";
const MAX_QUERY_CHARS: usize = 500;
const MAX_ORIGINATING_UTTERANCE_CHARS: usize = 16_000;
const MAX_RESULTS: usize = 5;
const MAX_RESPONSE_BYTES: usize = 2_000_000;
const MAX_TITLE_CHARS: usize = 180;
const MAX_SNIPPET_CHARS: usize = 420;
const MAX_URL_CHARS: usize = 2_048;
const SEARCH_TIMEOUT_MS: u64 = 8_000;
const CONNECT_TIMEOUT_MS: u64 = 750;
const WEB_GROUNDING_DISABLED_MESSAGE: &str =
    "Web search was not authorized for this request. No network request was sent.";
const ALLOWED_SEARCH_HOSTS: [&str; 5] = [
    "bing.com",
    "duckduckgo.com",
    "html.duckduckgo.com",
    "lite.duckduckgo.com",
    "www.bing.com",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SovereignSearchRequest {
    pub query: String,
    pub originating_utterance: String,
    #[serde(default)]
    pub max_results: Option<usize>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub mod_id: Option<String>,
    #[serde(default)]
    pub origin_turn_id: Option<String>,
    #[serde(default)]
    pub origin_generation_token: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SovereignSearchExecutionRequest {
    query: String,
    max_results: Option<usize>,
    session_id: Option<String>,
    origin_turn_id: Option<String>,
    origin_generation_token: Option<String>,
    authorization: SovereignSearchAuthorization,
}

#[derive(Debug, Clone)]
pub(crate) struct SovereignSearchAuthorization {
    source: SovereignSearchAuthorizationSource,
}

#[derive(Debug, Clone)]
enum SovereignSearchAuthorizationSource {
    DirectUserUtterance {
        originating_utterance: String,
        mod_id: Option<String>,
    },
    ApprovedActionPlan {
        plan_id: String,
        objective: String,
        approved_query: String,
    },
    ApprovedDecisionPack {
        plan_id: String,
        objective: String,
        approved_query: String,
        research_policy: crate::decision_research_policy::ResearchPolicy,
        policy_digest: String,
    },
    ApprovedDelegation {
        task_run_id: String,
        originating_user_objective: String,
        approved_query: String,
    },
    ApprovedMcpToolCall {
        audit_id: String,
        originating_user_objective: String,
        approved_query: String,
    },
}

impl SovereignSearchAuthorization {
    pub(crate) fn approved_mcp_tool_call(
        audit_id: impl Into<String>,
        originating_user_objective: impl Into<String>,
        approved_query: impl Into<String>,
    ) -> Self {
        Self {
            source: SovereignSearchAuthorizationSource::ApprovedMcpToolCall {
                audit_id: audit_id.into(),
                originating_user_objective: originating_user_objective.into(),
                approved_query: approved_query.into(),
            },
        }
    }

    pub(crate) fn approved_action_plan(
        plan_id: impl Into<String>,
        objective: impl Into<String>,
        approved_query: impl Into<String>,
    ) -> Self {
        Self {
            source: SovereignSearchAuthorizationSource::ApprovedActionPlan {
                plan_id: plan_id.into(),
                objective: objective.into(),
                approved_query: approved_query.into(),
            },
        }
    }

    pub(crate) fn approved_decision_pack(
        plan_id: impl Into<String>,
        objective: impl Into<String>,
        approved_query: impl Into<String>,
        research_policy: crate::decision_research_policy::ResearchPolicy,
        policy_digest: impl Into<String>,
    ) -> Self {
        Self {
            source: SovereignSearchAuthorizationSource::ApprovedDecisionPack {
                plan_id: plan_id.into(),
                objective: objective.into(),
                approved_query: approved_query.into(),
                research_policy,
                policy_digest: policy_digest.into(),
            },
        }
    }
}

impl SovereignSearchExecutionRequest {
    fn direct(request: SovereignSearchRequest) -> Self {
        Self {
            query: request.query,
            max_results: request.max_results,
            session_id: request.session_id,
            origin_turn_id: request.origin_turn_id,
            origin_generation_token: request.origin_generation_token,
            authorization: SovereignSearchAuthorization {
                source: SovereignSearchAuthorizationSource::DirectUserUtterance {
                    originating_utterance: request.originating_utterance,
                    mod_id: request.mod_id,
                },
            },
        }
    }

    pub(crate) fn approved_action_plan(
        query: impl Into<String>,
        max_results: Option<usize>,
        session_id: Option<String>,
        authorization: SovereignSearchAuthorization,
    ) -> Self {
        Self {
            query: query.into(),
            max_results,
            session_id,
            origin_turn_id: None,
            origin_generation_token: None,
            authorization,
        }
    }

    pub(crate) fn approved_delegation(
        query: impl Into<String>,
        max_results: Option<usize>,
        task_run_id: impl Into<String>,
        originating_user_objective: impl Into<String>,
        approved_query: impl Into<String>,
    ) -> Self {
        let query = query.into();
        Self {
            query,
            max_results,
            session_id: None,
            origin_turn_id: None,
            origin_generation_token: None,
            authorization: SovereignSearchAuthorization {
                source: SovereignSearchAuthorizationSource::ApprovedDelegation {
                    task_run_id: task_run_id.into(),
                    originating_user_objective: originating_user_objective.into(),
                    approved_query: approved_query.into(),
                },
            },
        }
    }

    pub(crate) fn approved_mcp_tool_call(
        query: impl Into<String>,
        max_results: Option<usize>,
        session_id: impl Into<String>,
        origin_turn_id: impl Into<String>,
        origin_generation_token: impl Into<String>,
        authorization: SovereignSearchAuthorization,
    ) -> Self {
        Self {
            query: query.into(),
            max_results,
            session_id: Some(session_id.into()),
            origin_turn_id: Some(origin_turn_id.into()),
            origin_generation_token: Some(origin_generation_token.into()),
            authorization,
        }
    }
}

#[derive(Debug, Default)]
struct VerifiedSearchAuthorization {
    allowed_result_hosts: Option<Vec<String>>,
    direct_context_urls: Vec<String>,
    direct_context_engine: Option<&'static str>,
    require_query_evidence: bool,
    required_content_patterns: Vec<Regex>,
}

impl VerifiedSearchAuthorization {
    fn allows_result_url(&self, url: &str) -> bool {
        self.allowed_result_hosts
            .as_ref()
            .is_none_or(|allowed_hosts| {
                let authorized = crate::security::mods::AuthorizedNetworkModCommand {
                    mod_id: "verified_search".to_string(),
                    search_query: String::new(),
                    allowed_hosts: allowed_hosts.clone(),
                    context_urls: Vec::new(),
                    required_context_evidence_patterns: Vec::new(),
                };
                authorized.allows_url(url)
            })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SovereignSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SovereignSearchResponse {
    pub query: String,
    pub engine: String,
    pub result_count: usize,
    pub results: Vec<SovereignSearchResult>,
    pub context_json: String,
    pub accessed_at_utc: String,
    pub retrieval_elapsed_ms: u64,
    pub dom_page_count: usize,
    pub headless_fallback_count: usize,
    pub degraded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocation_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub security: SovereignSearchSecurity,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SovereignSearchSecurity {
    pub api_key_required: bool,
    pub cookies_enabled: bool,
    pub browser_automation_enabled: bool,
    pub visible_browser_opened: bool,
    pub proxy_environment_enabled: bool,
    pub endpoint_allowlist: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelSovereignSearchRequest {
    pub session_id: String,
    pub origin_turn_id: String,
    pub origin_generation_token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelSovereignSearchResponse {
    pub cancelled: bool,
}

#[tauri::command(rename_all = "camelCase")]
pub fn cancel_sovereign_search(
    request: CancelSovereignSearchRequest,
) -> CancelSovereignSearchResponse {
    let cancelled = cancel_owned_search(
        &request.session_id,
        &request.origin_turn_id,
        &request.origin_generation_token,
    );
    CancelSovereignSearchResponse { cancelled }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn sovereign_duckduckgo_search(
    request: SovereignSearchRequest,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
) -> Result<SovereignSearchResponse, String> {
    let response = execute_sovereign_duckduckgo_search(
        SovereignSearchExecutionRequest::direct(request),
        Some(&app),
        Some(persistence.inner().clone()),
    )
    .await
    .map_err(|code| structured_search_error(&code))?;
    if !response.degraded
        && (response.receipt_digest.is_none() || response.invocation_index.is_none())
    {
        eprintln!("SOVEREIGN_SEARCH_RECEIPT_REQUIRED no inference context released");
        return Err(structured_search_error(SEARCH_UNAVAILABLE));
    }
    Ok(response)
}

pub(crate) async fn execute_sovereign_duckduckgo_search(
    request: SovereignSearchExecutionRequest,
    app: Option<&tauri::AppHandle>,
    persistence: Option<PersistenceEngine>,
) -> Result<SovereignSearchResponse, String> {
    let started_at = Instant::now();
    let run = SearchRunLease::begin(
        request.session_id.as_deref(),
        request.origin_turn_id.as_deref(),
        request.origin_generation_token.as_deref(),
    );
    let verified_origin = verified_context::VerifiedSearchOrigin::from_request(&request);
    let verification_query = request.query.clone();
    let Some(authorization) =
        web_grounding_authorization(&request, app, persistence.clone()).await?
    else {
        let response = disabled_search_response(&request, started_at);
        observe_terminal(run.correlation_id(), started_at, &response);
        return Ok(response);
    };
    search_receipt::emit_progress(app, persistence.as_ref(), &request, "search_started");

    let terminal_query = truncate_chars(&clean_text(&request.query), MAX_QUERY_CHARS);
    let browser_automation_enabled = app.is_some();
    let pipeline = search_duckduckgo_lite(request.clone(), authorization, app, started_at);
    tokio::pin!(pipeline);
    let deadline = tokio::time::sleep(OVERALL_SEARCH_TIMEOUT);
    tokio::pin!(deadline);
    let mut response = tokio::select! {
        _ = run.cancelled() => search_response(
            terminal_query.clone(),
            started_at,
            DUCKDUCKGO_SEARCH_ENGINE,
            Vec::new(),
            Vec::new(),
            true,
            Some(SEARCH_CANCELLED),
            browser_automation_enabled,
        ),
        _ = &mut deadline => search_response(
            terminal_query.clone(),
            started_at,
            DUCKDUCKGO_SEARCH_ENGINE,
            Vec::new(),
            Vec::new(),
            true,
            Some(SEARCH_RETRIEVAL_TIMEOUT),
            browser_automation_enabled,
        ),
        response = &mut pipeline => response,
    };
    if !response.degraded {
        if let Some((receipt_digest, invocation_index)) = persistence
            .as_ref()
            .and_then(|engine| search_receipt::persist(engine, &request, &response))
        {
            response.receipt_digest = Some(receipt_digest);
            response.invocation_index = Some(invocation_index);
        }
    }
    search_receipt::emit_progress(
        app,
        persistence.as_ref(),
        &request,
        if response.degraded {
            "search_failed"
        } else {
            "evidence_received"
        },
    );
    verified_context::register_success(verified_origin, &verification_query, &response);
    observe_terminal(run.correlation_id(), started_at, &response);
    Ok(response)
}

async fn search_duckduckgo_lite(
    request: SovereignSearchExecutionRequest,
    authorization: VerifiedSearchAuthorization,
    app: Option<&tauri::AppHandle>,
    started_at: Instant,
) -> SovereignSearchResponse {
    let query = truncate_chars(&clean_text(&request.query), MAX_QUERY_CHARS);
    let max_results = request
        .max_results
        .unwrap_or(MAX_RESULTS)
        .clamp(1, MAX_RESULTS);

    if query.is_empty() {
        return search_response(
            query,
            started_at,
            DUCKDUCKGO_SEARCH_ENGINE,
            Vec::new(),
            Vec::new(),
            true,
            Some(SEARCH_QUERY_INVALID),
            app.is_some(),
        );
    }

    let mod_evidence =
        ModQueryEvidence::from_query(&query, authorization.required_content_patterns.clone());
    if mod_evidence.minimum_matches == 0 {
        return search_response(
            query,
            started_at,
            DUCKDUCKGO_SEARCH_ENGINE,
            Vec::new(),
            Vec::new(),
            true,
            Some(SEARCH_QUERY_INVALID),
            app.is_some(),
        );
    }

    if !authorization.direct_context_urls.is_empty() {
        let batch = crate::dom_streaming::stream_search_results_with_transient_retry(
            &authorization.direct_context_urls,
            app,
            &mod_evidence.tokens,
            mod_evidence.minimum_matches,
            &mod_evidence.required_patterns,
            authorization
                .allowed_result_hosts
                .as_deref()
                .unwrap_or_default(),
        )
        .await;
        let pages = batch.contexts;
        if pages.is_empty() {
            eprintln!("SOVEREIGN_SEARCH_DECLARED_CONTEXT_IRRELEVANT");
        }
        if !pages.is_empty() {
            let results = pages
                .iter()
                .map(|page| SovereignSearchResult {
                    title: if page.title.trim().is_empty() {
                        "Approved public source".to_string()
                    } else {
                        page.title.clone()
                    },
                    url: page.url.clone(),
                    snippet: truncate_chars(&clean_text(&page.visible_text), MAX_SNIPPET_CHARS),
                })
                .collect::<Vec<_>>();
            return search_response(
                query,
                started_at,
                authorization
                    .direct_context_engine
                    .unwrap_or(MOD_DECLARED_CONTEXT_ENGINE),
                results,
                pages,
                false,
                None,
                app.is_some(),
            );
        }
        eprintln!("SOVEREIGN_SEARCH_DECLARED_CONTEXT_UNAVAILABLE");
    }

    let client = match search_client() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("SOVEREIGN_SEARCH_CLIENT_UNAVAILABLE {error}");
            return search_response(
                query,
                started_at,
                DUCKDUCKGO_SEARCH_ENGINE,
                Vec::new(),
                Vec::new(),
                true,
                Some(SEARCH_PROVIDER_UNAVAILABLE),
                app.is_some(),
            );
        }
    };

    let (selected_engine, mut results) =
        match provider_selection::fetch_ranked_provider_results(&client, &query, max_results).await
        {
            Ok(selection) => selection,
            Err(code) => {
                return search_response(
                    query,
                    started_at,
                    BING_SEARCH_ENGINE,
                    Vec::new(),
                    Vec::new(),
                    true,
                    Some(code),
                    app.is_some(),
                )
            }
        };

    results = results
        .into_iter()
        .filter(|result| authorization.allows_result_url(&result.url))
        .collect::<Vec<_>>();
    if results.is_empty() {
        return search_response(
            query,
            started_at,
            selected_engine,
            Vec::new(),
            Vec::new(),
            true,
            Some(SEARCH_NO_RESULTS),
            app.is_some(),
        );
    }
    let page_urls = results
        .iter()
        .take(crate::dom_streaming::MAX_SEARCH_PAGE_ATTEMPTS)
        .map(|result| result.url.clone())
        .collect::<Vec<_>>();
    let batch = crate::dom_streaming::stream_search_results_with_transient_retry(
        &page_urls,
        app,
        &mod_evidence.tokens,
        mod_evidence.minimum_matches,
        &mod_evidence.required_patterns,
        authorization
            .allowed_result_hosts
            .as_deref()
            .unwrap_or_default(),
    )
    .await;
    let attempted_count = batch.attempted_count;
    let terminal_error_code = batch.terminal_error_code();
    let pages = batch.contexts;
    if authorization.require_query_evidence {
        results.retain(|result| mod_evidence.matches_result(result));
    }
    if pages.is_empty() {
        eprintln!("SOVEREIGN_SEARCH_MOD_CONTEXT_IRRELEVANT");
        let code = if attempted_count == 0 {
            SEARCH_NO_RESULTS
        } else {
            terminal_error_code
        };
        return search_response(
            query,
            started_at,
            selected_engine,
            Vec::new(),
            Vec::new(),
            true,
            Some(code),
            app.is_some(),
        );
    }
    search_response(
        query,
        started_at,
        selected_engine,
        results,
        pages,
        false,
        None,
        app.is_some(),
    )
}

async fn web_grounding_authorization(
    request: &SovereignSearchExecutionRequest,
    app: Option<&tauri::AppHandle>,
    persistence: Option<PersistenceEngine>,
) -> Result<Option<VerifiedSearchAuthorization>, String> {
    if let Some((originating_utterance, mod_id)) = request.authorization.direct_mod_command() {
        let Some(persistence) = persistence else {
            return Ok(None);
        };
        let persistence_for_auth = persistence.clone();
        let mod_id = mod_id.to_string();
        let utterance = originating_utterance.to_string();
        let authorized = tauri::async_runtime::spawn_blocking(move || {
            crate::security::mods::authorize_active_network_mod_command(
                &persistence_for_auth,
                &mod_id,
                &utterance,
            )
        })
        .await
        .map_err(|error| {
            eprintln!("SOVEREIGN_SEARCH_MOD_AUTH_TASK_FAILED {error}");
            SEARCH_UNAVAILABLE.to_string()
        })?
        .ok();
        let Some(authorized) = authorized else {
            return Ok(None);
        };
        if !search_queries_match(&authorized.search_query, &request.query)
            || crate::local_app_intent::has_private_app_data_intent(&request.query)
        {
            return Ok(None);
        }
        let required_content_patterns = authorized
            .required_context_evidence_patterns
            .iter()
            .map(|pattern| Regex::new(pattern))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                eprintln!("SOVEREIGN_SEARCH_MOD_EVIDENCE_PATTERN_INVALID {error}");
                SEARCH_UNAVAILABLE.to_string()
            })?;
        return Ok(Some(VerifiedSearchAuthorization {
            allowed_result_hosts: Some(authorized.allowed_hosts),
            direct_context_urls: authorized.context_urls,
            direct_context_engine: Some(MOD_DECLARED_CONTEXT_ENGINE),
            require_query_evidence: true,
            required_content_patterns,
        }));
    }

    // The Search switch controls ambient freshness grounding. It must not veto
    // a bounded search the user explicitly requested in this turn or approved
    // in a signed action plan. That distinction is also the promise shown in
    // Privacy settings: ordinary prompts stay offline while explicit public
    // web requests remain available.
    let consent_enabled = if request
        .authorization
        .explicitly_authorizes_external_search()
    {
        true
    } else {
        let global_enabled = match app {
            Some(app) => {
                crate::settings::automated_web_grounding_enabled(app).map_err(|error| {
                    eprintln!("SOVEREIGN_SEARCH_SETTINGS_UNAVAILABLE {error}");
                    SEARCH_UNAVAILABLE.to_string()
                })?
            }
            None => crate::settings::automated_web_grounding_enabled_from_disk(),
        };
        let session_override =
            session_web_grounding_override(request.session_id.as_deref(), persistence.clone())
                .await
                .map_err(|error| {
                    eprintln!("SOVEREIGN_SEARCH_SESSION_SETTING_UNAVAILABLE {error}");
                    SEARCH_UNAVAILABLE.to_string()
                })?;
        effective_web_grounding_enabled(global_enabled, session_override)
    };
    if !consent_enabled {
        return Ok(None);
    }

    Ok(search_boundary_authorization(
        consent_enabled,
        &request.authorization,
        &request.query,
    ))
}

#[derive(Debug, Default)]
struct ModQueryEvidence {
    tokens: Vec<String>,
    minimum_matches: usize,
    required_patterns: Vec<Regex>,
}

impl ModQueryEvidence {
    fn from_query(query: &str, required_patterns: Vec<Regex>) -> Self {
        const LOW_SIGNAL_TERMS: &[&str] = &[
            "about",
            "and",
            "at",
            "best",
            "browse",
            "by",
            "compare",
            "current",
            "data",
            "date",
            "dates",
            "details",
            "do",
            "does",
            "facts",
            "find",
            "flight",
            "for",
            "from",
            "get",
            "give",
            "help",
            "hotel",
            "how",
            "in",
            "information",
            "is",
            "it",
            "latest",
            "live",
            "look",
            "me",
            "need",
            "of",
            "on",
            "online",
            "options",
            "or",
            "official",
            "please",
            "price",
            "prices",
            "release",
            "releases",
            "research",
            "results",
            "return",
            "returning",
            "search",
            "show",
            "stable",
            "the",
            "this",
            "to",
            "today",
            "using",
            "want",
            "web",
            "website",
            "websites",
            "what",
            "when",
            "where",
            "which",
            "who",
            "why",
            "will",
            "with",
            "you",
        ];

        let skip_command_name = usize::from(query.trim_start().starts_with('/'));
        let mut tokens = query
            .split(|character: char| !character.is_alphanumeric())
            .filter(|token| !token.is_empty())
            .skip(skip_command_name)
            .map(|token| token.to_lowercase())
            .filter(|token| {
                (token.chars().count() >= 2
                    || token.chars().all(|character| character.is_numeric()))
                    && !LOW_SIGNAL_TERMS.contains(&token.as_str())
            })
            .collect::<Vec<_>>();
        tokens.sort();
        tokens.dedup();
        let minimum_matches = tokens.len().min(2);
        Self {
            tokens,
            minimum_matches,
            required_patterns,
        }
    }

    fn matches_result(&self, result: &SovereignSearchResult) -> bool {
        self.matches_text_values([result.title.as_str(), result.snippet.as_str()])
    }

    #[cfg(test)]
    fn matches_page(&self, page: &crate::dom_streaming::DomContext) -> bool {
        self.minimum_matches > 0
            && crate::dom_streaming::context_matches_search_evidence(
                page,
                &self.tokens,
                self.minimum_matches,
                &self.required_patterns,
            )
    }

    fn matches_text_values<'a>(&self, values: impl IntoIterator<Item = &'a str>) -> bool {
        if self.minimum_matches == 0 {
            return false;
        }
        let values = values.into_iter().collect::<Vec<_>>();
        let mut evidence_tokens = HashSet::new();
        for value in &values {
            evidence_tokens.extend(
                value
                    .split(|character: char| !character.is_alphanumeric())
                    .filter(|token| !token.is_empty())
                    .map(str::to_lowercase),
            );
        }
        let query_matches = self
            .tokens
            .iter()
            .filter(|token| evidence_tokens.contains(token.as_str()))
            .take(self.minimum_matches)
            .count()
            >= self.minimum_matches;
        if !query_matches {
            return false;
        }
        let evidence_text = values.join("\n");
        self.required_patterns
            .iter()
            .all(|pattern| pattern.is_match(&evidence_text))
    }
}

#[cfg(test)]
fn mod_context_has_query_evidence(
    query: &str,
    results: &[SovereignSearchResult],
    pages: &[crate::dom_streaming::DomContext],
) -> bool {
    let requirement = ModQueryEvidence::from_query(query, Vec::new());
    results
        .iter()
        .any(|result| requirement.matches_result(result))
        || pages.iter().any(|page| requirement.matches_page(page))
}

fn effective_web_grounding_enabled(global_enabled: bool, session_override: Option<bool>) -> bool {
    session_override.unwrap_or(global_enabled)
}

impl SovereignSearchAuthorization {
    fn direct_mod_command(&self) -> Option<(&str, &str)> {
        match &self.source {
            SovereignSearchAuthorizationSource::DirectUserUtterance {
                originating_utterance,
                mod_id: Some(mod_id),
            } => Some((originating_utterance, mod_id)),
            _ => None,
        }
    }

    fn explicitly_authorizes_external_search(&self) -> bool {
        match &self.source {
            SovereignSearchAuthorizationSource::DirectUserUtterance {
                originating_utterance,
                mod_id,
            } => {
                let utterance = originating_utterance.trim();
                !utterance.is_empty()
                    && utterance.chars().count() <= MAX_ORIGINATING_UTTERANCE_CHARS
                    && mod_id.is_none()
                    && explicit_external_search_requested(utterance)
            }
            SovereignSearchAuthorizationSource::ApprovedActionPlan {
                plan_id, objective, ..
            } => !plan_id.trim().is_empty() && explicit_external_search_requested(objective),
            SovereignSearchAuthorizationSource::ApprovedDecisionPack {
                plan_id, objective, ..
            } => !plan_id.trim().is_empty() && explicit_external_search_requested(objective),
            SovereignSearchAuthorizationSource::ApprovedDelegation {
                task_run_id,
                originating_user_objective,
                approved_query,
            } => {
                !task_run_id.trim().is_empty()
                    && !approved_query.trim().is_empty()
                    && explicit_external_search_requested(originating_user_objective)
            }
            SovereignSearchAuthorizationSource::ApprovedMcpToolCall {
                audit_id,
                approved_query,
                ..
            } => !audit_id.trim().is_empty() && !approved_query.trim().is_empty(),
        }
    }

    fn binds_ambient_freshness_query(&self, query: &str) -> bool {
        match &self.source {
            SovereignSearchAuthorizationSource::DirectUserUtterance {
                originating_utterance,
                mod_id,
            } => {
                mod_id.is_none()
                    && freshness_search_requested(originating_utterance)
                    && search_queries_match(originating_utterance, query)
            }
            SovereignSearchAuthorizationSource::ApprovedActionPlan {
                objective,
                approved_query,
                ..
            }
            | SovereignSearchAuthorizationSource::ApprovedDelegation {
                originating_user_objective: objective,
                approved_query,
                ..
            } => {
                freshness_search_requested(objective)
                    && search_queries_match(objective, query)
                    && search_queries_match(approved_query, query)
            }
            SovereignSearchAuthorizationSource::ApprovedDecisionPack { .. }
            | SovereignSearchAuthorizationSource::ApprovedMcpToolCall { .. } => false,
        }
    }

    fn binds_query(&self, query: &str) -> bool {
        match &self.source {
            SovereignSearchAuthorizationSource::DirectUserUtterance {
                originating_utterance,
                mod_id,
            } => {
                mod_id.is_none()
                    && (explicit_search_queries_from_utterance(originating_utterance)
                        .iter()
                        .any(|approved_query| search_queries_match(approved_query, query))
                        || authorization_policy::objective_bound_refined_query_allowed(
                            originating_utterance,
                            query,
                        ))
            }
            SovereignSearchAuthorizationSource::ApprovedActionPlan { approved_query, .. }
            | SovereignSearchAuthorizationSource::ApprovedDelegation { approved_query, .. } => {
                approved_query.trim() == query.trim()
            }
            SovereignSearchAuthorizationSource::ApprovedMcpToolCall {
                originating_user_objective,
                approved_query,
                ..
            } => !originating_user_objective.trim().is_empty() && approved_query == query,
            SovereignSearchAuthorizationSource::ApprovedDecisionPack {
                objective,
                approved_query,
                research_policy,
                policy_digest,
                ..
            } => {
                approved_query.trim() == query.trim()
                    && crate::decision_research_policy::signed_policy_authority_for_query(
                        objective,
                        research_policy,
                        policy_digest,
                        query,
                    )
                    .is_some()
            }
        }
    }

    fn targets_private_data(&self) -> bool {
        match &self.source {
            SovereignSearchAuthorizationSource::DirectUserUtterance {
                originating_utterance,
                ..
            } => {
                crate::local_app_intent::has_private_app_data_intent(originating_utterance)
                    || localized_private_search_target(originating_utterance)
            }
            SovereignSearchAuthorizationSource::ApprovedActionPlan {
                objective,
                approved_query,
                ..
            } => {
                (crate::local_app_intent::has_private_app_data_intent(objective)
                    || localized_private_search_target(objective))
                    && !independent_public_research_query_allowed(objective, approved_query)
            }
            SovereignSearchAuthorizationSource::ApprovedDelegation {
                originating_user_objective,
                ..
            } => {
                crate::local_app_intent::has_private_app_data_intent(originating_user_objective)
                    || localized_private_search_target(originating_user_objective)
            }
            SovereignSearchAuthorizationSource::ApprovedMcpToolCall {
                originating_user_objective,
                approved_query,
                ..
            } => {
                crate::local_app_intent::has_private_app_data_intent(originating_user_objective)
                    || localized_private_search_target(originating_user_objective)
                    || crate::local_app_intent::has_private_app_data_intent(approved_query)
                    || localized_private_search_target(approved_query)
            }
            SovereignSearchAuthorizationSource::ApprovedDecisionPack {
                objective,
                approved_query,
                research_policy,
                policy_digest,
                ..
            } => {
                (crate::local_app_intent::has_private_app_data_intent(objective)
                    || localized_private_search_target(objective))
                    && crate::decision_research_policy::signed_policy_authority_for_query(
                        objective,
                        research_policy,
                        policy_digest,
                        approved_query,
                    )
                    .is_none()
            }
        }
    }
}

fn search_boundary_authorization(
    consent_enabled: bool,
    authorization: &SovereignSearchAuthorization,
    query: &str,
) -> Option<VerifiedSearchAuthorization> {
    let explicitly_bound =
        authorization.explicitly_authorizes_external_search() && authorization.binds_query(query);
    let ambient_freshness_bound =
        consent_enabled && authorization.binds_ambient_freshness_query(query);
    if !consent_enabled
        || query.trim().is_empty()
        || query.chars().count() > MAX_QUERY_CHARS
        || crate::local_app_intent::has_private_app_data_intent(query)
        || localized_private_search_target(query)
        || (!explicitly_bound && !ambient_freshness_bound)
        || authorization.targets_private_data()
    {
        return None;
    }
    let (allowed_result_hosts, direct_context_urls, direct_context_engine, require_query_evidence) =
        match &authorization.source {
            SovereignSearchAuthorizationSource::ApprovedDecisionPack {
                objective,
                approved_query,
                research_policy,
                policy_digest,
                ..
            } => {
                let binding =
                    crate::decision_research_policy::signed_policy_authority_binding_for_query(
                        objective,
                        research_policy,
                        policy_digest,
                        approved_query,
                    )?;
                (
                    Some(
                        binding
                            .profile
                            .hosts
                            .iter()
                            .flat_map(|host| [(*host).to_string(), format!("www.{host}")])
                            .collect(),
                    ),
                    binding
                        .direct_context_urls
                        .iter()
                        .map(|url| (*url).to_string())
                        .collect(),
                    Some(POLICY_REGISTERED_CONTEXT_ENGINE),
                    true,
                )
            }
            _ => (None, Vec::new(), None, false),
        };
    Some(VerifiedSearchAuthorization {
        allowed_result_hosts,
        direct_context_urls,
        direct_context_engine,
        require_query_evidence,
        ..VerifiedSearchAuthorization::default()
    })
}

pub(crate) fn delegated_search_authorization_is_valid(
    originating_user_objective: &str,
    approved_query: &str,
    query: &str,
) -> bool {
    let authorization = SovereignSearchAuthorization {
        source: SovereignSearchAuthorizationSource::ApprovedDelegation {
            task_run_id: "validated-delegation".to_string(),
            originating_user_objective: originating_user_objective.to_string(),
            approved_query: approved_query.to_string(),
        },
    };
    search_boundary_authorization(true, &authorization, query).is_some()
}

fn search_queries_match(approved: &str, requested: &str) -> bool {
    canonical_search_query(approved) == canonical_search_query(requested)
}

fn canonical_search_query(value: &str) -> String {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

fn explicit_search_queries_from_utterance(utterance: &str) -> Vec<String> {
    let Some(primary) = query_binding::explicit_search_query_from_utterance(utterance) else {
        return Vec::new();
    };
    authorization_policy::separate_release_queries(utterance, primary)
}

#[cfg(test)]
fn explicit_search_query_from_utterance(utterance: &str) -> Option<String> {
    query_binding::explicit_search_query_from_utterance(utterance)
}

fn strip_search_courtesy_prefix(value: &str) -> String {
    let mut value = value.trim().to_string();
    loop {
        let original = value.clone();
        for pattern in [
            r"(?i)^(?:please|oomu)\b[,\s:]*",
            r"(?i)^(?:can|could|would|will)\s+you\b[,\s:]*",
            r"(?i)^why\s+don['’]?t\s+you\b[,\s:]*",
            r"(?i)^i(?:'d|\s+would)\s+like\s+you\s+(?:to\s+)?",
            r"(?i)^i\s+(?:want|need)\s+you\s+(?:to\s+)?",
            r"(?i)^i\s+was\s+hoping\s+you\s+(?:could|would)\s+",
            r"(?i)^go\s+ahead\s+and\s+",
        ] {
            let regex = Regex::new(pattern).expect("static courtesy regex is valid");
            value = regex.replace(&value, "").trim().to_string();
        }
        if value == original {
            return value;
        }
    }
}

fn clean_search_topic(value: &str) -> String {
    clean_text(value)
        .trim_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '\"' | '\'' | '`' | ',' | '.' | ':' | ';' | '!' | '?'
                )
        })
        .trim()
        .to_string()
}

pub(crate) fn explicit_external_search_requested(query: &str) -> bool {
    authorization_policy::localized_explicit_search_query(query).is_some()
        || authorization_policy::explicit_external_search_requested(query)
        || continuation::authorized_browser_research_query(query).is_some()
}

fn freshness_search_requested(query: &str) -> bool {
    authorization_policy::freshness_search_requested(query)
}

fn localized_private_search_target(query: &str) -> bool {
    authorization_policy::localized_private_search_target(query)
}

pub(crate) fn independent_public_research_query_allowed(objective: &str, query: &str) -> bool {
    authorization_policy::independent_public_research_query_allowed(objective, query)
}

async fn session_web_grounding_override(
    session_id: Option<&str>,
    persistence: Option<PersistenceEngine>,
) -> Result<Option<bool>, String> {
    let Some(session_id) = session_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let Some(persistence) = persistence else {
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

fn disabled_search_response(
    request: &SovereignSearchExecutionRequest,
    started_at: Instant,
) -> SovereignSearchResponse {
    search_response(
        truncate_chars(&clean_text(&request.query), MAX_QUERY_CHARS),
        started_at,
        DUCKDUCKGO_SEARCH_ENGINE,
        Vec::new(),
        Vec::new(),
        true,
        Some(SEARCH_NOT_AUTHORIZED),
        false,
    )
}

fn search_response(
    query: String,
    started_at: Instant,
    engine: &str,
    mut results: Vec<SovereignSearchResult>,
    mut pages: Vec<crate::dom_streaming::DomContext>,
    mut degraded: bool,
    mut error_code: Option<&str>,
    browser_automation_enabled: bool,
) -> SovereignSearchResponse {
    let accessed_at_utc = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let has_usable_evidence = pages
        .iter()
        .any(crate::dom_streaming::context_has_bounded_search_evidence);
    if !degraded && !has_usable_evidence {
        degraded = true;
        error_code = Some(SEARCH_DOM_FAILED);
        results.clear();
        pages.clear();
    }
    let headless_fallback_count = crate::dom_streaming::headless_context_count(&pages);
    let dom_page_count = pages.len();
    let context_json = if results.is_empty() && pages.is_empty() {
        "[]".to_string()
    } else {
        serde_json::to_string(&serde_json::json!({
            "accessedAtUtc": accessed_at_utc,
            "results": results,
            "pages": pages,
        }))
        .unwrap_or_else(|_| "[]".to_string())
    };
    SovereignSearchResponse {
        query,
        engine: engine.to_string(),
        result_count: results.len(),
        results,
        context_json,
        accessed_at_utc,
        retrieval_elapsed_ms: elapsed_ms(started_at),
        dom_page_count,
        headless_fallback_count,
        degraded,
        receipt_digest: None,
        invocation_index: None,
        error_code: error_code.map(str::to_string),
        error: error_code.map(search_error_message).map(str::to_string),
        security: SovereignSearchSecurity {
            api_key_required: false,
            cookies_enabled: false,
            browser_automation_enabled,
            visible_browser_opened: false,
            proxy_environment_enabled: false,
            endpoint_allowlist: ALLOWED_SEARCH_HOSTS
                .iter()
                .map(|host| (*host).to_string())
                .collect(),
        },
    }
}

fn search_error_message(code: &str) -> &'static str {
    match code {
        SEARCH_NOT_AUTHORIZED => WEB_GROUNDING_DISABLED_MESSAGE,
        SEARCH_QUERY_INVALID => {
            "Web search requires a specific topic. No network request was sent."
        }
        _ => "Web search is unavailable right now. Try again.",
    }
}

fn structured_search_error(code: &str) -> String {
    serde_json::json!({
        "code": if matches!(
            code,
            SEARCH_NOT_AUTHORIZED
                | SEARCH_QUERY_INVALID
                | SEARCH_PROVIDER_CHALLENGE
                | SEARCH_PROVIDER_UNAVAILABLE
                | SEARCH_RETRIEVAL_TIMEOUT
                | SEARCH_NO_RESULTS
                | SEARCH_DOM_FAILED
                | SEARCH_CANCELLED
                | SEARCH_UNAVAILABLE
        ) {
            code
        } else {
            SEARCH_UNAVAILABLE
        },
        "message": search_error_message(code),
    })
    .to_string()
}

fn search_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_millis(CONNECT_TIMEOUT_MS))
        .timeout(Duration::from_millis(SEARCH_TIMEOUT_MS))
        .user_agent(search_user_agent())
        .build()
}

fn parse_duckduckgo_lite_results(document: &str, max_results: usize) -> Vec<SovereignSearchResult> {
    let page = Html::parse_document(document);
    let link_selector =
        Selector::parse("a.result-link, a.result__a").expect("static result selector is valid");
    let snippet_selector = Selector::parse(".result-snippet, .result__snippet")
        .expect("static snippet selector is valid");
    let snippets = page
        .select(&snippet_selector)
        .map(element_text)
        .filter(|text| !text.is_empty())
        .map(|text| truncate_chars(&text, MAX_SNIPPET_CHARS))
        .collect::<Vec<_>>();

    let mut results = Vec::new();
    let mut seen_urls = HashSet::new();
    let mut snippet_index = 0;

    for link in page.select(&link_selector) {
        if results.len() >= max_results {
            break;
        }

        let snippet = snippets.get(snippet_index).cloned().unwrap_or_default();
        snippet_index += 1;

        let title = truncate_chars(&element_text(link), MAX_TITLE_CHARS);
        let url = link
            .value()
            .attr("href")
            .and_then(normalize_result_url)
            .map(|url| truncate_chars(&url, MAX_URL_CHARS));

        let Some(url) = url else {
            continue;
        };
        if title.is_empty() || url.is_empty() || !seen_urls.insert(url.clone()) {
            continue;
        }

        results.push(SovereignSearchResult {
            title,
            url,
            snippet,
        });
    }

    results
}

fn duckduckgo_challenge_present(document: &str) -> bool {
    let normalized = document.to_ascii_lowercase();
    normalized.contains("anomaly-modal")
        || normalized.contains("unfortunately, bots use duckduckgo too")
}

fn bing_challenge_present(document: &str) -> bool {
    let normalized = document.to_ascii_lowercase();
    normalized.contains("one last step") && normalized.contains("solve the challenge")
        || normalized.contains("our systems have detected unusual traffic")
}

fn parse_bing_results(document: &str, max_results: usize) -> Vec<SovereignSearchResult> {
    let page = Html::parse_document(document);
    let result_selector = Selector::parse("li.b_algo").expect("static result selector is valid");
    let link_selector = Selector::parse("h2 a").expect("static result link selector is valid");
    let snippet_selector =
        Selector::parse(".b_caption p").expect("static result snippet selector is valid");
    let mut results = Vec::new();
    let mut seen_urls = HashSet::new();

    for result in page.select(&result_selector) {
        if results.len() >= max_results {
            break;
        }
        let Some(link) = result.select(&link_selector).next() else {
            continue;
        };
        let title = truncate_chars(&element_text(link), MAX_TITLE_CHARS);
        let url = link
            .value()
            .attr("href")
            .and_then(normalize_result_url)
            .map(|url| truncate_chars(&url, MAX_URL_CHARS));
        let Some(url) = url else {
            continue;
        };
        if title.is_empty() || !seen_urls.insert(url.clone()) {
            continue;
        }
        let snippet = result
            .select(&snippet_selector)
            .next()
            .map(element_text)
            .map(|text| truncate_chars(&text, MAX_SNIPPET_CHARS))
            .unwrap_or_default();
        results.push(SovereignSearchResult {
            title,
            url,
            snippet,
        });
    }

    results
}

fn element_text(element: ElementRef<'_>) -> String {
    clean_text(&element.text().collect::<Vec<_>>().join(" "))
}

fn clean_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
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

fn elapsed_ms(started_at: Instant) -> u64 {
    let elapsed = started_at.elapsed().as_millis();
    elapsed.min(u128::from(u64::MAX)) as u64
}

fn search_user_agent() -> &'static str {
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
     (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
}

#[cfg(test)]
mod tests;
