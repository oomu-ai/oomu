use super::*;

#[tokio::test]
async fn sprint_304_receipt_backed_spotlight_completion_stays_on_e4b() {
    let prompt = concat!(
        "Look online for Apple’s current macOS support page about Spotlight and give me the page title and link.\n\n",
        "Local Web Search Context\n",
        "Query: Apple’s current macOS support page about Spotlight\n",
        "Native-Receipt: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        "Invocation-Index: 1\n",
        "Result-Count: 1\n\n",
        "{\"pages\":[{\"title\":\"Search for anything with Spotlight on Mac\",",
        "\"url\":\"https://support.apple.com/guide/mac-help/search-with-spotlight-mchlp1008/mac\"}]}\n\n",
        "Verified-Native-Public-Grounding: true"
    );
    assert!(objective_policy::deterministic_hydrated_public_grounding_applies(prompt));
    assert!(
        !objective_policy::deterministic_hydrated_public_grounding_applies(
            "I will look online and give you the page title and link."
        )
    );

    let gemma = GemmaService::new_disabled("classifier unavailable by test contract");
    let cloud = ConfiguredCloudRouteSnapshot {
        provider_id: "prov-google-gemini".to_string(),
        model_id: Some("gemini-3.6-flash".to_string()),
        provider_name: "Google Gemini".to_string(),
        credential_configured: true,
    };
    let route = resolve_dynamic_model_route_with_frozen_cloud(
        &gemma,
        prompt,
        "local_model",
        "gemma-4-E4B-it-qat-q4_0-gguf",
        Some(&cloud),
    )
    .await
    .expect("receipt-backed evidence bypasses the disabled classifier");

    assert_eq!(route.provider_id, "local_model");
    assert_eq!(route.model_id, "gemma-4-E4B-it-qat-q4_0-gguf");
    assert_eq!(route.tier, "local_tier_1");
    assert_eq!(
        route.classifier_source,
        objective_policy::HYDRATED_PUBLIC_GROUNDING_POLICY_VERSION
    );
    assert_eq!(route.classifier_latency_ms, 0);
    assert!(!route.recovery_attempted);
    assert!(route
        .reason
        .contains("verified native public-search evidence"));
}
