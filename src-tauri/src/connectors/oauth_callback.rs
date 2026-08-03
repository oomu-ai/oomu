use super::{
    auth::{
        exchange_code, preserve_incremental_refresh_token, probe_identity_details,
        ConnectorCredential, IdentityProbe, OAuthAttemptSecret, OAUTH_TTL_MS,
    },
    manifest, repository,
};
use crate::{
    db::PersistenceEngine,
    foundation::{clock::unix_time_ms_i64, digest::sha256_hex},
    secret_store,
    sovereign_identity::SovereignIdentity,
};
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};
use url::Url;

fn localized_callback_body(engine: &PersistenceEngine, key: &str, fallback: &str) -> String {
    crate::settings::locale_state_for_engine(engine, None)
        .ok()
        .and_then(|state| {
            state
                .translations
                .pointer(&format!("/setup/{key}"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| fallback.to_string())
}

fn callback_stage_error(stage: &str, code: String) -> String {
    eprintln!(
        "OAUTH_CALLBACK_STAGE_FAILED stage={} code={}",
        crate::redaction::redacted_log_text(stage),
        crate::redaction::redacted_log_text(&code),
    );
    code
}

pub(super) fn persist_oauth_failure(
    engine: &PersistenceEngine,
    attempt_id: &str,
    code: &str,
    preserve_connection: bool,
) -> bool {
    repository::fail_oauth(engine, attempt_id, code, preserve_connection)
        .map_err(|error| {
            eprintln!(
                "OAUTH_FAILURE_PERSIST_FAILED code={} message={}",
                crate::redaction::redacted_log_text(code),
                crate::redaction::redacted_log_text(&error)
            );
        })
        .is_ok()
}

pub(super) fn receive_broker_completion(
    engine: PersistenceEngine,
    identity: SovereignIdentity,
    attempt_id: String,
    secret: OAuthAttemptSecret,
) {
    let deadline = Instant::now()
        + Duration::from_millis(
            secret
                .expires_at_ms
                .saturating_sub(unix_time_ms_i64())
                .max(1) as u64,
        );
    let Some(broker_attempt_id) = secret.broker_attempt_id.as_deref() else {
        let _ = persist_oauth_failure(&engine, &attempt_id, "oauth_broker_attempt_missing", false);
        let _ = secret_store::delete_connector_oauth_attempt(&attempt_id);
        return;
    };
    let mut delay = Duration::from_millis(750);
    loop {
        match super::oauth_broker::poll_authorization(
            &secret.connector_id,
            &secret.client_id,
            broker_attempt_id,
            &identity,
        ) {
            Ok(super::oauth_broker::BrokerAuthorizationPoll::Pending) => {}
            Ok(super::oauth_broker::BrokerAuthorizationPoll::Complete(credential)) => {
                let result =
                    persist_completed_credential(&engine, &attempt_id, &secret, credential);
                if let Err(code) = result {
                    if secret.created_new_account {
                        let _ = secret_store::delete_connector_credentials(&secret.connector_id);
                    }
                    let _ = persist_oauth_failure(
                        &engine,
                        &attempt_id,
                        &code,
                        !secret.created_new_account,
                    );
                }
                let _ = secret_store::delete_connector_oauth_attempt(&attempt_id);
                return;
            }
            Err(code)
                if matches!(
                    code.as_str(),
                    "oauth_broker_unreachable" | "oauth_broker_rejected"
                ) && Instant::now() < deadline => {}
            Err(code) => {
                if secret.created_new_account {
                    let _ = secret_store::delete_connector_credentials(&secret.connector_id);
                }
                let _ =
                    persist_oauth_failure(&engine, &attempt_id, &code, !secret.created_new_account);
                let _ = secret_store::delete_connector_oauth_attempt(&attempt_id);
                return;
            }
        }
        if Instant::now() >= deadline {
            if secret.created_new_account {
                let _ = secret_store::delete_connector_credentials(&secret.connector_id);
            }
            let _ = persist_oauth_failure(
                &engine,
                &attempt_id,
                "slack_authorization_expired",
                !secret.created_new_account,
            );
            let _ = secret_store::delete_connector_oauth_attempt(&attempt_id);
            return;
        }
        thread::sleep(delay);
        delay = (delay * 2).min(Duration::from_secs(4));
    }
}

pub(super) fn receive_callback(
    listener: TcpListener,
    engine: PersistenceEngine,
    identity: SovereignIdentity,
    attempt_id: String,
    secret: OAuthAttemptSecret,
) {
    let deadline = Instant::now() + Duration::from_millis(OAUTH_TTL_MS as u64);
    loop {
        match listener.accept() {
            Ok((mut stream, peer)) => {
                let result = if !peer.ip().is_loopback() {
                    Err("oauth_non_loopback_peer".to_string())
                } else {
                    handle_stream(&mut stream, &engine, &identity, &attempt_id, &secret)
                };
                let (status, body, delete_attempt) = match result {
                    Ok(()) => (
                        "200 OK",
                        localized_callback_body(
                            &engine,
                            "oauth_callback_success",
                            "OOMU connected the account. You can close this window and return to OOMU.",
                        ),
                        true,
                    ),
                    Err(code) => {
                        if secret.created_new_account {
                            let _ = secret_store::delete_connector_credentials(&secret.connector_id);
                        }
                        let persisted = persist_oauth_failure(
                            &engine,
                            &attempt_id,
                            &code,
                            !secret.created_new_account,
                        );
                        (
                            "400 Bad Request",
                            localized_callback_body(
                                &engine,
                                "oauth_callback_failure",
                                "OOMU could not finish connecting this account. Return to OOMU for repair guidance.",
                            ),
                            persisted,
                        )
                    }
                };
                let _ = stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
                if delete_attempt {
                    let _ = secret_store::delete_connector_oauth_attempt(&attempt_id);
                }
                return;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(100))
            }
            Err(_) => {
                if persist_oauth_failure(
                    &engine,
                    &attempt_id,
                    "oauth_loopback_failed",
                    !secret.created_new_account,
                ) {
                    let _ = secret_store::delete_connector_oauth_attempt(&attempt_id);
                }
                return;
            }
        }
        if Instant::now() >= deadline {
            if persist_oauth_failure(
                &engine,
                &attempt_id,
                "oauth_expired",
                !secret.created_new_account,
            ) {
                let _ = secret_store::delete_connector_oauth_attempt(&attempt_id);
            }
            return;
        }
    }
}

fn handle_stream(
    stream: &mut std::net::TcpStream,
    engine: &PersistenceEngine,
    identity: &SovereignIdentity,
    attempt_id: &str,
    secret: &OAuthAttemptSecret,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| error.to_string())?;
    let mut buffer = [0_u8; 16_384];
    let count = stream
        .read(&mut buffer)
        .map_err(|_| "oauth_callback_read_failed".to_string())?;
    let request = std::str::from_utf8(&buffer[..count])
        .map_err(|_| "oauth_callback_invalid_utf8".to_string())?;
    let target = callback_target(request)?;
    let code = verify_callback_target(target, secret, unix_time_ms_i64())?;
    let credential = exchange_code(secret, &code, identity)
        .map_err(|error| callback_stage_error("token_exchange", error))?;
    persist_completed_credential(engine, attempt_id, secret, credential)
}

fn previous_credential(
    secret: &OAuthAttemptSecret,
) -> Result<(Option<String>, Option<ConnectorCredential>), String> {
    let serialized = if secret.created_new_account {
        None
    } else {
        secret_store::get_connector_credentials(&secret.connector_id)?
    };
    let parsed = serialized
        .as_deref()
        .map(serde_json::from_str::<ConnectorCredential>)
        .transpose()
        .map_err(|_| "connector_previous_credential_invalid".to_string())?;
    Ok((serialized, parsed))
}

fn merge_incremental_credential(
    previous: Option<&ConnectorCredential>,
    credential: &mut ConnectorCredential,
) {
    preserve_incremental_refresh_token(previous, credential);
    let Some(previous) = previous else { return };
    if credential.bot_access_token.is_none() {
        credential.bot_access_token = previous.bot_access_token.clone();
    }
    for scope in &previous.scopes {
        if !credential.scopes.contains(scope) {
            credential.scopes.push(scope.clone());
        }
    }
}

fn bind_verified_identity(
    engine: &PersistenceEngine,
    secret: &OAuthAttemptSecret,
    credential: &mut ConnectorCredential,
) -> Result<IdentityProbe, String> {
    let identity = probe_identity_details(credential)?;
    let existing = repository::identity_binding_hash(engine, &secret.connector_id)?;
    let observed = sha256_hex(identity.subject.as_bytes());
    if !secret.created_new_account
        && secret.manifest_id != super::microsoft365::MANIFEST_ID
        && existing.as_deref().is_some_and(|value| value != observed)
    {
        return Err("connector_account_binding_changed".to_string());
    }
    if let Some(metadata) = identity.metadata.as_ref() {
        if !secret.created_new_account
            && existing
                .as_deref()
                .is_some_and(|value| value != metadata.identity_binding_hash)
        {
            return Err("connector_account_binding_changed".to_string());
        }
        credential.tenant_id = Some(metadata.tenant_id.clone());
        credential.tenant_label = Some(metadata.tenant_label.clone());
        credential.account_id = Some(metadata.account_id.clone());
        credential.account_principal = Some(metadata.account_principal.clone());
        credential.identity_binding_hash = Some(metadata.identity_binding_hash.clone());
    }
    Ok(identity)
}

fn restore_previous_credential(connector_id: &str, previous: Option<String>) {
    if let Some(serialized) = previous {
        let _ = secret_store::set_connector_credentials(connector_id, &serialized);
    } else {
        let _ = secret_store::delete_connector_credentials(connector_id);
    }
}

fn persist_completed_credential(
    engine: &PersistenceEngine,
    attempt_id: &str,
    secret: &OAuthAttemptSecret,
    mut credential: ConnectorCredential,
) -> Result<(), String> {
    let (previous_serialized, previous_parsed) = previous_credential(secret)
        .map_err(|error| callback_stage_error("previous_credential", error))?;
    merge_incremental_credential(previous_parsed.as_ref(), &mut credential);
    if credential.manifest_id == "google_workspace" {
        credential.scopes = manifest::normalize_google_scopes(&credential.scopes);
    }
    let consent_complete = if credential.manifest_id == "google_workspace" {
        manifest::google_scopes_include(&secret.requested_scopes, &credential.scopes)
    } else {
        secret
            .requested_scopes
            .iter()
            .all(|scope| credential.scopes.contains(scope))
    };
    if !consent_complete {
        return Err(callback_stage_error(
            "consent_validation",
            "connector_requested_consent_missing".to_string(),
        ));
    }
    if credential.manifest_id == super::microsoft365::MANIFEST_ID
        && credential.refresh_token.is_none()
    {
        return Err(callback_stage_error(
            "refresh_token_validation",
            "microsoft_refresh_token_missing".to_string(),
        ));
    }
    let identity = bind_verified_identity(engine, secret, &mut credential)
        .map_err(|error| callback_stage_error("identity_probe", error))?;
    secret_store::set_connector_credentials(
        &secret.connector_id,
        &serde_json::to_string(&credential)
            .map_err(|error| callback_stage_error("credential_serialize", error.to_string()))?,
    )
    .map_err(|error| callback_stage_error("credential_store", error))?;
    let persisted = repository::finish_oauth(
        engine,
        attempt_id,
        &secret.connector_id,
        &identity.label,
        &identity.subject,
        &credential.scopes,
        credential.expires_at_ms,
        credential.refresh_expires_at_ms,
        identity.metadata.as_ref(),
    );
    if let Err(error) = persisted {
        restore_previous_credential(&secret.connector_id, previous_serialized);
        return Err(callback_stage_error("repository_commit", error));
    }
    Ok(())
}

fn callback_target(request: &str) -> Result<&str, String> {
    let mut parts = request
        .lines()
        .next()
        .ok_or_else(|| "oauth_callback_invalid_request".to_string())?
        .split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "oauth_callback_invalid_request".to_string())?;
    let target = parts
        .next()
        .ok_or_else(|| "oauth_callback_invalid_request".to_string())?;
    let version = parts
        .next()
        .ok_or_else(|| "oauth_callback_invalid_request".to_string())?;
    if method != "GET" || !version.starts_with("HTTP/1.") || parts.next().is_some() {
        return Err("oauth_callback_invalid_request".to_string());
    }
    Ok(target)
}

fn verify_callback_target(
    target: &str,
    secret: &OAuthAttemptSecret,
    now: i64,
) -> Result<String, String> {
    let callback = Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| "oauth_callback_invalid_url".to_string())?;
    if callback.path() != "/oauth/callback" {
        return Err("oauth_redirect_mismatch".to_string());
    }
    if now > secret.expires_at_ms {
        return Err("oauth_expired".to_string());
    }
    let mut state = None;
    let mut code = None;
    let mut issuer = None;
    for (key, value) in callback.query_pairs() {
        match key.as_ref() {
            "state" if state.is_some() => return Err("oauth_state_duplicate".to_string()),
            "state" => state = Some(value.into_owned()),
            "code" if code.is_some() => return Err("oauth_code_duplicate".to_string()),
            "code" => code = Some(value.into_owned()),
            "iss" if issuer.is_some() => return Err("oauth_issuer_duplicate".to_string()),
            "iss" => issuer = Some(value.into_owned()),
            _ => {}
        }
    }
    if state.as_deref() != Some(secret.state.as_str()) {
        return Err("oauth_state_mismatch".to_string());
    }
    if secret.manifest_id == "google_workspace"
        && issuer.as_deref() != Some("https://accounts.google.com")
    {
        return Err("oauth_issuer_mismatch".to_string());
    }
    code.ok_or_else(|| "oauth_code_missing".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret() -> OAuthAttemptSecret {
        OAuthAttemptSecret {
            connector_id: "connector_00000000-0000-4000-8000-000000000099".to_string(),
            manifest_id: "google_workspace".to_string(),
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
    fn callback_rejects_state_redirect_and_expiry_mismatch() {
        let secret = secret();
        assert_eq!(
            verify_callback_target(
                "/oauth/callback?state=wrong&code=abc&iss=https%3A%2F%2Faccounts.google.com",
                &secret,
                1
            )
            .unwrap_err(),
            "oauth_state_mismatch"
        );
        assert_eq!(
            verify_callback_target(
                "/other?state=expected&code=abc&iss=https%3A%2F%2Faccounts.google.com",
                &secret,
                1
            )
            .unwrap_err(),
            "oauth_redirect_mismatch"
        );
        assert_eq!(
            verify_callback_target(
                "/oauth/callback?state=expected&code=abc&iss=https%3A%2F%2Faccounts.google.com",
                &secret,
                20_000
            )
            .unwrap_err(),
            "oauth_expired"
        );
    }

    #[test]
    fn callback_requires_get_and_unique_security_parameters() {
        assert_eq!(
            callback_target("POST /oauth/callback?state=expected&code=abc HTTP/1.1\r\n")
                .unwrap_err(),
            "oauth_callback_invalid_request"
        );
        let secret = secret();
        assert_eq!(
            verify_callback_target(
                "/oauth/callback?state=expected&state=expected&code=abc",
                &secret,
                1,
            )
            .unwrap_err(),
            "oauth_state_duplicate"
        );
        assert_eq!(
            verify_callback_target(
                "/oauth/callback?state=expected&code=abc&code=def",
                &secret,
                1,
            )
            .unwrap_err(),
            "oauth_code_duplicate"
        );
    }

    #[test]
    fn google_callback_accepts_verified_fields_with_provider_extras() {
        let secret = secret();
        let code = verify_callback_target(
            "/oauth/callback?state=expected&iss=https%3A%2F%2Faccounts.google.com&code=one-time-code&scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fuserinfo.profile+https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fuserinfo.email&authuser=1&prompt=consent",
            &secret,
            1,
        )
        .unwrap();
        assert_eq!(code, "one-time-code");
    }
}
