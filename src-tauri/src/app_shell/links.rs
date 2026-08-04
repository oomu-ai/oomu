use tauri_plugin_opener::OpenerExt;

const OOMU_MARKETPLACE_URL: &str = "https://oomu.io/";
const OOMU_PRIVACY_POLICY_URL: &str = "https://oomu.ai/privacy.html";
const MAX_EXTERNAL_HTTP_URL_BYTES: usize = 8 * 1024;

fn validated_external_http_url(raw: &str) -> Result<url::Url, &'static str> {
    if raw.is_empty() || raw.len() > MAX_EXTERNAL_HTTP_URL_BYTES || raw.trim() != raw {
        return Err("external_url_invalid");
    }
    let url = url::Url::parse(raw).map_err(|_| "external_url_invalid")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("external_url_unsafe_scheme");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("external_url_credentials_blocked");
    }
    Ok(url)
}

#[tauri::command]
pub(crate) fn open_external_http_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let url = validated_external_http_url(&url).map_err(str::to_string)?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| "external_url_open_failed".to_string())
}

#[tauri::command]
pub(crate) fn open_oomu_privacy_policy(app: tauri::AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(OOMU_PRIVACY_POLICY_URL, None::<&str>)
        .map_err(|error| format!("Unable to open the fixed OOMU privacy policy URL: {error}"))
}

#[tauri::command]
pub(crate) fn open_oomu_marketplace(app: tauri::AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(OOMU_MARKETPLACE_URL, None::<&str>)
        .map_err(|error| format!("Unable to open the fixed OOMU marketplace URL: {error}"))
}

#[cfg(test)]
mod tests {
    use super::validated_external_http_url;

    #[test]
    fn external_browser_links_accept_only_credential_free_http_urls() {
        assert!(validated_external_http_url("https://example.com/source?q=oomu#result").is_ok());
        assert!(validated_external_http_url("http://example.com/").is_ok());
        for blocked in [
            "javascript:alert(1)",
            "file:///tmp/private",
            "mailto:user@example.com",
            "https://user:secret@example.com/",
            " https://example.com/",
            "https:///",
        ] {
            assert!(validated_external_http_url(blocked).is_err(), "{blocked}");
        }
    }
}
