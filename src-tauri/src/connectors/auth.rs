use super::{
    manifest, microsoft365, oauth_broker,
    oauth_protocol::{authorization_code_form, parse_oauth_http_response, post_oauth_form},
    repository, BeginOAuthResponse, ConnectorIdentityMetadata,
};
use crate::{
    db::PersistenceEngine,
    foundation::{clock::unix_time_ms_i64, digest::sha256_hex},
    secret_store,
    sovereign_identity::SovereignIdentity,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand_core::{OsRng, RngCore};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{net::TcpListener, thread};

mod authorization;

pub(super) const OAUTH_TTL_MS: i64 = 5 * 60 * 1_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct OAuthAttemptSecret {
    pub connector_id: String,
    pub manifest_id: String,
    pub client_id: String,
    pub state: String,
    pub verifier: String,
    pub nonce: String,
    pub redirect_uri: String,
    pub expires_at_ms: i64,
    pub requested_scopes: Vec<String>,
    pub created_new_account: bool,
    #[serde(default)]
    pub broker_attempt_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct ConnectorCredential {
    pub manifest_id: String,
    pub access_token: String,
    #[serde(default)]
    pub bot_access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub scopes: Vec<String>,
    pub expires_at_ms: Option<i64>,
    pub refresh_expires_at_ms: Option<i64>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub tenant_label: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub account_principal: Option<String>,
    #[serde(default)]
    pub identity_binding_hash: Option<String>,
}

fn random_url_secret(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    OsRng.fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

fn loopback_redirect_uri(manifest_id: &str, port: u16) -> String {
    debug_assert!(
        manifest_id != microsoft365::MANIFEST_ID || port == microsoft365::LOOPBACK_REDIRECT_PORT
    );
    let host = if manifest_id == "slack" {
        "localhost"
    } else {
        "127.0.0.1"
    };
    format!("http://{host}:{port}/oauth/callback")
}

fn prepare_authorization(
    engine: &PersistenceEngine,
    manifest_id: &str,
    existing_connector_id: Option<&str>,
    requested_operations: &[String],
) -> Result<(u32, String, Vec<String>, Option<String>), String> {
    authorization::prepare(
        engine,
        manifest_id,
        existing_connector_id,
        requested_operations,
    )
}

fn persist_attempt(
    engine: &PersistenceEngine,
    attempt_id: &str,
    secret: &OAuthAttemptSecret,
) -> Result<(), String> {
    let persisted = secret_store::set_connector_oauth_attempt(
        attempt_id,
        &serde_json::to_string(secret).map_err(|error| error.to_string())?,
    )
    .and_then(|_| {
        repository::record_oauth_attempt(
            engine,
            attempt_id,
            &secret.connector_id,
            &sha256_hex(secret.state.as_bytes()),
            &secret.redirect_uri,
            secret.expires_at_ms,
        )
    });
    if let Err(error) = persisted {
        let _ = secret_store::delete_connector_oauth_attempt(attempt_id);
        if secret.created_new_account {
            let _ = repository::disconnect(engine, &secret.connector_id);
        }
        return Err(error);
    }
    Ok(())
}

pub(super) fn begin(
    engine: &PersistenceEngine,
    identity: &SovereignIdentity,
    manifest_id: &str,
    existing_connector_id: Option<&str>,
    requested_operations: &[String],
) -> Result<BeginOAuthResponse, String> {
    let (manifest_version, client_id, requested_scopes, prepared_existing) = prepare_authorization(
        engine,
        manifest_id,
        existing_connector_id,
        requested_operations,
    )?;
    if manifest_id == "slack" && !manifest::slack_bot_scopes(requested_operations)?.is_empty() {
        return begin_slack_messaging_authorization(
            engine,
            identity,
            &client_id,
            prepared_existing,
            manifest_version,
            &requested_scopes,
            requested_operations,
        );
    }
    let bind_port = match manifest_id {
        "slack" => option_env!("OOMU_SLACK_OAUTH_REDIRECT_PORT")
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(53_682),
        microsoft365::MANIFEST_ID => microsoft365::LOOPBACK_REDIRECT_PORT,
        _ => 0,
    };
    let listener = TcpListener::bind(("127.0.0.1", bind_port))
        .map_err(|error| format!("Unable to bind the exact OAuth loopback redirect: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let redirect_uri = loopback_redirect_uri(manifest_id, port);
    let (connector_id, created_new_account) = if let Some(connector_id) = prepared_existing {
        (connector_id, false)
    } else {
        (
            repository::create_account(engine, manifest_id, manifest_version)?,
            true,
        )
    };
    let attempt_id = format!("oauth_{}", random_url_secret(18));
    let state = random_url_secret(32);
    let verifier = random_url_secret(64);
    let nonce = random_url_secret(32);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let expires_at_ms = unix_time_ms_i64() + OAUTH_TTL_MS;
    let secret = OAuthAttemptSecret {
        connector_id: connector_id.clone(),
        manifest_id: manifest_id.to_string(),
        client_id: client_id.clone(),
        state: state.clone(),
        verifier,
        nonce: nonce.clone(),
        redirect_uri: redirect_uri.clone(),
        expires_at_ms,
        requested_scopes: requested_scopes.clone(),
        created_new_account,
        broker_attempt_id: None,
    };
    persist_attempt(engine, &attempt_id, &secret)?;
    let authorization = authorization::authorization_url(
        manifest_id,
        &client_id,
        &redirect_uri,
        &state,
        &challenge,
        &nonce,
        &requested_scopes,
        requested_operations,
        created_new_account,
    )?;
    let worker_engine = engine.clone();
    let worker_identity = identity.clone();
    thread::Builder::new()
        .name(format!("oauth-loopback-{manifest_id}"))
        .spawn(move || {
            super::oauth_callback::receive_callback(
                listener,
                worker_engine,
                worker_identity,
                attempt_id,
                secret,
            )
        })
        .map_err(|e| e.to_string())?;
    Ok(BeginOAuthResponse {
        connector_id,
        authorization_url: authorization.to_string(),
        expires_at_ms,
        requested_scopes,
    })
}

fn begin_slack_messaging_authorization(
    engine: &PersistenceEngine,
    identity: &SovereignIdentity,
    client_id: &str,
    existing_connector_id: Option<String>,
    manifest_version: u32,
    requested_scopes: &[String],
    requested_operations: &[String],
) -> Result<BeginOAuthResponse, String> {
    let (connector_id, created_new_account) = if let Some(connector_id) = existing_connector_id {
        (connector_id, false)
    } else {
        (
            repository::create_account(engine, "slack", manifest_version)?,
            true,
        )
    };
    let user_scopes = manifest::slack_read_scopes();
    let bot_scopes = manifest::slack_bot_scopes(requested_operations)?;
    let started = match oauth_broker::begin_authorization(
        &connector_id,
        client_id,
        &user_scopes,
        &bot_scopes,
        identity,
    ) {
        Ok(started) => started,
        Err(error) => {
            if created_new_account {
                let _ = repository::disconnect(engine, &connector_id);
            }
            return Err(error);
        }
    };
    let attempt_id = format!("oauth_{}", random_url_secret(18));
    let secret = OAuthAttemptSecret {
        connector_id: connector_id.clone(),
        manifest_id: "slack".to_string(),
        client_id: client_id.to_string(),
        state: random_url_secret(32),
        verifier: String::new(),
        nonce: random_url_secret(32),
        redirect_uri: started.authorization_url.clone(),
        expires_at_ms: started.expires_at_ms,
        requested_scopes: requested_scopes.to_vec(),
        created_new_account,
        broker_attempt_id: Some(started.broker_attempt_id),
    };
    persist_attempt(engine, &attempt_id, &secret)?;
    let worker_engine = engine.clone();
    let worker_identity = identity.clone();
    let worker_attempt_id = attempt_id.clone();
    thread::Builder::new()
        .name("oauth-broker-slack".to_string())
        .spawn(move || {
            super::oauth_callback::receive_broker_completion(
                worker_engine,
                worker_identity,
                worker_attempt_id,
                secret,
            )
        })
        .map_err(|error| error.to_string())?;
    Ok(BeginOAuthResponse {
        connector_id,
        authorization_url: started.authorization_url,
        expires_at_ms: started.expires_at_ms,
        requested_scopes: requested_scopes.to_vec(),
    })
}

pub(super) fn exchange_code(
    secret: &OAuthAttemptSecret,
    code: &str,
    _identity: &SovereignIdentity,
) -> Result<ConnectorCredential, String> {
    if secret.manifest_id == microsoft365::MANIFEST_ID {
        return microsoft365::exchange(microsoft365::ExchangeRequest {
            client_id: &secret.client_id,
            code,
            verifier: &secret.verifier,
            redirect_uri: &secret.redirect_uri,
            nonce: &secret.nonce,
            requested_scopes: &secret.requested_scopes,
        });
    }
    let endpoint = match secret.manifest_id.as_str() {
        "google_workspace" => "https://oauth2.googleapis.com/token",
        "slack" => "https://slack.com/api/oauth.v2.access",
        _ => return Err("connector_oauth_provider_unsupported".to_string()),
    };
    let form = authorization_code_form(secret, code)?;
    let response = post_oauth_form(endpoint, &form, "oauth_token_exchange_unreachable")?;
    let response = parse_oauth_http_response(
        &secret.manifest_id,
        "token",
        response,
        "oauth_token_response_invalid",
    )?;
    let source = response.get("authed_user").unwrap_or(&response);
    let access_token = source
        .get("access_token")
        .or_else(|| response.get("access_token"))
        .and_then(Value::as_str)
        .ok_or_else(|| "oauth_access_token_missing".to_string())?
        .to_string();
    let refresh_token = source
        .get("refresh_token")
        .or_else(|| response.get("refresh_token"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let expires_in = source
        .get("expires_in")
        .or_else(|| response.get("expires_in"))
        .and_then(Value::as_i64);
    let mut scope_values = source
        .get("scope")
        .or_else(|| response.get("scope"))
        .and_then(Value::as_str)
        .map(|raw| {
            raw.split([',', ' '])
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| secret.requested_scopes.clone());
    if secret.manifest_id == "google_workspace" {
        scope_values = manifest::normalize_google_scopes(&scope_values);
    }
    Ok(ConnectorCredential {
        manifest_id: secret.manifest_id.clone(),
        access_token,
        bot_access_token: None,
        refresh_token,
        token_type: source
            .get("token_type")
            .or_else(|| response.get("token_type"))
            .and_then(Value::as_str)
            .unwrap_or("Bearer")
            .to_string(),
        scopes: scope_values,
        expires_at_ms: expires_in.map(|seconds| unix_time_ms_i64() + seconds * 1_000),
        refresh_expires_at_ms: (secret.manifest_id == "slack")
            .then(|| unix_time_ms_i64() + 30 * 24 * 60 * 60 * 1_000),
        tenant_id: None,
        tenant_label: None,
        account_id: None,
        account_principal: None,
        identity_binding_hash: None,
    })
}

pub(super) struct IdentityProbe {
    pub label: String,
    pub subject: String,
    pub metadata: Option<ConnectorIdentityMetadata>,
}

fn slack_identity_from_response(response: &Value) -> Result<IdentityProbe, String> {
    let user_id = response
        .get("user_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "connector_identity_probe_invalid".to_string())?;
    let team_id = response
        .get("team_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "connector_identity_probe_invalid".to_string())?;
    let label = response
        .get("user")
        .or_else(|| response.get("team"))
        .and_then(Value::as_str)
        .unwrap_or(user_id)
        .to_string();
    let now = unix_time_ms_i64();
    Ok(IdentityProbe {
        label: label.clone(),
        subject: user_id.to_string(),
        metadata: Some(ConnectorIdentityMetadata {
            tenant_id: team_id.to_string(),
            tenant_label: response
                .get("team")
                .and_then(Value::as_str)
                .unwrap_or(team_id)
                .to_string(),
            account_id: user_id.to_string(),
            account_principal: label,
            account_kind: "work".to_string(),
            identity_binding_hash: sha256_hex(user_id.as_bytes()),
            data_routing: vec!["https://slack.com".to_string()],
            consent_reviewed_at_ms: now,
            identity_verified_at_ms: now,
        }),
    })
}

fn google_identity_from_response(response: &Value) -> Result<IdentityProbe, String> {
    let subject = response
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "connector_identity_probe_invalid".to_string())?;
    let label = response
        .get("email")
        .or_else(|| response.get("name"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Connected account")
        .to_string();
    Ok(IdentityProbe {
        label,
        subject: subject.to_string(),
        metadata: None,
    })
}

fn identity_probe_request(
    client: &reqwest::blocking::Client,
    credential: &ConnectorCredential,
) -> Result<reqwest::blocking::RequestBuilder, String> {
    let endpoint = match credential.manifest_id.as_str() {
        "google_workspace" => "https://www.googleapis.com/oauth2/v2/userinfo",
        "slack" => "https://slack.com/api/auth.test",
        _ => return Err("connector_identity_provider_unsupported".to_string()),
    };
    let request = if credential.manifest_id == "slack" {
        client.post(endpoint)
    } else {
        client.get(endpoint)
    };
    Ok(request.bearer_auth(&credential.access_token))
}

pub(super) fn preserve_incremental_refresh_token(
    previous: Option<&ConnectorCredential>,
    replacement: &mut ConnectorCredential,
) {
    if matches!(
        replacement.manifest_id.as_str(),
        microsoft365::MANIFEST_ID | "google_workspace" | "slack"
    ) && replacement.refresh_token.is_none()
    {
        replacement.refresh_token = previous.and_then(|value| value.refresh_token.clone());
    }
}

pub(super) fn probe_identity_details(
    credential: &ConnectorCredential,
) -> Result<IdentityProbe, String> {
    if credential.manifest_id == microsoft365::MANIFEST_ID {
        let identity = microsoft365::probe_identity(credential)?;
        return Ok(IdentityProbe {
            label: identity.label,
            subject: identity.subject,
            metadata: Some(identity.metadata),
        });
    }
    let client = reqwest::blocking::Client::new();
    let response = identity_probe_request(&client, credential)?
        .send()
        .map_err(|_| "connector_identity_probe_offline".to_string())?;
    let status = response.status();
    let response: Value = response
        .json()
        .map_err(|_| "connector_identity_probe_invalid".to_string())?;
    if !status.is_success() {
        return Err(if status == StatusCode::UNAUTHORIZED {
            "connector_authorization_revoked"
        } else if status == StatusCode::TOO_MANY_REQUESTS {
            "connector_rate_limited"
        } else {
            "connector_identity_probe_rejected"
        }
        .to_string());
    }
    if response.get("ok").is_some_and(|value| value == false) {
        return Err(response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("connector_identity_probe_rejected")
            .to_string());
    }
    if credential.manifest_id == "slack" {
        return slack_identity_from_response(&response);
    }
    if credential.manifest_id == "google_workspace" {
        return google_identity_from_response(&response);
    }
    let label = response
        .get("email")
        .or_else(|| response.get("user"))
        .or_else(|| response.get("team"))
        .and_then(Value::as_str)
        .unwrap_or("Connected account")
        .to_string();
    let subject = response
        .get("id")
        .or_else(|| response.get("user_id"))
        .or_else(|| response.get("team_id"))
        .and_then(Value::as_str)
        .unwrap_or(&label)
        .to_string();
    Ok(IdentityProbe {
        label,
        subject,
        metadata: None,
    })
}

pub(super) fn probe_identity(credential: &ConnectorCredential) -> Result<(String, String), String> {
    let identity = probe_identity_details(credential)?;
    Ok((identity.label, identity.subject))
}

pub(super) fn credentials(connector_id: &str) -> Result<ConnectorCredential, String> {
    let raw = secret_store::get_connector_credentials(connector_id)?
        .ok_or_else(|| "Connector credential is unavailable in Keychain.".to_string())?;
    serde_json::from_str(&raw).map_err(|_| "Connector credential is invalid.".to_string())
}

impl ConnectorCredential {
    pub(super) fn slack_bot_token(&self) -> Result<&str, String> {
        self.bot_access_token
            .as_deref()
            .or_else(|| {
                self.access_token
                    .starts_with("xoxb-")
                    .then_some(self.access_token.as_str())
            })
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| "slack_messaging_consent_required".to_string())
    }
}

pub(super) use super::auth_refresh::refresh_if_needed;

pub(super) fn revoke(connector_id: &str) -> Result<(), String> {
    let mut remote_failure = None;
    if let Ok(credential) = credentials(connector_id) {
        if credential.manifest_id == microsoft365::MANIFEST_ID {
            // Microsoft does not expose a generic OAuth token-revocation endpoint
            // for native clients. Disconnect is an immediate local Keychain revoke.
            return secret_store::delete_connector_credentials(connector_id);
        }
        let endpoint = if credential.manifest_id == "google_workspace" {
            "https://oauth2.googleapis.com/revoke"
        } else {
            "https://slack.com/api/auth.revoke"
        };
        let response = if credential.manifest_id == "google_workspace" {
            reqwest::blocking::Client::new()
                .post(endpoint)
                .form(&[("token", credential.access_token.as_str())])
                .send()
        } else {
            reqwest::blocking::Client::new()
                .post(endpoint)
                .bearer_auth(&credential.access_token)
                .send()
        };
        remote_failure = match response {
            Ok(response) if response.status().is_success() => None,
            Ok(_) => Some("connector_revocation_rejected"),
            Err(_) => Some("connector_revocation_unreachable"),
        };
    }
    // Removing a connection means OOMU must stop using it immediately. Provider
    // revocation is best-effort because an expired token or offline provider must
    // never trap the user's credential in this app. The provider's own access
    // page remains the authoritative place to revoke a grant server-side.
    secret_store::delete_connector_credentials(connector_id)?;
    if let Some(code) = remote_failure {
        eprintln!("CONNECTOR_REMOTE_REVOCATION_INCOMPLETE code={code}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
