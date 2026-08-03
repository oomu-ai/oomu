use super::{
    bing_challenge_present, duckduckgo_challenge_present, fetch_bing_html,
    fetch_duckduckgo_lite_html, parse_bing_results, parse_duckduckgo_lite_results,
    provider_terminal_code, ProviderAttempt, SovereignSearchResult, BING_SEARCH_ENGINE,
    DUCKDUCKGO_SEARCH_ENGINE,
};

pub(super) async fn fetch_ranked_provider_results(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Result<(&'static str, Vec<SovereignSearchResult>), &'static str> {
    let primary = duckduckgo_attempt(client, query, max_results).await;
    if let ProviderAttempt::Results(results) = primary {
        return Ok((DUCKDUCKGO_SEARCH_ENGINE, results));
    }

    let fallback = bing_attempt(client, query, max_results).await;
    if let ProviderAttempt::Results(results) = fallback {
        return Ok((BING_SEARCH_ENGINE, results));
    }
    Err(provider_terminal_code(&primary, &fallback))
}

async fn duckduckgo_attempt(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> ProviderAttempt {
    match fetch_duckduckgo_lite_html(client, query).await {
        Ok(document) if !duckduckgo_challenge_present(&document) => {
            parsed_attempt(parse_duckduckgo_lite_results(&document, max_results))
        }
        Ok(_) => {
            eprintln!("SOVEREIGN_SEARCH_DUCKDUCKGO_CHALLENGE");
            ProviderAttempt::Challenge
        }
        Err(error) => {
            eprintln!(
                "SOVEREIGN_SEARCH_DUCKDUCKGO_UNAVAILABLE reason={}",
                error.observability_code()
            );
            error.provider_attempt()
        }
    }
}

async fn bing_attempt(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> ProviderAttempt {
    match fetch_bing_html(client, query).await {
        Ok(document) if !bing_challenge_present(&document) => {
            parsed_attempt(parse_bing_results(&document, max_results))
        }
        Ok(_) => {
            eprintln!("SOVEREIGN_SEARCH_BING_CHALLENGE");
            ProviderAttempt::Challenge
        }
        Err(error) => {
            eprintln!(
                "SOVEREIGN_SEARCH_BING_UNAVAILABLE reason={}",
                error.observability_code()
            );
            error.provider_attempt()
        }
    }
}

fn parsed_attempt(results: Vec<SovereignSearchResult>) -> ProviderAttempt {
    if results.is_empty() {
        ProviderAttempt::NoResults
    } else {
        ProviderAttempt::Results(results)
    }
}
