use super::*;

#[derive(Deserialize)]
struct LocaleSearchVector {
    locale: String,
    public: String,
    ordinary: String,
    freshness: String,
    private: String,
    pronoun: String,
    mixed: String,
}

#[derive(Deserialize)]
struct AuthorityVector {
    id: String,
    utterance: String,
    explicit: bool,
    authorized: bool,
    query: Option<String>,
}

fn authority_vectors() -> Vec<AuthorityVector> {
    serde_json::from_str(include_str!(
        "../../../../src/lib/searchAuthorization/search-authority-vectors.json"
    ))
    .expect("the shared search authority vectors must remain valid JSON")
}

#[test]
fn native_authorization_matches_the_shared_authority_corpus() {
    for vector in authority_vectors() {
        assert_eq!(
            explicit_external_search_requested(&vector.utterance),
            vector.explicit,
            "{} explicit decision",
            vector.id
        );
        assert_eq!(
            explicit_search_query_from_utterance(&vector.utterance),
            vector.query,
            "{} query extraction",
            vector.id
        );
        let query = vector.query.as_deref().unwrap_or(vector.utterance.as_str());
        let authorization = SovereignSearchAuthorization {
            source: SovereignSearchAuthorizationSource::DirectUserUtterance {
                originating_utterance: vector.utterance.clone(),
                mod_id: None,
            },
        };
        assert_eq!(
            search_boundary_authorization(vector.explicit, &authorization, query).is_some(),
            vector.authorized,
            "{} native authorization",
            vector.id
        );
    }
}

#[tokio::test]
async fn sprint_304_ordinary_spotlight_request_binds_one_headless_query() {
    let utterance = "Look online for Apple’s current macOS support page about Spotlight and give me the page title and link.";
    let query = explicit_search_query_from_utterance(utterance)
        .expect("the explicit public request must bind a search query");
    assert_eq!(query, "Apple’s current macOS support page about Spotlight");

    let request = SovereignSearchExecutionRequest::direct(SovereignSearchRequest {
        query,
        originating_utterance: utterance.to_string(),
        max_results: Some(5),
        session_id: None,
        mod_id: None,
        origin_turn_id: None,
        origin_generation_token: None,
    });
    assert!(web_grounding_authorization(&request, None, None)
        .await
        .expect("explicit headless authorization resolves")
        .is_some());
}

#[test]
fn native_extracts_two_objective_bound_release_queries() {
    let objective = "Go online and research the latest stable releases of Rust and Node.js from their official websites. Search each separately, compare their release dates, and cite both official sources.";
    let queries = explicit_search_queries_from_utterance(objective);
    assert_eq!(
        queries,
        vec![
            "latest stable Rust release date official website",
            "latest stable Node.js release date official website",
        ]
    );
    let authorization = SovereignSearchAuthorization {
        source: SovereignSearchAuthorizationSource::DirectUserUtterance {
            originating_utterance: objective.to_string(),
            mod_id: None,
        },
    };
    assert!(queries
        .iter()
        .all(|query| search_boundary_authorization(true, &authorization, query).is_some()));
}

#[test]
fn native_allows_version_bound_release_notes_refinement() {
    let objective = "I'm trying to decide whether it's worth updating Rust right now. Could you look online to find the latest stable Rust release, then check the official release notes for that exact version and tell me whether it includes any newly stabilized language features? Give me a short recommendation with the version, release date, one example if there is one, and links to the official pages you used.";
    assert!(authorization_policy::objective_bound_refined_query_allowed(
        objective,
        "Rust 1.97.1 official release notes",
    ));
    assert!(
        !authorization_policy::objective_bound_refined_query_allowed(
            objective,
            "Node.js 24.0.0 official release notes",
        )
    );
}

#[test]
fn approved_mcp_search_binds_the_exact_shield_approved_query() {
    let objective = "What’s the latest edition of the book “Writing AI Prompts for Dummies”?";
    let query = "latest edition Writing AI Prompts for Dummies";
    let authorization =
        SovereignSearchAuthorization::approved_mcp_tool_call("mcp-audit-1", objective, query);

    assert!(search_boundary_authorization(true, &authorization, query).is_some());
    assert!(search_boundary_authorization(
        true,
        &authorization,
        "latest edition Unrelated Private Book",
    )
    .is_none());
}

#[test]
fn approved_mcp_search_accepts_public_refinements_without_language_keywords() {
    for (objective, query) in [
        (
            "What’s the latest edition of Writing AI Prompts for Dummies?",
            "Writing AI Prompts for Dummies latest edition reviews 2026",
        ),
        (
            "¿Cuál es la edición más reciente de Writing AI Prompts for Dummies?",
            "Writing AI Prompts for Dummies edición más reciente sitio oficial 2026",
        ),
        (
            "Writing AI Prompts for Dummies の最新版は何ですか？",
            "Writing AI Prompts for Dummies 最新版 公式サイト 2026",
        ),
    ] {
        let authorization = SovereignSearchAuthorization::approved_mcp_tool_call(
            "mcp-audit-public",
            objective,
            query,
        );
        assert!(search_boundary_authorization(true, &authorization, query).is_some());
    }
}

#[test]
fn approved_mcp_search_cannot_reframe_private_app_data_as_public_search() {
    let objective = "What is on my calendar today?";
    let query = "my calendar today";
    let authorization =
        SovereignSearchAuthorization::approved_mcp_tool_call("mcp-audit-2", objective, query);

    assert!(search_boundary_authorization(true, &authorization, query).is_none());
}

#[test]
fn native_preserves_date_less_separate_release_queries() {
    let objective = "Go online and research the latest stable releases of Rust and Node.js from their official websites. Search each separately and cite both official sources.";
    assert_eq!(
        explicit_search_queries_from_utterance(objective),
        vec![
            "latest stable release of Rust official website",
            "latest stable release of Node.js official website",
        ]
    );
}

fn locale_search_vectors() -> Vec<LocaleSearchVector> {
    serde_json::from_str(include_str!(
        "../../../../src/lib/__tests__/fixtures/search-intent/locale-matrix.json"
    ))
    .expect("the shared localized search matrix must remain valid JSON")
}

#[tokio::test]
async fn native_authorization_matches_the_shared_localized_search_matrix() {
    for vector in locale_search_vectors() {
        let public_query = explicit_search_query_from_utterance(&vector.public)
            .unwrap_or_else(|| panic!("{} public directive was not parsed", vector.locale));
        let public_request = SovereignSearchExecutionRequest::direct(SovereignSearchRequest {
            query: public_query,
            originating_utterance: vector.public.clone(),
            max_results: Some(5),
            session_id: None,
            mod_id: None,
            origin_turn_id: None,
            origin_generation_token: None,
        });
        assert!(
            web_grounding_authorization(&public_request, None, None)
                .await
                .unwrap()
                .is_some(),
            "{} explicit public search must bypass the ambient switch",
            vector.locale
        );

        for denied in [
            &vector.ordinary,
            &vector.private,
            &vector.pronoun,
            &vector.mixed,
        ] {
            let authorization = SovereignSearchAuthorization {
                source: SovereignSearchAuthorizationSource::DirectUserUtterance {
                    originating_utterance: denied.clone(),
                    mod_id: None,
                },
            };
            assert!(
                search_boundary_authorization(true, &authorization, denied).is_none(),
                "{} must deny non-public vector: {}",
                vector.locale,
                denied
            );
        }

        let freshness = SovereignSearchAuthorization {
            source: SovereignSearchAuthorizationSource::DirectUserUtterance {
                originating_utterance: vector.freshness.clone(),
                mod_id: None,
            },
        };
        assert!(
            search_boundary_authorization(false, &freshness, &vector.freshness).is_none(),
            "{} freshness must stay offline with ambient Search off",
            vector.locale
        );
        assert!(
            search_boundary_authorization(true, &freshness, &vector.freshness).is_some(),
            "{} freshness must bind its exact utterance with ambient Search on",
            vector.locale
        );
        assert!(
            search_boundary_authorization(true, &freshness, "different current topic").is_none(),
            "{} freshness authority must not widen to another query",
            vector.locale
        );
    }
}

#[test]
fn parser_extracts_titles_urls_and_snippets() {
    let results = parse_duckduckgo_lite_results(SEARCH_RESULT_FIXTURE, MAX_RESULTS);

    assert_eq!(
        results,
        vec![
            SovereignSearchResult {
                title: "Alpha Result".to_string(),
                url: "https://example.com/alpha?q=1".to_string(),
                snippet: "First public result snippet.".to_string(),
            },
            SovereignSearchResult {
                title: "Beta Result".to_string(),
                url: "https://example.org/beta".to_string(),
                snippet: "Second snippet with extra whitespace.".to_string(),
            },
            SovereignSearchResult {
                title: "Gamma Result".to_string(),
                url: "https://example.net/gamma".to_string(),
                snippet: "Third lite endpoint snippet.".to_string(),
            },
        ]
    );
}

#[test]
fn parser_caps_result_count_and_deduplicates_urls() {
    let duplicate_fixture = format!(
            "{SEARCH_RESULT_FIXTURE}<a class='result-link' href='https://example.net/gamma'>Duplicate</a>"
        );

    let results = parse_duckduckgo_lite_results(&duplicate_fixture, 2);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].title, "Alpha Result");
    assert_eq!(results[1].title, "Beta Result");
}

#[test]
fn query_shaped_urls_and_substrings_cannot_authenticate_generic_content() {
    let generic_page = crate::dom_streaming::DomContext {
        url: "https://www.kayak.com/flights/ROC-SIN/2027-03-14/2027-03-21".to_string(),
        title: "ROC to SIN | Flight search".to_string(),
        visible_text: "Baggage fees may apply. Hacker Fares combine separate tickets.".to_string(),
        inputs: Vec::new(),
        buttons: Vec::new(),
        links: Vec::new(),
        tables: Vec::new(),
        temporal_evidence: Vec::new(),
        extraction_method: "static_html".to_string(),
    };
    let generic_result = SovereignSearchResult {
        title: "Using flight search".to_string(),
        url: "https://www.kayak.com/flights/ROC-SIN/2027-03-14/2027-03-21".to_string(),
        snippet: "Baggage fees may apply. Hacker Fares combine separate tickets.".to_string(),
    };
    let query = "travel best flight from ROC to SIN on March 14 2027 returning March 21 2027";

    assert!(!mod_context_has_query_evidence(
        query,
        &[generic_result],
        &[generic_page]
    ));
}

#[test]
fn privacy_gate_never_sends_private_app_requests_to_search() {
    for (originating_utterance, query) in [
        (
            "Search the web for what is in my calendar tomorrow",
            "events tomorrow",
        ),
        ("Search online for my unread email", "unread messages"),
        (
            "Search Google for the newest photo in my photo library",
            "newest image",
        ),
        ("Search online for my contacts", "contacts"),
        ("Search online for my iMessages", "my iMessages"),
        (
            "Search the web for my recently added Apple Music songs",
            "recently added songs",
        ),
    ] {
        let authorization = SovereignSearchAuthorization {
            source: SovereignSearchAuthorizationSource::DirectUserUtterance {
                originating_utterance: originating_utterance.to_string(),
                mod_id: None,
            },
        };
        assert!(
            search_boundary_authorization(true, &authorization, query).is_none(),
            "{originating_utterance}"
        );
    }
}
