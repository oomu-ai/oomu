use super::{auth::OAuthAttemptSecret, manifest, microsoft365};
use reqwest::StatusCode;
use serde_json::Value;

type OAuthForm = Vec<(&'static str, String)>;

fn google_client_authentication_required(manifest_id: &str, phase: &str, response: &Value) -> bool {
    if manifest_id != "google_workspace"
        || phase != "token"
        || response.get("error").and_then(Value::as_str) != Some("invalid_request")
    {
        return false;
    }
    response
        .get("error_description")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .is_some_and(|description| {
            description.contains("client_secret") && description.contains("missing")
        })
}

pub(super) fn authorization_code_form(
    secret: &OAuthAttemptSecret,
    code: &str,
) -> Result<OAuthForm, String> {
    authorization_code_form_with_google_credential(
        secret,
        code,
        manifest::google_oauth_client_secret(),
    )
}

fn authorization_code_form_with_google_credential(
    secret: &OAuthAttemptSecret,
    code: &str,
    google_credential: Option<&str>,
) -> Result<OAuthForm, String> {
    let mut form = vec![
        ("client_id", secret.client_id.clone()),
        ("code", code.to_string()),
        ("code_verifier", secret.verifier.clone()),
        ("redirect_uri", secret.redirect_uri.clone()),
        ("grant_type", "authorization_code".to_string()),
    ];
    if secret.manifest_id == "google_workspace" {
        form.push((
            "client_secret",
            google_credential
                .ok_or_else(|| "oauth_client_secret_missing".to_string())?
                .to_string(),
        ));
    }
    Ok(form)
}

pub(super) fn refresh_token_form(
    manifest_id: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<OAuthForm, String> {
    refresh_token_form_with_google_credential(
        manifest_id,
        client_id,
        refresh_token,
        manifest::google_oauth_client_secret(),
    )
}

fn refresh_token_form_with_google_credential(
    manifest_id: &str,
    client_id: &str,
    refresh_token: &str,
    google_credential: Option<&str>,
) -> Result<OAuthForm, String> {
    let mut form = vec![
        ("client_id", client_id.to_string()),
        ("refresh_token", refresh_token.to_string()),
        ("grant_type", "refresh_token".to_string()),
    ];
    if manifest_id == "google_workspace" {
        form.push((
            "client_secret",
            google_credential
                .ok_or_else(|| "oauth_client_secret_missing".to_string())?
                .to_string(),
        ));
    }
    Ok(form)
}

pub(super) fn post_oauth_form(
    endpoint: &str,
    form: &OAuthForm,
    unreachable_code: &str,
) -> Result<reqwest::blocking::Response, String> {
    reqwest::blocking::Client::new()
        .post(endpoint)
        .form(form)
        .send()
        .map_err(|_| unreachable_code.to_string())
}

fn provider_oauth_error_code(
    manifest_id: &str,
    phase: &str,
    status: StatusCode,
    response: &Value,
) -> String {
    let provider = if manifest_id == "google_workspace" {
        "google"
    } else if manifest_id == microsoft365::MANIFEST_ID {
        "microsoft"
    } else {
        "slack"
    };
    let category = if google_client_authentication_required(manifest_id, phase, response) {
        "client_authentication_required"
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        "rate_limited"
    } else if status.is_server_error() {
        "unavailable"
    } else {
        match response.get("error").and_then(Value::as_str) {
            Some("invalid_request") => "invalid_request",
            Some("invalid_grant") | Some("invalid_code") | Some("invalid_code_verifier") => {
                "invalid_grant"
            }
            Some("access_denied") => "access_denied",
            Some("unauthorized_client") | Some("invalid_client") | Some("bad_client_secret") => {
                "invalid_client"
            }
            Some("invalid_scope") | Some("no_scopes") => "invalid_scope",
            Some("bad_redirect_uri") => "redirect_mismatch",
            Some("pkce_not_allowed") => "pkce_unavailable",
            Some("temporarily_unavailable") | Some("server_error") => "unavailable",
            _ => "rejected",
        }
    };
    format!("{provider}_{phase}_{category}")
}

pub(super) fn parse_oauth_http_response(
    manifest_id: &str,
    phase: &str,
    response: reqwest::blocking::Response,
    invalid_response_code: &str,
) -> Result<Value, String> {
    let status = response.status();
    let body: Value = match response.json() {
        Ok(body) => body,
        Err(_) if status.is_success() => return Err(invalid_response_code.to_string()),
        Err(_) => Value::Null,
    };
    if !status.is_success()
        || body.get("error").is_some()
        || body.get("ok").is_some_and(|value| value == false)
    {
        let code = provider_oauth_error_code(manifest_id, phase, status, &body);
        eprintln!("OAUTH_PROVIDER_REQUEST_FAILED code={code} status={status}");
        return Err(code);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn provider_secret(manifest_id: &str) -> OAuthAttemptSecret {
        OAuthAttemptSecret {
            connector_id: "connector_00000000-0000-4000-8000-000000000099".to_string(),
            manifest_id: manifest_id.to_string(),
            client_id: "client".to_string(),
            state: "expected".to_string(),
            verifier: "verifier".to_string(),
            nonce: "nonce".to_string(),
            redirect_uri: "http://127.0.0.1:4000/oauth/callback".to_string(),
            expires_at_ms: 10_000,
            requested_scopes: vec![],
            created_new_account: true,
            broker_attempt_id: None,
        }
    }

    #[test]
    fn google_desktop_forms_include_the_required_protocol_credential() {
        let secret = provider_secret("google_workspace");
        let expected = "google-desktop-protocol-credential";
        let exchange = authorization_code_form_with_google_credential(
            &secret,
            "one-time-code",
            Some(expected),
        )
        .unwrap();
        let refresh = refresh_token_form_with_google_credential(
            "google_workspace",
            "client",
            "refresh-token",
            Some(expected),
        )
        .unwrap();
        assert!(exchange
            .iter()
            .any(|(key, value)| *key == "client_secret" && value == expected));
        assert!(refresh
            .iter()
            .any(|(key, value)| *key == "client_secret" && value == expected));
        assert!(!serde_json::to_string(&secret)
            .unwrap()
            .contains("client_secret"));
        assert_eq!(
            authorization_code_form_with_google_credential(&secret, "code", None).unwrap_err(),
            "oauth_client_secret_missing"
        );
    }

    #[test]
    fn google_loopback_exchange_sends_pkce_and_the_required_protocol_credential() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut content_length = None;
            loop {
                let mut chunk = [0_u8; 4096];
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0);
                request.extend_from_slice(&chunk[..read]);
                if content_length.is_none() {
                    if let Some(header_end) =
                        request.windows(4).position(|value| value == b"\r\n\r\n")
                    {
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        content_length = headers.lines().find_map(|line| {
                            line.strip_prefix("content-length: ")
                                .or_else(|| line.strip_prefix("Content-Length: "))
                                .and_then(|value| value.parse::<usize>().ok())
                        });
                    }
                }
                if let Some(length) = content_length {
                    let header_end = request
                        .windows(4)
                        .position(|value| value == b"\r\n\r\n")
                        .unwrap()
                        + 4;
                    if request.len() >= header_end + length {
                        break;
                    }
                }
            }
            let header_end = request
                .windows(4)
                .position(|value| value == b"\r\n\r\n")
                .unwrap()
                + 4;
            let body = String::from_utf8(request[header_end..].to_vec()).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 42\r\nConnection: close\r\n\r\n{\"access_token\":\"token\",\"expires_in\":3600}",
                )
                .unwrap();
            body
        });

        let secret = provider_secret("google_workspace");
        let expected = "google-desktop-protocol-credential";
        let form = authorization_code_form_with_google_credential(
            &secret,
            "one-time-code",
            Some(expected),
        )
        .unwrap();
        let response = post_oauth_form(
            &format!("http://{address}/token"),
            &form,
            "oauth_token_exchange_unreachable",
        )
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = server.join().unwrap();
        let parameters = url::form_urlencoded::parse(body.as_bytes()).collect::<Vec<_>>();
        assert!(parameters
            .iter()
            .any(|(key, value)| key == "code_verifier" && value == "verifier"));
        assert!(parameters
            .iter()
            .any(|(key, value)| key == "client_secret" && value == expected));
    }

    #[test]
    fn oauth_public_client_slack_pkce_forms_never_send_a_client_secret() {
        let secret = provider_secret("slack");
        let exchange = authorization_code_form(&secret, "one-time-code").unwrap();
        let refresh = refresh_token_form("slack", "client", "refresh-token").unwrap();
        assert!(exchange
            .iter()
            .any(|(key, value)| { *key == "code_verifier" && value == "verifier" }));
        assert!(!exchange.iter().any(|(key, _)| *key == "client_secret"));
        assert!(!refresh.iter().any(|(key, _)| *key == "client_secret"));
        assert!(!serde_json::to_string(&secret)
            .unwrap()
            .contains("client_secret"));
    }

    #[test]
    fn provider_errors_use_stable_non_descriptive_codes() {
        assert_eq!(
            provider_oauth_error_code(
                "google_workspace",
                "token",
                StatusCode::BAD_REQUEST,
                &serde_json::json!({
                    "error": "invalid_request",
                    "error_description": "client_secret is missing"
                }),
            ),
            "google_token_client_authentication_required"
        );
        assert_eq!(
            provider_oauth_error_code(
                "google_workspace",
                "token",
                StatusCode::BAD_REQUEST,
                &serde_json::json!({
                    "error": "invalid_request",
                    "error_description": "provider detail that must not be persisted"
                }),
            ),
            "google_token_invalid_request"
        );
        assert_eq!(
            provider_oauth_error_code(
                "google_workspace",
                "refresh",
                StatusCode::TOO_MANY_REQUESTS,
                &Value::Null,
            ),
            "google_refresh_rate_limited"
        );
        assert_eq!(
            provider_oauth_error_code(
                "slack",
                "token",
                StatusCode::OK,
                &serde_json::json!({"ok": false, "error": "unrecognized_provider_text"}),
            ),
            "slack_token_rejected"
        );
        assert_eq!(
            provider_oauth_error_code(
                "slack",
                "token",
                StatusCode::OK,
                &serde_json::json!({"ok": false, "error": "invalid_code_verifier"}),
            ),
            "slack_token_invalid_grant"
        );
        assert_eq!(
            provider_oauth_error_code(
                "slack",
                "token",
                StatusCode::OK,
                &serde_json::json!({"ok": false, "error": "bad_redirect_uri"}),
            ),
            "slack_token_redirect_mismatch"
        );
    }
}
