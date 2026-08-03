use reqwest::{
    blocking::Client,
    redirect::{Attempt, Policy},
};
use std::time::Duration;
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const AUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const GRAPH_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_GRAPH_REDIRECTS: usize = 3;

fn client(timeout: Duration, redirects: Policy) -> Result<Client, String> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        .redirect(redirects)
        .build()
        .map_err(|_| "microsoft_http_client_unavailable".to_string())
}

pub(super) fn auth_client() -> Result<Client, String> {
    client(AUTH_REQUEST_TIMEOUT, Policy::none())
}

fn permitted_graph_redirect(url: &Url) -> bool {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return false;
    }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    host == "graph.microsoft.com"
        || host.ends_with(".sharepoint.com")
        || host.ends_with(".sharepointonline.com")
        || host.ends_with(".1drv.com")
}

fn graph_redirect_policy(attempt: Attempt<'_>) -> reqwest::redirect::Action {
    if attempt.previous().len() >= MAX_GRAPH_REDIRECTS || !permitted_graph_redirect(attempt.url()) {
        attempt.stop()
    } else {
        attempt.follow()
    }
}

pub(super) fn graph_client() -> Result<Client, String> {
    client(GRAPH_REQUEST_TIMEOUT, Policy::custom(graph_redirect_policy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_microsoft_client_has_finite_connect_and_request_timeouts() {
        assert!(CONNECT_TIMEOUT <= Duration::from_secs(5));
        assert!(AUTH_REQUEST_TIMEOUT <= Duration::from_secs(20));
        assert!(GRAPH_REQUEST_TIMEOUT <= Duration::from_secs(60));
        assert!(auth_client().is_ok());
        assert!(graph_client().is_ok());
    }

    #[test]
    fn graph_redirects_are_limited_to_declared_microsoft_file_hosts() {
        for allowed in [
            "https://graph.microsoft.com/v1.0/me",
            "https://tenant.sharepoint.com/download",
            "https://tenant.sharepointonline.com/download",
            "https://public.dm.files.1drv.com/download",
        ] {
            assert!(permitted_graph_redirect(&Url::parse(allowed).unwrap()));
        }
        for rejected in [
            "http://tenant.sharepoint.com/download",
            "https://sharepoint.com.attacker.example/download",
            "https://graph.microsoft.com.attacker.example/v1.0/me",
            "https://example.com/download",
        ] {
            assert!(!permitted_graph_redirect(&Url::parse(rejected).unwrap()));
        }
    }
}
