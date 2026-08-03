use super::{
    broker_configuration, BrokerAuthorizationStartResponse, BrokerHttpResponse, SCHEMA_VERSION,
};
use crate::foundation::clock::unix_time_ms_i64;
use std::collections::HashSet;
use url::Url;

fn query_scope_set(url: &Url, name: &str) -> Option<HashSet<String>> {
    let values = url
        .query_pairs()
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    if values.len() != 1 {
        return None;
    }
    Some(
        values[0]
            .split([',', ' '])
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn valid_broker_attempt_id(value: &str) -> bool {
    (24..=256).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub(super) fn validate_authorization_start(
    decoded: &BrokerAuthorizationStartResponse,
    response: &BrokerHttpResponse,
    client_id: &str,
    requested_user_scopes: &[String],
    requested_bot_scopes: &[String],
) -> Result<(), String> {
    let authorization = Url::parse(&decoded.authorization_url)
        .map_err(|_| "slack_authorization_url_invalid".to_string())?;
    let redirect = authorization
        .query_pairs()
        .find(|(key, _)| key == "redirect_uri")
        .map(|(_, value)| value.into_owned())
        .and_then(|value| Url::parse(&value).ok());
    let broker_host = broker_configuration()?.0.host_str().map(str::to_string);
    let expected_user = requested_user_scopes
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let expected_bot = requested_bot_scopes.iter().cloned().collect::<HashSet<_>>();
    let client_matches = authorization
        .query_pairs()
        .filter(|(key, _)| key == "client_id")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>()
        == [client_id.to_string()];
    let states = authorization
        .query_pairs()
        .filter(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    let state_is_present = states.len() == 1 && !states[0].is_empty();
    let redirect_is_pinned = redirect.as_ref().is_some_and(|value| {
        value.scheme() == "https"
            && value.host_str().map(str::to_string) == broker_host
            && value.username().is_empty()
            && value.password().is_none()
    });
    if decoded.schema_version != SCHEMA_VERSION
        || decoded.request_digest != response.request_digest
        || decoded.nonce != response.nonce
        || decoded.expires_at_ms <= unix_time_ms_i64()
        || decoded.expires_at_ms > unix_time_ms_i64() + 10 * 60 * 1_000
        || !valid_broker_attempt_id(&decoded.broker_attempt_id)
        || decoded.authorization_url.len() > 4_096
        || authorization.scheme() != "https"
        || authorization.host_str() != Some("slack.com")
        || authorization.path() != "/oauth/v2/authorize"
        || authorization.username() != ""
        || authorization.password().is_some()
        || !client_matches
        || !state_is_present
        || !redirect_is_pinned
        || query_scope_set(&authorization, "user_scope") != Some(expected_user)
        || query_scope_set(&authorization, "scope") != Some(expected_bot)
    {
        return Err("slack_authorization_url_invalid".to_string());
    }
    Ok(())
}
