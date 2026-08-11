use super::*;

#[test]
fn result_only_context_cannot_claim_grounded_success() {
    let response = search_response(
        "rust stable".to_string(),
        Instant::now(),
        DUCKDUCKGO_SEARCH_ENGINE,
        vec![SovereignSearchResult {
            title: "Rust".to_string(),
            url: "https://www.rust-lang.org/".to_string(),
            snippet: "A language empowering everyone.".to_string(),
        }],
        Vec::new(),
        false,
        None,
        true,
    );

    assert!(response.degraded);
    assert_eq!(response.error_code.as_deref(), Some(SEARCH_DOM_FAILED));
    assert_eq!(response.context_json, "[]");
}

#[test]
fn verified_source_page_context_is_a_grounded_success() {
    let page = crate::dom_streaming::DomContext {
        url: "https://www.rust-lang.org/learn".to_string(),
        title: "Learn Rust".to_string(),
        visible_text: "Rust is a programming language focused on performance, type safety, and productivity. Its official learning resources include the Rust Book, examples, standard-library documentation, and practical tools for new and experienced developers.".to_string(),
        inputs: Vec::new(),
        buttons: Vec::new(),
        links: Vec::new(),
        tables: Vec::new(),
        temporal_evidence: Vec::new(),
        extraction_method: "static_html".to_string(),
    };
    let response = search_response(
        "rust stable".to_string(),
        Instant::now(),
        DUCKDUCKGO_SEARCH_ENGINE,
        vec![SovereignSearchResult {
            title: "Rust".to_string(),
            url: "https://www.rust-lang.org/learn".to_string(),
            snippet: "The official Rust learning resources.".to_string(),
        }],
        vec![page],
        false,
        None,
        true,
    );

    assert!(!response.degraded);
    assert_eq!(response.error_code, None);
    assert!(response
        .context_json
        .contains("official Rust learning resources"));
    let context: serde_json::Value = serde_json::from_str(&response.context_json).unwrap();
    assert_eq!(
        context
            .get("accessedAtUtc")
            .and_then(serde_json::Value::as_str),
        Some(response.accessed_at_utc.as_str())
    );
    assert!(response.accessed_at_utc.ends_with('Z'));
    assert!(chrono::DateTime::parse_from_rfc3339(&response.accessed_at_utc).is_ok());
}

#[test]
fn official_release_query_qualifies_the_official_subject_homepage() {
    let requirement = ModQueryEvidence::from_query(
        "latest stable Rust release date official website",
        Vec::new(),
    );
    let page = crate::dom_streaming::DomContext {
        url: "https://rust-lang.org/".to_string(),
        title: "Rust Programming Language".to_string(),
        visible_text: "Rust is a language empowering everyone to build reliable and efficient software. Version 1.97.1 is available now.".to_string(),
        inputs: Vec::new(),
        buttons: Vec::new(),
        links: Vec::new(),
        tables: Vec::new(),
        temporal_evidence: Vec::new(),
        extraction_method: "static_html".to_string(),
    };

    assert_eq!(requirement.tokens, vec!["rust"]);
    assert_eq!(requirement.minimum_matches, 1);
    assert!(requirement.matches_page(&page));
}

#[test]
fn duckduckgo_challenge_is_not_treated_as_an_empty_success() {
    assert!(duckduckgo_challenge_present(
        r#"<div class="anomaly-modal">Unfortunately, bots use DuckDuckGo too.</div>"#
    ));
    assert!(!duckduckgo_challenge_present(SEARCH_RESULT_FIXTURE));
}

#[test]
fn bing_fallback_parser_decodes_targets_and_extracts_snippets() {
    let fixture = r#"
        <ol id="b_results">
          <li class="b_algo">
            <h2><a href="https://www.bing.com/ck/a?u=a1aHR0cHM6Ly93d3cuZ29vZ2xlLmNvbS8">Google Flights</a></h2>
            <div class="b_caption"><p>Compare public flight options.</p></div>
          </li>
        </ol>
        "#;
    assert_eq!(
        parse_bing_results(fixture, MAX_RESULTS),
        vec![SovereignSearchResult {
            title: "Google Flights".to_string(),
            url: "https://www.google.com/".to_string(),
            snippet: "Compare public flight options.".to_string(),
        }]
    );
}

#[test]
fn declared_content_evidence_patterns_use_all_semantics() {
    let requirement = ModQueryEvidence::from_query(
        "travel best flight from ROC to SIN",
        vec![
            Regex::new(r"\$\s*\d{2,}").unwrap(),
            Regex::new(r"(?i)\b(?:nonstop|\d+\s+stops?|\d{1,2}:\d{2}\s*(?:am|pm)?)\b").unwrap(),
        ],
    );
    let mut page = crate::dom_streaming::DomContext {
        url: "https://travel.example/flights/ROC-SIN".to_string(),
        title: "ROC to SIN | Flight search".to_string(),
        visible_text: "ROC to SIN. Baggage fees may apply. Hacker Fares combine separate tickets."
            .to_string(),
        inputs: Vec::new(),
        buttons: Vec::new(),
        links: Vec::new(),
        tables: Vec::new(),
        temporal_evidence: Vec::new(),
        extraction_method: "static_html".to_string(),
    };

    assert!(!requirement.matches_page(&page));
    page.visible_text = "ROC to SIN · Singapore Airlines · $850 round trip".to_string();
    assert!(!requirement.matches_page(&page));
    page.visible_text =
        "ROC to SIN · Singapore Airlines · 6:10 pm · 1 stop · $850 round trip".to_string();
    assert!(requirement.matches_page(&page));
}

#[test]
fn headless_mod_context_supports_future_mods_with_a_single_specific_anchor() {
    let result = SovereignSearchResult {
        title: "AAPL quote".to_string(),
        url: "https://markets.example.com/AAPL".to_string(),
        snippet: "Current public market facts for AAPL.".to_string(),
    };

    assert!(mod_context_has_query_evidence("quote AAPL", &[result], &[]));
}

#[test]
fn bing_challenge_is_not_treated_as_a_source_response() {
    assert!(bing_challenge_present(
        "One last step. Please solve the challenge below to continue."
    ));
    assert!(bing_challenge_present(
        "Our systems have detected unusual traffic from your network."
    ));
    assert!(!bing_challenge_present(
        "Compare public flight options with current schedules."
    ));
}

#[test]
fn privacy_gate_uses_session_override_or_global_default() {
    assert!(!effective_web_grounding_enabled(false, None));
    assert!(effective_web_grounding_enabled(false, Some(true)));
    assert!(!effective_web_grounding_enabled(true, Some(false)));
    assert!(effective_web_grounding_enabled(true, None));
}

#[test]
fn direct_network_mod_commands_use_separate_per_turn_authority() {
    let authorization = SovereignSearchAuthorization {
        source: SovereignSearchAuthorizationSource::DirectUserUtterance {
            originating_utterance: "/research current battery prices".to_string(),
            mod_id: Some("com.example.market_research".to_string()),
        },
    };
    assert_eq!(
        authorization.direct_mod_command(),
        Some((
            "/research current battery prices",
            "com.example.market_research",
        ))
    );

    let ordinary_search = SovereignSearchAuthorization {
        source: SovereignSearchAuthorizationSource::DirectUserUtterance {
            originating_utterance: "Search online for ROC to SIN".to_string(),
            mod_id: None,
        },
    };
    assert_eq!(ordinary_search.direct_mod_command(), None);
    assert!(search_boundary_authorization(false, &ordinary_search, "ROC to SIN").is_none());
}

#[test]
fn direct_search_boundary_requires_consent_and_explicit_originating_utterance() {
    for (consent_enabled, explicit_utterance, expected) in [
        (false, false, false),
        (false, true, false),
        (true, false, false),
        (true, true, true),
    ] {
        let authorization = SovereignSearchAuthorization {
            source: SovereignSearchAuthorizationSource::DirectUserUtterance {
                originating_utterance: if explicit_utterance {
                    "Search online for Blackpink tour dates".to_string()
                } else {
                    "What are the latest Blackpink tour dates?".to_string()
                },
                mod_id: None,
            },
        };
        assert_eq!(
            search_boundary_authorization(consent_enabled, &authorization, "Blackpink tour dates")
                .is_some(),
            expected,
            "consent={consent_enabled} explicit={explicit_utterance}"
        );
    }
}

#[tokio::test]
async fn explicit_public_search_does_not_depend_on_the_ambient_search_switch() {
    let request = SovereignSearchExecutionRequest::direct(SovereignSearchRequest {
        query: "the latest weekly U.S. on-highway diesel fuel price from the official U.S. Energy Information Administration".to_string(),
        originating_utterance: "Search the public web for the latest weekly U.S. on-highway diesel fuel price from the official U.S. Energy Information Administration. Cite the exact source URL and access time.".to_string(),
        max_results: Some(5),
        session_id: None,
        mod_id: None,
        origin_turn_id: None,
        origin_generation_token: None,
    });

    let authorization = web_grounding_authorization(&request, None, None)
        .await
        .expect("explicit authorization should be evaluated without settings storage");

    assert!(authorization.is_some());
}

#[tokio::test]
async fn approved_scenario_research_does_not_depend_on_the_ambient_search_switch() {
    let objective = "Reconcile every quoted amount and margin, identify all exceptions, and independently research current primary or official web sources for fuel or freight conditions.";
    let query = "official fuel conditions";
    let request = SovereignSearchExecutionRequest::approved_action_plan(
        query,
        Some(5),
        None,
        SovereignSearchAuthorization::approved_action_plan("plan-scenario-one", objective, query),
    );

    let authorization = web_grounding_authorization(&request, None, None)
        .await
        .expect("approved-plan authorization should be evaluated without settings storage");

    assert!(authorization.is_some());
}

#[test]
fn explicit_external_search_language_excludes_freshness_and_loose_lookup_verbs() {
    for prompt in [
        "search online for Blackpink tour dates",
        "search the web for Blackpink tour dates",
        "search Google for Blackpink tour dates",
        "use the internet to find Blackpink tour dates",
        "use Google to search for Blackpink tour dates",
        "search DuckDuckGo for Blackpink tour dates",
        "look Blackpink tour dates up online",
    ] {
        assert!(explicit_external_search_requested(prompt), "{prompt}");
    }
    for prompt in [
        "What is the latest Blackpink tour schedule?",
        "Check Blackpink tour dates",
        "Look up Blackpink tour dates",
        "What is current today?",
        "How does web search work?",
        "Did you search online?",
        "Why did you search online?",
        "I did not ask you to search online.",
        "I didn't ask you to search online.",
        "How do I use the internet?",
        "Google search is useful.",
        "Web search is useful.",
        "The phrase search online appears in the UI.",
        "Can we talk about how to search online?",
        "I searched online yesterday.",
        "Please explain how to search online.",
        "Search the repository for Google OAuth code.",
        "Search this document for the word online.",
    ] {
        assert!(!explicit_external_search_requested(prompt), "{prompt}");
    }
}

#[test]
fn natural_web_search_requests_require_an_imperative_and_a_public_surface() {
    for (prompt, expected_query) in [
        (
            "Take a look online and find out the next time the Red Sox are playing in Boston",
            "the next time the Red Sox are playing in Boston",
        ),
        (
            "Have a quick look on the web for affordable hotels in Hershey PA",
            "affordable hotels in Hershey PA",
        ),
        (
            "Run a web search for the latest macOS release",
            "the latest macOS release",
        ),
        (
            "See what you can find on the internet about OOMU public beta",
            "OOMU public beta",
        ),
        ("Search for Boston weather using the web", "Boston weather"),
        (
            "Go on the internet and find the next lunar eclipse",
            "the next lunar eclipse",
        ),
    ] {
        assert!(explicit_external_search_requested(prompt), "{prompt}");
        assert_eq!(
            explicit_search_query_from_utterance(prompt).as_deref(),
            Some(expected_query),
            "{prompt}"
        );
    }

    for prompt in [
        "I take a look online every morning.",
        "Taking a look online is part of my routine.",
        "A web search would help here.",
        "What does 'look online' mean?",
        "Did you search the web already?",
        "Do not look online for this.",
        "Take a look at this document and summarize it.",
        "Search the repository for web search routing.",
        "Check Google Calendar for my meetings.",
    ] {
        assert!(!explicit_external_search_requested(prompt), "{prompt}");
        assert_eq!(
            explicit_search_query_from_utterance(prompt),
            None,
            "{prompt}"
        );
    }
}

#[test]
fn coordinated_independent_web_research_is_an_explicit_directive_not_a_mention() {
    for prompt in [
        "Independently research current primary or official web sources for fuel conditions.",
        "Reconcile every quoted amount and margin, identify all exceptions, and independently research current primary or official web sources for fuel or freight conditions that could materially affect the recommendation.",
        "Please review the supplier data and independently research authoritative web sources for current freight conditions.",
    ] {
        assert!(explicit_external_search_requested(prompt), "{prompt}");
    }

    for prompt in [
        "Did you reconcile every quoted amount and independently research current primary or official web sources?",
        "Why independently research current primary or official web sources?",
        "I did not ask you to independently research current primary or official web sources.",
        "Do not reconcile the amounts and independently research current primary or official web sources.",
        "Reconcile the amounts but do not independently research current primary or official web sources.",
        "The analyst will reconcile the amounts and independently research current primary or official web sources.",
        "The instructions describe how to independently research current primary or official web sources.",
        "We discussed the exceptions and independently research current primary or official web sources.",
        "Reconcile the amounts and independently research whether official web sources are needed.",
        "Reconcile the amounts and independently research the local repository for official web sources.",
    ] {
        assert!(!explicit_external_search_requested(prompt), "{prompt}");
    }
}

#[test]
fn direct_imperative_official_web_research_is_explicit_but_mentions_remain_offline() {
    let authorized = "Research current fuel or freight conditions that could materially affect the recommendation using primary or official web sources.";
    assert!(explicit_external_search_requested(authorized));
    assert!(independent_public_research_query_allowed(
        authorized,
        "official fuel conditions"
    ));

    for prompt in [
        "Did you research current fuel conditions using primary or official web sources?",
        "Do not research current fuel conditions using primary or official web sources.",
        "Research my local files using primary or official web sources.",
        "Research current fuel conditions.",
    ] {
        assert!(!explicit_external_search_requested(prompt), "{prompt}");
    }
}

#[test]
fn named_primary_or_official_sources_are_an_explicit_bounded_research_directive() {
    let scenario_four = "Research current primary or official sources on scheduled/background agent capabilities in OpenClaw and Claude Cowork.";
    assert!(explicit_external_search_requested(scenario_four));
    assert_eq!(
        explicit_search_query_from_utterance(scenario_four).as_deref(),
        Some("scheduled/background agent capabilities in OpenClaw and Claude Cowork")
    );
    assert!(independent_public_research_query_allowed(
        scenario_four,
        "scheduled background agent capabilities OpenClaw Claude Cowork"
    ));

    for prompt in [
        "Did you research current primary or official sources on background agents?",
        "Do not research current primary or official sources on background agents.",
        "Research current primary or official sources in this document.",
        "Research current primary or official sources on my calendar.",
        "Research current sources on background agents.",
        "Research current background-agent capabilities.",
    ] {
        assert!(!explicit_external_search_requested(prompt), "{prompt}");
    }
}

#[test]
fn approved_internal_search_sources_remain_typed_and_boundary_checked() {
    let approved_plan = SovereignSearchAuthorization::approved_action_plan(
        "plan-1",
        "Search the web for current Rust release notes",
        "current Rust release notes",
    );
    assert!(
        search_boundary_authorization(true, &approved_plan, "current Rust release notes").is_some()
    );

    let implicit_plan = SovereignSearchAuthorization::approved_action_plan(
        "plan-2",
        "Find the current Rust release notes",
        "current Rust release notes",
    );
    assert!(
        search_boundary_authorization(true, &implicit_plan, "current Rust release notes").is_none()
    );

    let delegated = SovereignSearchExecutionRequest::approved_delegation(
        "current Rust release notes",
        Some(5),
        "delegation-run-1",
        "Search online for current Rust release notes",
        "current Rust release notes",
    );
    assert!(
        search_boundary_authorization(true, &delegated.authorization, &delegated.query).is_some()
    );
}

#[test]
fn approved_plan_allows_only_the_independent_public_query_in_a_private_app_workflow() {
    let objective = "Read my unread emails and independently research official web sources for public fuel conditions.";
    let public = SovereignSearchAuthorization::approved_action_plan(
        "plan-public",
        objective,
        "public fuel conditions",
    );
    assert!(
        search_boundary_authorization(true, &public, "public fuel conditions").is_some(),
        "a static public query explicitly separated from private work stays usable"
    );

    let substituted = SovereignSearchAuthorization::approved_action_plan(
        "plan-private",
        objective,
        "Acme renewal quote 48291",
    );
    assert!(
        search_boundary_authorization(true, &substituted, "Acme renewal quote 48291").is_none(),
        "private-derived or otherwise unapproved query material cannot cross the boundary"
    );
}

#[test]
fn decision_pack_search_requires_exact_signed_policy_membership_and_host() {
    let objective = "Read /private/tmp/input.json. Independently research current primary or official web sources for fuel or freight conditions.";
    let policy = crate::decision_research_policy::compile_research_policy(objective).unwrap();
    let digest = crate::foundation::digest::sha256_hex(&serde_json::to_vec(&policy).unwrap());
    let query = policy.subjects[1].query_alternatives[0].query.clone();
    let approved = SovereignSearchAuthorization::approved_decision_pack(
        "plan-decision-pack",
        objective,
        &query,
        policy.clone(),
        &digest,
    );
    let verified = search_boundary_authorization(true, &approved, &query)
        .expect("the exact signed query should retain its registered host profile");
    assert_eq!(
        verified.direct_context_urls,
        vec!["https://www.bts.gov/tags/transportation-services-index"]
    );
    assert_eq!(
        verified.direct_context_engine,
        Some(POLICY_REGISTERED_CONTEXT_ENGINE)
    );
    assert!(verified.require_query_evidence);
    assert!(verified.allows_result_url("https://www.bts.gov/newsroom/current-freight-release"));
    assert!(!verified
        .allows_result_url("https://www.bts.gov.example.com/newsroom/current-freight-release"));
    assert!(search_boundary_authorization(
        true,
        &approved,
        "site:bts.gov freight transportation services index latest mutated"
    )
    .is_none());

    let bad_digest = SovereignSearchAuthorization::approved_decision_pack(
        "plan-decision-pack",
        objective,
        &query,
        policy,
        "0".repeat(64),
    );
    assert!(search_boundary_authorization(true, &bad_digest, &query).is_none());
}

#[tokio::test]
#[ignore = "requires live access to the registered EIA public page"]
async fn signed_policy_direct_context_uses_production_dom_streaming() {
    let objective =
        "Independently research current primary or official web sources for fuel conditions.";
    let policy = crate::decision_research_policy::compile_research_policy(objective).unwrap();
    let digest = crate::decision_research_policy::policy_digest(&policy).unwrap();
    let query = policy.subjects[0].query_alternatives[0].query.clone();
    let authorization = SovereignSearchAuthorization::approved_decision_pack(
        "plan-live-eia",
        objective,
        &query,
        policy,
        digest,
    );
    let verified = search_boundary_authorization(true, &authorization, &query)
        .expect("signed EIA query should resolve its registered context");
    let response = search_duckduckgo_lite(
        SovereignSearchExecutionRequest::approved_action_plan(&query, Some(5), None, authorization),
        verified,
        None,
        Instant::now(),
    )
    .await;

    assert_eq!(response.engine, POLICY_REGISTERED_CONTEXT_ENGINE);
    assert!(!response.degraded, "{:?}", response.error);
    assert!(response.dom_page_count >= 1);
    assert!(response
        .results
        .iter()
        .all(|result| result.url.starts_with("https://www.eia.gov/")));
    assert!(response
        .context_json
        .to_ascii_lowercase()
        .contains("diesel"));
}

#[test]
fn disabled_search_response_returns_no_grounding_context() {
    let response = disabled_search_response(
        &SovereignSearchExecutionRequest::direct(SovereignSearchRequest {
            query: " current weather ".to_string(),
            originating_utterance: "What is the current weather?".to_string(),
            max_results: Some(5),
            session_id: None,
            mod_id: None,
            origin_turn_id: None,
            origin_generation_token: None,
        }),
        Instant::now(),
    );

    assert!(response.degraded);
    assert_eq!(response.result_count, 0);
    assert!(response.results.is_empty());
    assert_eq!(response.context_json, "[]");
    assert_eq!(
        response.error.as_deref(),
        Some(WEB_GROUNDING_DISABLED_MESSAGE)
    );
    assert_eq!(response.error_code.as_deref(), Some(SEARCH_NOT_AUTHORIZED));
}
