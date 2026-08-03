use super::{contract::GRAPH_ROOT, graph_response::parse_json};
use reqwest::blocking::Client;
use serde_json::Value;
use url::Url;

pub(super) fn list_chats(client: &Client, token: &str) -> Result<(Value, bool), String> {
    let mut url = Url::parse(&format!("{GRAPH_ROOT}/me/chats"))
        .map_err(|_| "microsoft_endpoint_invalid".to_string())?;
    url.query_pairs_mut()
        .append_pair("$top", "25")
        .append_pair("$select", "id,topic,chatType,lastUpdatedDateTime,webUrl");
    parse_json(
        client
            .get(url)
            .bearer_auth(token)
            .send()
            .map_err(|_| "microsoft_request_offline".to_string())?,
    )
}

pub(super) fn resolve_site(
    client: &Client,
    token: &str,
    args: &Value,
) -> Result<(Value, bool), String> {
    let raw = args
        .get("siteUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| "microsoft_argument_siteUrl_required".to_string())?;
    let url = site_graph_url(raw)?;
    parse_json(
        client
            .get(url)
            .bearer_auth(token)
            .send()
            .map_err(|_| "microsoft_request_offline".to_string())?,
    )
}

fn site_graph_url(raw: &str) -> Result<Url, String> {
    let site = Url::parse(raw).map_err(|_| "microsoft_argument_siteUrl_invalid".to_string())?;
    let host = site
        .host_str()
        .filter(|host| host.ends_with(".sharepoint.com") || host.ends_with(".sharepointonline.com"))
        .ok_or_else(|| "microsoft_argument_siteUrl_invalid".to_string())?;
    if site.scheme() != "https"
        || site.username() != ""
        || site.password().is_some()
        || site.port().is_some()
        || site.query().is_some()
        || site.fragment().is_some()
    {
        return Err("microsoft_argument_siteUrl_invalid".to_string());
    }
    let segments = site
        .path_segments()
        .ok_or_else(|| "microsoft_argument_siteUrl_invalid".to_string())?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty()
        || segments.len() > 32
        || segments.iter().any(|segment| {
            segment.len() > 255 || *segment == "." || *segment == ".." || segment.contains(':')
        })
    {
        return Err("microsoft_argument_siteUrl_invalid".to_string());
    }
    let mut graph = Url::parse(GRAPH_ROOT).map_err(|_| "microsoft_endpoint_invalid".to_string())?;
    {
        let mut path = graph
            .path_segments_mut()
            .map_err(|_| "microsoft_endpoint_invalid".to_string())?;
        path.pop_if_empty().push("sites").push(&format!("{host}:"));
        for segment in segments {
            path.push(segment);
        }
    }
    graph
        .query_pairs_mut()
        .append_pair("$select", "id,displayName,name,webUrl,lastModifiedDateTime");
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharepoint_site_resolution_accepts_only_exact_https_tenant_urls() {
        let valid = site_graph_url("https://tenant.sharepoint.com/sites/Finance").unwrap();
        assert!(valid
            .as_str()
            .contains("/sites/tenant.sharepoint.com:/sites/Finance"));
        assert!(site_graph_url("https://evil.example/sites/Finance").is_err());
        assert!(site_graph_url("https://tenant.sharepoint.com/").is_err());
        assert!(site_graph_url("https://tenant.sharepoint.com/sites/Finance?token=x").is_err());
    }
}
