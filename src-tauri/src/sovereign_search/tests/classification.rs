use super::*;

#[test]
fn headless_mod_context_requires_task_specific_query_evidence() {
    let generic_landing_page = crate::dom_streaming::DomContext {
        url: "https://www.google.com/travel/flights".to_string(),
        title: "Google Flights".to_string(),
        visible_text: "Search destinations and discover popular domestic trips.".to_string(),
        inputs: Vec::new(),
        buttons: vec!["Explore destinations".to_string()],
        links: Vec::new(),
        tables: Vec::new(),
        temporal_evidence: Vec::new(),
        extraction_method: "static_http".to_string(),
    };
    let itinerary_page = crate::dom_streaming::DomContext {
        url: "https://www.kayak.com/flights/ROC-SIN/2027-03-14/2027-03-21".to_string(),
        title: "ROC to SIN".to_string(),
        visible_text: "Rochester to Singapore, March 14 through March 21, 2027.".to_string(),
        ..generic_landing_page.clone()
    };
    let query =
        "travel I need the best flight from ROC to SIN on March 14 2027 returning March 21 2027";

    assert!(!mod_context_has_query_evidence(
        query,
        &[],
        &[generic_landing_page]
    ));
    assert!(mod_context_has_query_evidence(
        query,
        &[],
        &[itinerary_page]
    ));
}

#[test]
fn query_evidence_must_coexist_in_one_result_or_page() {
    let results = vec![
        SovereignSearchResult {
            title: "Departing ROC".to_string(),
            url: "https://travel.example/one".to_string(),
            snippet: "Origin details".to_string(),
        },
        SovereignSearchResult {
            title: "Arriving SIN".to_string(),
            url: "https://travel.example/two".to_string(),
            snippet: "Destination details".to_string(),
        },
    ];

    assert!(!mod_context_has_query_evidence(
        "travel ROC to SIN",
        &results,
        &[]
    ));
}

#[test]
fn direct_search_boundary_binds_the_query_to_the_explicit_utterance() {
    for (originating_utterance, query) in [
        (
            "Search online for Blackpink tour dates",
            "Blackpink tour dates",
        ),
        (
            "Please search Google for \"OOMU privacy policy\"",
            "OOMU privacy policy",
        ),
        (
            "Use the internet to find Rust 2.0 release notes",
            "Rust 2.0 release notes",
        ),
        (
            "Look Blackpink tour dates up online",
            "Blackpink tour dates",
        ),
    ] {
        let authorization = SovereignSearchAuthorization {
            source: SovereignSearchAuthorizationSource::DirectUserUtterance {
                originating_utterance: originating_utterance.to_string(),
                mod_id: None,
            },
        };
        assert!(
            search_boundary_authorization(true, &authorization, query).is_some(),
            "{originating_utterance}"
        );
    }

    let authorization = SovereignSearchAuthorization {
        source: SovereignSearchAuthorizationSource::DirectUserUtterance {
            originating_utterance: "Search online for Blackpink tour dates".to_string(),
            mod_id: None,
        },
    };
    assert!(
        search_boundary_authorization(true, &authorization, "private account records").is_none()
    );

    let contextual = SovereignSearchAuthorization {
        source: SovereignSearchAuthorizationSource::DirectUserUtterance {
            originating_utterance: "Search online for that".to_string(),
            mod_id: None,
        },
    };
    assert!(search_boundary_authorization(true, &contextual, "Blackpink tour dates").is_none());
}
