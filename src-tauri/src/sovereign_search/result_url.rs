use base64::{
    engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD},
    Engine as _,
};

pub(super) fn normalize_result_url(raw_href: &str) -> Option<String> {
    let href = raw_href.trim();
    if href.is_empty() {
        return None;
    }

    let absolute_href = if href.starts_with("//") {
        format!("https:{href}")
    } else if href.starts_with('/') {
        format!("https://duckduckgo.com{href}")
    } else {
        href.to_string()
    };

    let mut url = reqwest::Url::parse(&absolute_href).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    if matches!(host.as_str(), "bing.com" | "www.bing.com") && url.path().starts_with("/ck/a") {
        return normalize_result_url(&decode_bing_redirect(&url)?);
    }
    if super::ALLOWED_SEARCH_HOSTS.contains(&host.as_str()) && url.path().starts_with("/l/") {
        let target = url
            .query_pairs()
            .find_map(|(key, value)| (key == "uddg").then(|| value.into_owned()))?;
        return normalize_result_url(&target);
    }

    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
}

fn decode_bing_redirect(url: &reqwest::Url) -> Option<String> {
    let encoded = url
        .query_pairs()
        .find_map(|(key, value)| (key == "u").then(|| value.into_owned()))?;
    let payload = encoded.strip_prefix("a1")?;
    URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| STANDARD_NO_PAD.decode(payload))
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}
