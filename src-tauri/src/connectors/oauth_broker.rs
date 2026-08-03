use super::auth::{ConnectorCredential, OAuthAttemptSecret};
use crate::{
    foundation::{clock::unix_time_ms_i64, digest::sha256_hex},
    sovereign_identity::SovereignIdentity,
};
use rand_core::{OsRng, RngCore};
use rustls::{pki_types::ServerName, ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs},
    sync::Arc,
    time::{Duration, Instant},
};
use url::Url;

mod authorization_validation;
use authorization_validation::validate_authorization_start;

const BROKER_URL: Option<&str> = option_env!("OOMU_SLACK_OAUTH_BROKER_URL");
const BROKER_CERT_SHA256: Option<&str> = option_env!("OOMU_SLACK_OAUTH_BROKER_CERT_SHA256");
const SCHEMA_VERSION: u8 = 2;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrokerRequestPayload<'a> {
    schema_version: u8,
    action: &'a str,
    client_id: &'a str,
    authorization_code: Option<&'a str>,
    refresh_token: Option<&'a str>,
    code_verifier: Option<&'a str>,
    redirect_uri: Option<&'a str>,
    broker_attempt_id: Option<&'a str>,
    requested_user_scopes: Option<&'a [String]>,
    requested_bot_scopes: Option<&'a [String]>,
    nonce: String,
    issued_at_ms: i64,
    expires_at_ms: i64,
    app_version: &'static str,
    installation_state_digest: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SignedBrokerRequest<'a> {
    #[serde(flatten)]
    payload: BrokerRequestPayload<'a>,
    request_digest: String,
    installation_public_key: String,
    signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrokerResponse {
    schema_version: u8,
    request_digest: String,
    nonce: String,
    access_token: String,
    #[serde(default)]
    bot_access_token: Option<String>,
    refresh_token: Option<String>,
    token_type: String,
    scopes: Vec<String>,
    expires_at_ms: Option<i64>,
    refresh_expires_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrokerSocketResponse {
    schema_version: u8,
    request_digest: String,
    nonce: String,
    socket_url: String,
    expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrokerAuthorizationStartResponse {
    schema_version: u8,
    request_digest: String,
    nonce: String,
    broker_attempt_id: String,
    authorization_url: String,
    expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrokerAuthorizationPollResponse {
    schema_version: u8,
    request_digest: String,
    nonce: String,
    state: String,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    bot_access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    expires_at_ms: Option<i64>,
    #[serde(default)]
    refresh_expires_at_ms: Option<i64>,
    #[serde(default)]
    error_code: Option<String>,
}

#[derive(Debug)]
struct BrokerHttpResponse {
    body: Vec<u8>,
    request_digest: String,
    nonce: String,
}

#[derive(Clone, Debug)]
pub(super) struct BrokerAuthorizationStart {
    pub authorization_url: String,
    pub broker_attempt_id: String,
    pub expires_at_ms: i64,
}

pub(super) enum BrokerAuthorizationPoll {
    Pending,
    Complete(ConnectorCredential),
}

pub(super) fn configured() -> bool {
    broker_configuration().is_ok()
}

fn broker_configuration() -> Result<(Url, [u8; 32]), String> {
    let raw_url = BROKER_URL.ok_or_else(|| "oauth_broker_unconfigured".to_string())?;
    let url = Url::parse(raw_url).map_err(|_| "oauth_broker_unconfigured".to_string())?;
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || url.path().len() > 256
    {
        return Err("oauth_broker_unconfigured".to_string());
    }
    let decoded =
        hex::decode(BROKER_CERT_SHA256.ok_or_else(|| "oauth_broker_unconfigured".to_string())?)
            .map_err(|_| "oauth_broker_unconfigured".to_string())?;
    let pin = decoded
        .try_into()
        .map_err(|_| "oauth_broker_unconfigured".to_string())?;
    Ok((url, pin))
}

fn global_address(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified())
        }
        IpAddr::V6(ip) => {
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local())
        }
    }
}

fn pinned_tls_stream(
    url: &Url,
    expected_pin: &[u8; 32],
) -> Result<StreamOwned<ClientConnection, TcpStream>, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "oauth_broker_unconfigured".to_string())?
        .to_string();
    let addresses = (host.as_str(), 443)
        .to_socket_addrs()
        .map_err(|_| "oauth_broker_unreachable".to_string())?
        .filter(|address| global_address(&address.ip()))
        .collect::<Vec<SocketAddr>>();
    let mut tcp = addresses
        .iter()
        .find_map(|address| TcpStream::connect_timeout(address, CONNECT_TIMEOUT).ok())
        .ok_or_else(|| "oauth_broker_unreachable".to_string())?;
    tcp.set_read_timeout(Some(IO_TIMEOUT))
        .and_then(|_| tcp.set_write_timeout(Some(IO_TIMEOUT)))
        .map_err(|_| "oauth_broker_unreachable".to_string())?;
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let server_name =
        ServerName::try_from(host).map_err(|_| "oauth_broker_unconfigured".to_string())?;
    let mut connection = ClientConnection::new(Arc::new(config), server_name)
        .map_err(|_| "oauth_broker_unreachable".to_string())?;
    connection
        .complete_io(&mut tcp)
        .map_err(|_| "oauth_broker_unreachable".to_string())?;
    let certificate = connection
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .ok_or_else(|| "oauth_broker_pin_mismatch".to_string())?;
    if Sha256::digest(certificate.as_ref()).as_slice() != expected_pin {
        return Err("oauth_broker_pin_mismatch".to_string());
    }
    Ok(StreamOwned::new(connection, tcp))
}

fn random_nonce() -> String {
    let mut bytes = [0_u8; 24];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn complete_http_response_length(response: &[u8]) -> Result<Option<usize>, String> {
    let Some(split) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Ok(None);
    };
    let head = std::str::from_utf8(&response[..split])
        .map_err(|_| "oauth_broker_response_invalid".to_string())?;
    if head
        .lines()
        .any(|line| line.to_ascii_lowercase().starts_with("transfer-encoding:"))
    {
        return Err("oauth_broker_response_invalid".to_string());
    }
    let lengths = head
        .lines()
        .filter_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim())
            })
        })
        .collect::<Vec<_>>();
    if lengths.len() != 1 {
        return Err("oauth_broker_response_invalid".to_string());
    }
    let body_length = lengths[0]
        .parse::<usize>()
        .ok()
        .filter(|length| *length <= MAX_RESPONSE_BYTES)
        .ok_or_else(|| "oauth_broker_response_invalid".to_string())?;
    let total = split
        .checked_add(4)
        .and_then(|length| length.checked_add(body_length))
        .filter(|length| *length <= MAX_RESPONSE_BYTES)
        .ok_or_else(|| "oauth_broker_response_invalid".to_string())?;
    Ok((response.len() >= total).then_some(total))
}

fn read_bounded_http_response(
    stream: &mut StreamOwned<ClientConnection, TcpStream>,
) -> Result<Vec<u8>, String> {
    let started = Instant::now();
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let remaining = TOTAL_REQUEST_TIMEOUT
            .checked_sub(started.elapsed())
            .ok_or_else(|| "oauth_broker_unreachable".to_string())?;
        stream
            .sock
            .set_read_timeout(Some(remaining.min(IO_TIMEOUT)))
            .map_err(|_| "oauth_broker_unreachable".to_string())?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                response.extend_from_slice(&chunk[..read]);
                if response.len() > MAX_RESPONSE_BYTES {
                    return Err("oauth_broker_response_invalid".to_string());
                }
                if let Some(total) = complete_http_response_length(&response)? {
                    if response.len() != total {
                        return Err("oauth_broker_response_invalid".to_string());
                    }
                    return Ok(response);
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if started.elapsed() >= TOTAL_REQUEST_TIMEOUT {
                    return Err("oauth_broker_unreachable".to_string());
                }
            }
            Err(_) => return Err("oauth_broker_unreachable".to_string()),
        }
    }
    if let Some(total) = complete_http_response_length(&response)? {
        if response.len() == total {
            return Ok(response);
        }
    }
    Err("oauth_broker_response_invalid".to_string())
}

fn post_signed_request(
    payload: BrokerRequestPayload<'_>,
    identity: &SovereignIdentity,
) -> Result<BrokerHttpResponse, String> {
    let (url, pin) = broker_configuration()?;
    let canonical =
        serde_json::to_string(&payload).map_err(|_| "oauth_broker_request_invalid".to_string())?;
    let request_digest = sha256_hex(canonical.as_bytes());
    let nonce = payload.nonce.clone();
    let signature = identity
        .sign_payload(&canonical)
        .map_err(|_| "oauth_broker_identity_unavailable".to_string())?;
    let request = SignedBrokerRequest {
        payload,
        request_digest: request_digest.clone(),
        installation_public_key: signature.public_key,
        signature: signature.signature,
    };
    let body =
        serde_json::to_vec(&request).map_err(|_| "oauth_broker_request_invalid".to_string())?;
    if body.len() > 32 * 1024 {
        return Err("oauth_broker_request_invalid".to_string());
    }
    let mut stream = pinned_tls_stream(&url, &pin)?;
    let host = url.host_str().unwrap_or_default();
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let request_head = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(request_head.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .and_then(|_| stream.flush())
        .map_err(|_| "oauth_broker_unreachable".to_string())?;
    let response = read_bounded_http_response(&mut stream)?;
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "oauth_broker_response_invalid".to_string())?;
    let head = std::str::from_utf8(&response[..split])
        .map_err(|_| "oauth_broker_response_invalid".to_string())?;
    if !head
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("HTTP/1.1 200 "))
    {
        return Err("oauth_broker_rejected".to_string());
    }
    Ok(BrokerHttpResponse {
        body: response[split + 4..].to_vec(),
        request_digest,
        nonce,
    })
}

fn refresh_request(
    secret: &OAuthAttemptSecret,
    refresh_token: &str,
    identity: &SovereignIdentity,
) -> Result<ConnectorCredential, String> {
    let issued_at_ms = unix_time_ms_i64();
    let payload = BrokerRequestPayload {
        schema_version: SCHEMA_VERSION,
        action: "refresh",
        client_id: &secret.client_id,
        authorization_code: None,
        refresh_token: Some(refresh_token),
        code_verifier: None,
        redirect_uri: None,
        broker_attempt_id: None,
        requested_user_scopes: None,
        requested_bot_scopes: None,
        nonce: random_nonce(),
        issued_at_ms,
        expires_at_ms: issued_at_ms + 30_000,
        app_version: env!("CARGO_PKG_VERSION"),
        installation_state_digest: sha256_hex(
            format!("{}:{}", secret.connector_id, secret.manifest_id).as_bytes(),
        ),
    };
    let response = post_signed_request(payload, identity)?;
    let decoded: BrokerResponse = serde_json::from_slice(&response.body)
        .map_err(|_| "oauth_broker_response_invalid".to_string())?;
    if decoded.schema_version != SCHEMA_VERSION
        || decoded.request_digest != response.request_digest
        || decoded.nonce != response.nonce
        || decoded.access_token.is_empty()
        || decoded.access_token.len() > 16_384
        || decoded
            .bot_access_token
            .as_ref()
            .is_some_and(|token| token.is_empty() || token.len() > 16_384)
        || decoded
            .refresh_token
            .as_ref()
            .is_some_and(|token| token.len() > 16_384)
        || decoded.scopes.len() > 128
        || decoded.scopes.iter().any(|scope| scope.len() > 256)
        || !decoded.token_type.eq_ignore_ascii_case("bearer")
    {
        return Err("oauth_broker_response_invalid".to_string());
    }
    Ok(ConnectorCredential {
        manifest_id: "slack".to_string(),
        access_token: decoded.access_token,
        bot_access_token: decoded.bot_access_token,
        refresh_token: decoded.refresh_token,
        token_type: decoded.token_type,
        scopes: decoded.scopes,
        expires_at_ms: decoded.expires_at_ms,
        refresh_expires_at_ms: decoded.refresh_expires_at_ms,
        tenant_id: None,
        tenant_label: None,
        account_id: None,
        account_principal: None,
        identity_binding_hash: None,
    })
}

pub(super) fn begin_authorization(
    connector_id: &str,
    client_id: &str,
    requested_user_scopes: &[String],
    requested_bot_scopes: &[String],
    identity: &SovereignIdentity,
) -> Result<BrokerAuthorizationStart, String> {
    if requested_bot_scopes.is_empty() {
        return Err("slack_messaging_scopes_required".to_string());
    }
    let issued_at_ms = unix_time_ms_i64();
    let response = post_signed_request(
        BrokerRequestPayload {
            schema_version: SCHEMA_VERSION,
            action: "authorization_start",
            client_id,
            authorization_code: None,
            refresh_token: None,
            code_verifier: None,
            redirect_uri: None,
            broker_attempt_id: None,
            requested_user_scopes: Some(requested_user_scopes),
            requested_bot_scopes: Some(requested_bot_scopes),
            nonce: random_nonce(),
            issued_at_ms,
            expires_at_ms: issued_at_ms + 30_000,
            app_version: env!("CARGO_PKG_VERSION"),
            installation_state_digest: sha256_hex(format!("{connector_id}:slack").as_bytes()),
        },
        identity,
    )?;
    let decoded: BrokerAuthorizationStartResponse = serde_json::from_slice(&response.body)
        .map_err(|_| "oauth_broker_response_invalid".to_string())?;
    validate_authorization_start(
        &decoded,
        &response,
        client_id,
        requested_user_scopes,
        requested_bot_scopes,
    )?;
    Ok(BrokerAuthorizationStart {
        authorization_url: decoded.authorization_url,
        broker_attempt_id: decoded.broker_attempt_id,
        expires_at_ms: decoded.expires_at_ms,
    })
}

fn required_bounded_token(value: Option<String>) -> Result<String, String> {
    value
        .filter(|token| !token.is_empty() && token.len() <= 16_384)
        .ok_or_else(|| "oauth_broker_response_invalid".to_string())
}

fn completed_authorization_credential(
    decoded: BrokerAuthorizationPollResponse,
) -> Result<ConnectorCredential, String> {
    let access_token = required_bounded_token(decoded.access_token)?;
    let bot_access_token = required_bounded_token(decoded.bot_access_token)?;
    let token_type = decoded
        .token_type
        .filter(|value| value.eq_ignore_ascii_case("bearer"))
        .ok_or_else(|| "oauth_broker_response_invalid".to_string())?;
    if decoded.scopes.len() > 128
        || decoded.scopes.iter().any(|scope| scope.len() > 256)
        || decoded
            .refresh_token
            .as_ref()
            .is_some_and(|token| token.is_empty() || token.len() > 16_384)
        || !decoded.scopes.iter().any(|scope| scope == "chat:write")
    {
        return Err("oauth_broker_response_invalid".to_string());
    }
    Ok(ConnectorCredential {
        manifest_id: "slack".to_string(),
        access_token,
        bot_access_token: Some(bot_access_token),
        refresh_token: decoded.refresh_token,
        token_type,
        scopes: decoded.scopes,
        expires_at_ms: decoded.expires_at_ms,
        refresh_expires_at_ms: decoded.refresh_expires_at_ms,
        tenant_id: None,
        tenant_label: None,
        account_id: None,
        account_principal: None,
        identity_binding_hash: None,
    })
}

fn authorization_poll_result(
    decoded: BrokerAuthorizationPollResponse,
) -> Result<BrokerAuthorizationPoll, String> {
    match decoded.state.as_str() {
        "pending" => Ok(BrokerAuthorizationPoll::Pending),
        "complete" => {
            completed_authorization_credential(decoded).map(BrokerAuthorizationPoll::Complete)
        }
        "failed" => Err(match decoded.error_code.as_deref() {
            Some("access_denied") => "slack_authorization_access_denied",
            Some("invalid_scope") => "slack_authorization_invalid_scope",
            Some("workspace_restricted") => "slack_authorization_workspace_restricted",
            Some("expired") => "slack_authorization_expired",
            _ => "slack_authorization_rejected",
        }
        .to_string()),
        _ => Err("oauth_broker_response_invalid".to_string()),
    }
}

pub(super) fn poll_authorization(
    connector_id: &str,
    client_id: &str,
    broker_attempt_id: &str,
    identity: &SovereignIdentity,
) -> Result<BrokerAuthorizationPoll, String> {
    let issued_at_ms = unix_time_ms_i64();
    let response = post_signed_request(
        BrokerRequestPayload {
            schema_version: SCHEMA_VERSION,
            action: "authorization_poll",
            client_id,
            authorization_code: None,
            refresh_token: None,
            code_verifier: None,
            redirect_uri: None,
            broker_attempt_id: Some(broker_attempt_id),
            requested_user_scopes: None,
            requested_bot_scopes: None,
            nonce: random_nonce(),
            issued_at_ms,
            expires_at_ms: issued_at_ms + 30_000,
            app_version: env!("CARGO_PKG_VERSION"),
            installation_state_digest: sha256_hex(format!("{connector_id}:slack").as_bytes()),
        },
        identity,
    )?;
    let decoded: BrokerAuthorizationPollResponse = serde_json::from_slice(&response.body)
        .map_err(|_| "oauth_broker_response_invalid".to_string())?;
    if decoded.schema_version != SCHEMA_VERSION
        || decoded.request_digest != response.request_digest
        || decoded.nonce != response.nonce
    {
        return Err("oauth_broker_response_invalid".to_string());
    }
    authorization_poll_result(decoded)
}

pub(super) fn open_socket(
    connector_id: &str,
    client_id: &str,
    identity: &SovereignIdentity,
) -> Result<String, String> {
    let issued_at_ms = unix_time_ms_i64();
    let payload = BrokerRequestPayload {
        schema_version: SCHEMA_VERSION,
        action: "socket_open",
        client_id,
        authorization_code: None,
        refresh_token: None,
        code_verifier: None,
        redirect_uri: None,
        broker_attempt_id: None,
        requested_user_scopes: None,
        requested_bot_scopes: None,
        nonce: random_nonce(),
        issued_at_ms,
        expires_at_ms: issued_at_ms + 30_000,
        app_version: env!("CARGO_PKG_VERSION"),
        installation_state_digest: sha256_hex(format!("{connector_id}:slack").as_bytes()),
    };
    let response = post_signed_request(payload, identity)?;
    let decoded: BrokerSocketResponse = serde_json::from_slice(&response.body)
        .map_err(|_| "oauth_broker_response_invalid".to_string())?;
    let socket_url =
        Url::parse(&decoded.socket_url).map_err(|_| "slack_socket_url_invalid".to_string())?;
    let allowed_host = socket_url
        .host_str()
        .is_some_and(|host| host == "slack.com" || host.ends_with(".slack.com"));
    if decoded.schema_version != SCHEMA_VERSION
        || decoded.request_digest != response.request_digest
        || decoded.nonce != response.nonce
        || decoded.expires_at_ms <= unix_time_ms_i64()
        || decoded.expires_at_ms > unix_time_ms_i64() + 120_000
        || decoded.socket_url.len() > 4_096
        || socket_url.scheme() != "wss"
        || !allowed_host
        || socket_url.username() != ""
        || socket_url.password().is_some()
    {
        return Err("slack_socket_url_invalid".to_string());
    }
    Ok(decoded.socket_url)
}

pub(super) fn refresh(
    connector_id: &str,
    client_id: &str,
    current: &ConnectorCredential,
    refresh_token: &str,
    identity: &SovereignIdentity,
) -> Result<ConnectorCredential, String> {
    let secret = OAuthAttemptSecret {
        connector_id: connector_id.to_string(),
        manifest_id: "slack".to_string(),
        client_id: client_id.to_string(),
        state: String::new(),
        verifier: String::new(),
        nonce: String::new(),
        redirect_uri: String::new(),
        expires_at_ms: current.expires_at_ms.unwrap_or_default(),
        requested_scopes: current.scopes.clone(),
        created_new_account: false,
        broker_attempt_id: None,
    };
    refresh_request(&secret, refresh_token, identity)
}

#[cfg(test)]
mod tests;
