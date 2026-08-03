use super::{
    auth::{credentials, preserve_incremental_refresh_token, ConnectorCredential},
    manifest, microsoft365, oauth_broker,
    oauth_protocol::{parse_oauth_http_response, post_oauth_form, refresh_token_form},
    repository,
};
use crate::{
    db::PersistenceEngine, foundation::clock::unix_time_ms_i64, secret_store,
    sovereign_identity::SovereignIdentity,
};
use serde_json::Value;

fn persist(
    engine: &PersistenceEngine,
    connector_id: &str,
    probe_code: &str,
    credential: ConnectorCredential,
) -> Result<ConnectorCredential, String> {
    secret_store::set_connector_credentials(
        connector_id,
        &serde_json::to_string(&credential).map_err(|error| error.to_string())?,
    )?;
    repository::record_probe(
        engine,
        connector_id,
        "authorized",
        probe_code,
        credential.expires_at_ms,
    )?;
    Ok(credential)
}

fn refresh_slack_messaging(
    engine: &PersistenceEngine,
    connector_id: &str,
    client_id: &str,
    current: &ConnectorCredential,
    refresh_token: &str,
    identity: &SovereignIdentity,
) -> Result<ConnectorCredential, String> {
    let mut refreshed =
        oauth_broker::refresh(connector_id, client_id, current, refresh_token, identity)?;
    preserve_incremental_refresh_token(Some(current), &mut refreshed);
    if refreshed.bot_access_token.is_none() {
        refreshed.bot_access_token = current.bot_access_token.clone();
    }
    for scope in &current.scopes {
        if !refreshed.scopes.contains(scope) {
            refreshed.scopes.push(scope.clone());
        }
    }
    persist(engine, connector_id, "token_refreshed", refreshed)
}

fn refresh_direct(
    engine: &PersistenceEngine,
    connector_id: &str,
    client_id: &str,
    current: ConnectorCredential,
    refresh_token: &str,
) -> Result<ConnectorCredential, String> {
    let endpoint = match current.manifest_id.as_str() {
        "google_workspace" => "https://oauth2.googleapis.com/token",
        "slack" => "https://slack.com/api/oauth.v2.access",
        _ => return Err("connector_oauth_provider_unsupported".to_string()),
    };
    let form = refresh_token_form(&current.manifest_id, client_id, refresh_token)?;
    let response = post_oauth_form(endpoint, &form, "oauth_refresh_unreachable")?;
    let response = parse_oauth_http_response(
        &current.manifest_id,
        "refresh",
        response,
        "oauth_refresh_invalid",
    )?;
    let source = response.get("authed_user").unwrap_or(&response);
    let mut refreshed = current;
    refreshed.access_token = source
        .get("access_token")
        .or_else(|| response.get("access_token"))
        .and_then(Value::as_str)
        .ok_or_else(|| "oauth_access_token_missing".to_string())?
        .to_string();
    if let Some(rotated) = source
        .get("refresh_token")
        .or_else(|| response.get("refresh_token"))
        .and_then(Value::as_str)
    {
        refreshed.refresh_token = Some(rotated.to_string());
    }
    refreshed.expires_at_ms = source
        .get("expires_in")
        .or_else(|| response.get("expires_in"))
        .and_then(Value::as_i64)
        .map(|seconds| unix_time_ms_i64() + seconds * 1_000);
    if refreshed.manifest_id == "slack" {
        refreshed.refresh_expires_at_ms = Some(unix_time_ms_i64() + 30 * 24 * 60 * 60 * 1_000);
    }
    persist(engine, connector_id, "token_refreshed", refreshed)
}

pub(super) fn refresh_if_needed(
    engine: &PersistenceEngine,
    connector_id: &str,
    identity: Option<&SovereignIdentity>,
) -> Result<ConnectorCredential, String> {
    let current = credentials(connector_id)?;
    if current
        .expires_at_ms
        .is_none_or(|expires| expires > unix_time_ms_i64() + 60_000)
    {
        return Ok(current);
    }
    let client_id = manifest::oauth_client_id(&current.manifest_id)
        .ok_or_else(|| "OAuth client identity is unavailable.".to_string())?;
    if current.manifest_id == microsoft365::MANIFEST_ID {
        let refreshed = microsoft365::refresh(&current, client_id)?;
        return persist(engine, connector_id, "microsoft_token_refreshed", refreshed);
    }
    let refresh_token = current
        .refresh_token
        .clone()
        .ok_or_else(|| "oauth_refresh_token_missing".to_string())?;
    if current.manifest_id == "slack" && current.bot_access_token.is_some() {
        let identity = identity.ok_or_else(|| "oauth_broker_identity_unavailable".to_string())?;
        return refresh_slack_messaging(
            engine,
            connector_id,
            client_id,
            &current,
            &refresh_token,
            identity,
        );
    }
    refresh_direct(engine, connector_id, client_id, current, &refresh_token)
}
