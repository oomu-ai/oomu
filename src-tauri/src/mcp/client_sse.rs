use crate::mcp::client::{
    parse_json_rpc_message, JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
    McpClientError,
};
use crate::mcp::shield::{sanitize_outgoing_payload_for_transport, McpTransportConfig};
#[cfg(test)]
use crate::network_policy::resolve_destination;
use crate::network_policy::{
    revalidate_destination, validate_connected_peer, validate_redirect_destination,
    CanonicalDestination, ResolvedDestinationClass,
};
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use reqwest::{redirect::Policy, StatusCode};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::time::{sleep, timeout, Instant};

const SSE_CONTENT_TYPE: &str = "text/event-stream";
const JSON_CONTENT_TYPE: &str = "application/json";
pub const REMOTE_RESPONSE_BYTE_LIMIT: usize = 1_048_576;
pub const REMOTE_REQUEST_BYTE_LIMIT: usize = 262_144;
pub const REMOTE_SSE_EVENT_BYTE_LIMIT: usize = 262_144;
pub const REMOTE_REDIRECT_LIMIT: usize = 3;
#[cfg(not(test))]
const REMOTE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const REMOTE_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(not(test))]
const REMOTE_FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(test)]
const REMOTE_FIRST_BYTE_TIMEOUT: Duration = Duration::from_millis(350);
#[cfg(not(test))]
const REMOTE_READ_IDLE_TIMEOUT: Duration = Duration::from_secs(20);
#[cfg(test)]
const REMOTE_READ_IDLE_TIMEOUT: Duration = Duration::from_millis(350);
#[cfg(not(test))]
const REMOTE_TOTAL_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
const REMOTE_TOTAL_TIMEOUT: Duration = Duration::from_secs(4);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

pub struct RemoteTransportClient {
    bootstrap_client: reqwest::Client,
    destination: CanonicalDestination,
    certificate_state: StdMutex<CertificateState>,
    terminal_policy_failure: Arc<AtomicBool>,
    additional_tls_roots: Vec<CertificateDer<'static>>,
}

struct CertificateState {
    binding: Option<String>,
    pinned_client: Option<reqwest::Client>,
}

struct ObservedCertificate {
    binding: String,
    leaf_der: Option<Vec<u8>>,
}

#[derive(Debug)]
struct CertificatePinVerifier {
    standard_verifier: Arc<dyn ServerCertVerifier>,
    expected_leaf_sha256: [u8; 32],
    terminal_policy_failure: Arc<AtomicBool>,
}

impl ServerCertVerifier for CertificatePinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let verified = self.standard_verifier.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        );
        let verified = match verified {
            Ok(verified) => verified,
            Err(error) => {
                self.terminal_policy_failure.store(true, Ordering::Release);
                return Err(error);
            }
        };

        let observed = Sha256::digest(end_entity.as_ref());
        if observed.as_slice() != self.expected_leaf_sha256 {
            self.terminal_policy_failure.store(true, Ordering::Release);
            return Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        Ok(verified)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.standard_verifier
            .verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.standard_verifier
            .verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.standard_verifier.supported_verify_schemes()
    }
}

impl RemoteTransportClient {
    pub fn destination(&self) -> &CanonicalDestination {
        &self.destination
    }

    pub fn certificate_binding(&self) -> Result<String, McpClientError> {
        self.ensure_certificate_policy_active()?;
        self.certificate_state
            .lock()
            .map_err(|_| {
                McpClientError::transport(
                    "Remote MCP certificate binding state is unavailable.".to_string(),
                )
            })?
            .binding
            .clone()
            .ok_or_else(|| {
                McpClientError::permission(
                    "Remote MCP TLS certificate has not been bound by initialization.".to_string(),
                )
            })
    }

    fn validate_response_certificate(
        &self,
        response: &reqwest::Response,
    ) -> Result<(), McpClientError> {
        self.ensure_certificate_policy_active()?;
        let observed = observed_certificate_binding(self.destination.url().scheme(), response)?;
        let mut state = self.certificate_state.lock().map_err(|_| {
            McpClientError::transport(
                "Remote MCP certificate binding state is unavailable.".to_string(),
            )
        })?;
        match state.binding.as_ref() {
            Some(approved) if approved != &observed.binding => {
                self.terminal_policy_failure.store(true, Ordering::Release);
                Err(certificate_policy_changed_error())
            }
            Some(_) => Ok(()),
            None => {
                let pinned_client = if self.destination.url().scheme() == "https" {
                    let leaf_der = observed.leaf_der.as_deref().ok_or_else(|| {
                        self.terminal_policy_failure.store(true, Ordering::Release);
                        McpClientError::permission(
                            "Remote MCP HTTPS response did not expose a peer certificate for binding."
                                .to_string(),
                        )
                    })?;
                    let expected_leaf_sha256: [u8; 32] = Sha256::digest(leaf_der).into();
                    Some(build_policy_client(
                        &self.destination,
                        Some(expected_leaf_sha256),
                        &self.terminal_policy_failure,
                        &self.additional_tls_roots,
                    )?)
                } else {
                    None
                };
                state.binding = Some(observed.binding);
                state.pinned_client = pinned_client;
                Ok(())
            }
        }
    }

    fn request_client(
        &self,
        allow_unpinned_bootstrap: bool,
    ) -> Result<reqwest::Client, McpClientError> {
        self.ensure_certificate_policy_active()?;
        if self.destination.url().scheme() == "http" {
            return Ok(self.bootstrap_client.clone());
        }
        let state = self.certificate_state.lock().map_err(|_| {
            McpClientError::transport(
                "Remote MCP certificate binding state is unavailable.".to_string(),
            )
        })?;
        match (&state.binding, &state.pinned_client) {
            (Some(_), Some(client)) => Ok(client.clone()),
            (None, None) if allow_unpinned_bootstrap => Ok(self.bootstrap_client.clone()),
            (None, None) => Err(McpClientError::permission(
                "Remote MCP HTTPS transport cannot send non-initialization data before its certificate is bound."
                    .to_string(),
            )),
            _ => {
                self.terminal_policy_failure.store(true, Ordering::Release);
                Err(certificate_policy_changed_error())
            }
        }
    }

    fn ensure_certificate_policy_active(&self) -> Result<(), McpClientError> {
        if self.terminal_policy_failure.load(Ordering::Acquire) {
            return Err(certificate_policy_changed_error());
        }
        Ok(())
    }
}

fn certificate_policy_changed_error() -> McpClientError {
    McpClientError::permission(
        "Remote MCP TLS certificate changed; the transport and all prior authority are revoked."
            .to_string(),
    )
}

pub async fn build_remote_transport_client(
    transport: &McpTransportConfig,
    approved_destination: &CanonicalDestination,
    cancellation: &Arc<AtomicBool>,
) -> Result<RemoteTransportClient, McpClientError> {
    build_remote_transport_client_with_roots(
        transport,
        approved_destination,
        cancellation,
        Vec::new(),
    )
    .await
}

async fn build_remote_transport_client_with_roots(
    transport: &McpTransportConfig,
    approved_destination: &CanonicalDestination,
    cancellation: &Arc<AtomicBool>,
    additional_tls_roots: Vec<CertificateDer<'static>>,
) -> Result<RemoteTransportClient, McpClientError> {
    let endpoint = transport.endpoint().ok_or_else(|| {
        McpClientError::transport(
            "Remote MCP transport client cannot be built for a local route.".to_string(),
        )
    })?;
    let destination_transport = transport.destination_transport().ok_or_else(|| {
        McpClientError::transport("Remote MCP transport type is unavailable.".to_string())
    })?;

    if approved_destination.transport() != destination_transport
        || approved_destination.canonical_url() != endpoint
    {
        return Err(McpClientError::permission(
            "Remote MCP destination does not match its Shield approval binding.".to_string(),
        ));
    }
    let destination = await_with_cancellation(
        revalidate_destination(approved_destination),
        REMOTE_CONNECT_TIMEOUT,
        cancellation,
        || {
            McpClientError::transport(
                "Remote MCP destination build revalidation exceeded its deadline.".to_string(),
            )
        },
    )
    .await?
    .map_err(network_policy_error)?;
    ensure_not_cancelled("remote initialization", cancellation)?;

    let terminal_policy_failure = Arc::new(AtomicBool::new(false));
    let bootstrap_client = build_policy_client(
        &destination,
        None,
        &terminal_policy_failure,
        &additional_tls_roots,
    )?;

    Ok(RemoteTransportClient {
        bootstrap_client,
        destination,
        certificate_state: StdMutex::new(CertificateState {
            binding: None,
            pinned_client: None,
        }),
        terminal_policy_failure,
        additional_tls_roots,
    })
}

fn build_policy_client(
    destination: &CanonicalDestination,
    expected_leaf_sha256: Option<[u8; 32]>,
    terminal_policy_failure: &Arc<AtomicBool>,
    additional_tls_roots: &[CertificateDer<'static>],
) -> Result<reqwest::Client, McpClientError> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .tls_info(true)
        .connect_timeout(REMOTE_CONNECT_TIMEOUT)
        .read_timeout(REMOTE_READ_IDLE_TIMEOUT)
        .timeout(REMOTE_TOTAL_TIMEOUT)
        .pool_idle_timeout(Duration::from_secs(15))
        .tcp_keepalive(Duration::from_secs(15));

    if let Some(expected_leaf_sha256) = expected_leaf_sha256 {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        for root in additional_tls_roots {
            roots.add(root.clone()).map_err(|_| {
                McpClientError::transport(
                    "Failed to configure an additional remote MCP TLS trust anchor.".to_string(),
                )
            })?;
        }
        let standard_verifier = WebPkiServerVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|_| {
                McpClientError::transport(
                    "Failed to build the standard remote MCP TLS verifier.".to_string(),
                )
            })?;
        let pin_verifier = Arc::new(CertificatePinVerifier {
            standard_verifier,
            expected_leaf_sha256,
            terminal_policy_failure: terminal_policy_failure.clone(),
        });
        let mut tls_config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(pin_verifier)
            .with_no_client_auth();
        tls_config.alpn_protocols = vec![b"http/1.1".to_vec()];
        builder = builder.use_preconfigured_tls(tls_config);
    } else {
        builder = builder.use_rustls_tls();
        for root in additional_tls_roots {
            let certificate = reqwest::Certificate::from_der(root.as_ref()).map_err(|_| {
                McpClientError::transport(
                    "Failed to configure an additional remote MCP TLS trust anchor.".to_string(),
                )
            })?;
            builder = builder.add_root_certificate(certificate);
        }
    }
    if destination.destination_class() == ResolvedDestinationClass::Public {
        builder = builder.https_only(true);
    }
    builder =
        builder.resolve_to_addrs(destination.host(), &destination.resolved_socket_addresses());
    builder.build().map_err(|error| {
        McpClientError::transport(format!(
            "Failed to build policy-pinned remote MCP client: {}",
            crate::redaction::redact_network_error(&error.to_string())
        ))
    })
}

pub async fn revalidate_remote_destination(
    destination: &CanonicalDestination,
    cancellation: &Arc<AtomicBool>,
) -> Result<CanonicalDestination, McpClientError> {
    await_with_cancellation(
        revalidate_destination(destination),
        REMOTE_CONNECT_TIMEOUT,
        cancellation,
        || {
            McpClientError::transport(
                "Remote MCP authority revalidation exceeded its deadline.".to_string(),
            )
        },
    )
    .await?
    .map_err(network_policy_error)
}

#[cfg(test)]
async fn build_test_remote_transport_client(
    transport: &McpTransportConfig,
) -> Result<RemoteTransportClient, McpClientError> {
    build_test_remote_transport_client_with_roots(transport, Vec::new()).await
}

#[cfg(test)]
async fn build_test_remote_transport_client_with_roots(
    transport: &McpTransportConfig,
    additional_tls_roots: Vec<CertificateDer<'static>>,
) -> Result<RemoteTransportClient, McpClientError> {
    let endpoint = transport.endpoint().ok_or_else(|| {
        McpClientError::transport("Test remote endpoint is unavailable.".to_string())
    })?;
    let destination = resolve_destination(
        endpoint,
        transport.destination_transport().ok_or_else(|| {
            McpClientError::transport("Test remote transport is unavailable.".to_string())
        })?,
        transport.local_origin_grant(),
    )
    .await
    .map_err(network_policy_error)?;
    let cancellation = Arc::new(AtomicBool::new(false));
    build_remote_transport_client_with_roots(
        transport,
        &destination,
        &cancellation,
        additional_tls_roots,
    )
    .await
}

pub async fn send_remote_request(
    server_name: &str,
    transport: &McpTransportConfig,
    remote: &RemoteTransportClient,
    request: JsonRpcRequest,
    cancellation: &Arc<AtomicBool>,
) -> Result<JsonRpcResponse, McpClientError> {
    let request_id = request.id.clone();
    let allow_unpinned_bootstrap = request.method == "initialize";
    let payload = serialize_and_sanitize(server_name, transport, &request)?;
    let result = timeout(
        REMOTE_TOTAL_TIMEOUT,
        send_remote_request_inner(
            server_name,
            transport,
            remote,
            payload,
            &request_id,
            cancellation,
            allow_unpinned_bootstrap,
        ),
    )
    .await
    .map_err(|_| {
        McpClientError::transport(format!(
            "Remote MCP request to '{server_name}' exceeded its total deadline."
        ))
    })??;

    match result.message {
        JsonRpcMessage::Response(response) if response.id == request_id => {
            log_remote_security_event(
                server_name,
                remote.destination(),
                "request_complete",
                result.received_bytes,
                "allowed",
            );
            Ok(response)
        }
        JsonRpcMessage::Response(_) => Err(McpClientError::protocol(format!(
            "MCP HTTP response id for '{server_name}' did not match the request id."
        ))),
        JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) => {
            Err(McpClientError::protocol(format!(
                "MCP HTTP server '{server_name}' returned a non-response JSON-RPC message."
            )))
        }
    }
}

pub async fn send_remote_notification(
    server_name: &str,
    transport: &McpTransportConfig,
    remote: &RemoteTransportClient,
    notification: JsonRpcNotification,
    cancellation: &Arc<AtomicBool>,
) -> Result<(), McpClientError> {
    let payload = serialize_and_sanitize(server_name, transport, &notification)?;
    timeout(
        REMOTE_TOTAL_TIMEOUT,
        send_remote_notification_inner(server_name, remote, payload, cancellation, false),
    )
    .await
    .map_err(|_| {
        McpClientError::transport(format!(
            "Remote MCP notification to '{server_name}' exceeded its total deadline."
        ))
    })??;
    Ok(())
}

struct ParsedRemoteResponse {
    message: JsonRpcMessage,
    received_bytes: usize,
}

async fn send_remote_request_inner(
    server_name: &str,
    transport: &McpTransportConfig,
    remote: &RemoteTransportClient,
    payload: String,
    request_id: &Value,
    cancellation: &Arc<AtomicBool>,
    allow_unpinned_bootstrap: bool,
) -> Result<ParsedRemoteResponse, McpClientError> {
    let response = dispatch_with_policy(
        server_name,
        remote,
        payload,
        cancellation,
        allow_unpinned_bootstrap,
    )
    .await?;
    let status = response.status();
    let is_sse = response_is_sse(&response) || matches!(transport, McpTransportConfig::Sse { .. });
    let body = read_bounded_response(server_name, response, cancellation).await?;
    if !status.is_success() {
        return Err(McpClientError::transport(format!(
            "MCP HTTP server '{server_name}' returned status {status}."
        )));
    }
    let text = String::from_utf8(body).map_err(|_| {
        McpClientError::protocol(format!(
            "MCP HTTP response from '{server_name}' was not valid UTF-8."
        ))
    })?;
    let received_bytes = text.len();
    let message = if is_sse {
        parse_sse_text_bounded(server_name, &text, request_id)?
    } else {
        parse_json_rpc_or_sse_text_bounded(server_name, &text, request_id)?
    };
    Ok(ParsedRemoteResponse {
        message,
        received_bytes,
    })
}

async fn send_remote_notification_inner(
    server_name: &str,
    remote: &RemoteTransportClient,
    payload: String,
    cancellation: &Arc<AtomicBool>,
    allow_unpinned_bootstrap: bool,
) -> Result<(), McpClientError> {
    let response = dispatch_with_policy(
        server_name,
        remote,
        payload,
        cancellation,
        allow_unpinned_bootstrap,
    )
    .await?;
    let status = response.status();
    let bytes = read_bounded_response(server_name, response, cancellation).await?;
    if !status.is_success() {
        return Err(McpClientError::transport(format!(
            "MCP HTTP server '{server_name}' rejected a notification with status {status}."
        )));
    }
    log_remote_security_event(
        server_name,
        remote.destination(),
        "notification_complete",
        bytes.len(),
        "allowed",
    );
    Ok(())
}

async fn dispatch_with_policy(
    server_name: &str,
    remote: &RemoteTransportClient,
    payload: String,
    cancellation: &Arc<AtomicBool>,
    allow_unpinned_bootstrap: bool,
) -> Result<reqwest::Response, McpClientError> {
    ensure_not_cancelled(server_name, cancellation)?;
    let mut destination = await_with_cancellation(
        revalidate_destination(remote.destination()),
        REMOTE_CONNECT_TIMEOUT,
        cancellation,
        || {
            McpClientError::transport(format!(
                "Remote MCP destination revalidation for '{server_name}' exceeded its deadline."
            ))
        },
    )
    .await?
    .map_err(network_policy_error)?;

    for redirect_count in 0..=REMOTE_REDIRECT_LIMIT {
        ensure_not_cancelled(server_name, cancellation)?;
        // Reselect on every iteration. The first initialize response binds the
        // leaf, so even its redirect can no longer use the bootstrap client.
        let request_client = remote.request_client(allow_unpinned_bootstrap)?;
        let send = request_client
            .post(destination.url().clone())
            .header(CONTENT_TYPE, JSON_CONTENT_TYPE)
            .header(ACCEPT, "application/json, text/event-stream")
            .body(payload.clone())
            .send();
        let response =
            await_with_cancellation(send, REMOTE_FIRST_BYTE_TIMEOUT, cancellation, || {
                McpClientError::transport(format!(
                    "Timed out waiting for the first response byte from MCP server '{server_name}'."
                ))
            })
            .await?
            .map_err(|error| {
                if remote.terminal_policy_failure.load(Ordering::Acquire) {
                    certificate_policy_changed_error()
                } else if error.is_timeout() {
                    McpClientError::transport(format!(
                    "Timed out waiting for the first response byte from MCP server '{server_name}'."
                ))
                } else {
                    McpClientError::transport(format!(
                        "Failed to dispatch policy-bound MCP request to '{server_name}': {}",
                        crate::redaction::redact_network_error(&error.to_string())
                    ))
                }
            })?;

        validate_connected_peer(&destination, response.remote_addr())
            .map_err(network_policy_error)?;
        remote.validate_response_certificate(&response)?;
        ensure_content_length(server_name, &response)?;

        if !response.status().is_redirection() {
            return Ok(response);
        }
        if redirect_count == REMOTE_REDIRECT_LIMIT {
            return Err(McpClientError::transport(format!(
                "MCP server '{server_name}' exceeded the redirect limit."
            )));
        }
        if !matches!(
            response.status(),
            StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT
        ) {
            return Err(McpClientError::permission(format!(
                "MCP server '{server_name}' attempted a method-changing redirect."
            )));
        }
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                McpClientError::protocol(format!(
                    "MCP server '{server_name}' returned a redirect without a valid Location header."
                ))
            })?;
        let redirect_url = destination.url().join(location).map_err(|_| {
            McpClientError::protocol(format!(
                "MCP server '{server_name}' returned a malformed redirect target."
            ))
        })?;
        destination = await_with_cancellation(
            validate_redirect_destination(remote.destination(), redirect_url.as_str()),
            REMOTE_CONNECT_TIMEOUT,
            cancellation,
            || {
                McpClientError::transport(format!(
                    "Remote MCP redirect revalidation for '{server_name}' exceeded its deadline."
                ))
            },
        )
        .await?
        .map_err(network_policy_error)?;
    }
    Err(McpClientError::transport(format!(
        "MCP server '{server_name}' redirect handling terminated unexpectedly."
    )))
}

fn ensure_content_length(
    server_name: &str,
    response: &reqwest::Response,
) -> Result<(), McpClientError> {
    if let Some(length) = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        if length > REMOTE_RESPONSE_BYTE_LIMIT as u64 {
            return Err(McpClientError::protocol(format!(
                "MCP response from '{server_name}' exceeded the response size limit before buffering."
            )));
        }
    }
    Ok(())
}

async fn read_bounded_response(
    server_name: &str,
    response: reqwest::Response,
    cancellation: &Arc<AtomicBool>,
) -> Result<Vec<u8>, McpClientError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    loop {
        let next = await_with_cancellation(
            stream.next(),
            REMOTE_READ_IDLE_TIMEOUT,
            cancellation,
            || {
                McpClientError::transport(format!(
                    "Timed out waiting for the MCP response body from '{server_name}'."
                ))
            },
        )
        .await?;
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|error| {
            McpClientError::transport(format!(
                "Failed to stream MCP response from '{server_name}': {}",
                crate::redaction::redact_network_error(&error.to_string())
            ))
        })?;
        if body.len().saturating_add(chunk.len()) > REMOTE_RESPONSE_BYTE_LIMIT {
            return Err(McpClientError::protocol(format!(
                "MCP response from '{server_name}' exceeded the streaming byte limit."
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn await_with_cancellation<F, T, E>(
    future: F,
    deadline: Duration,
    cancellation: &Arc<AtomicBool>,
    timeout_error: E,
) -> Result<T, McpClientError>
where
    F: Future<Output = T>,
    E: FnOnce() -> McpClientError,
{
    let mut future = Box::pin(future);
    let started = Instant::now();
    loop {
        ensure_not_cancelled("remote", cancellation)?;
        tokio::select! {
            output = &mut future => return Ok(output),
            _ = sleep(CANCELLATION_POLL_INTERVAL) => {
                if started.elapsed() >= deadline {
                    return Err(timeout_error());
                }
            }
        }
    }
}

fn ensure_not_cancelled(
    server_name: &str,
    cancellation: &Arc<AtomicBool>,
) -> Result<(), McpClientError> {
    if cancellation.load(Ordering::Acquire) {
        return Err(McpClientError::cancelled(format!(
            "MCP network operation for '{server_name}' was cancelled."
        )));
    }
    Ok(())
}

fn serialize_and_sanitize<T: serde::Serialize>(
    server_name: &str,
    transport: &McpTransportConfig,
    message: &T,
) -> Result<String, McpClientError> {
    let payload = serde_json::to_string(message).map_err(|error| {
        McpClientError::protocol(format!(
            "Failed to serialize MCP request for '{server_name}': {error}"
        ))
    })?;
    if payload.len() > REMOTE_REQUEST_BYTE_LIMIT {
        return Err(McpClientError::protocol(format!(
            "MCP request for '{server_name}' exceeded the outbound byte limit before transmission."
        )));
    }
    let sanitized =
        sanitize_outgoing_payload_for_transport(&payload, transport).map_err(|error| {
            McpClientError::transport(format!(
                "MCP request for '{server_name}' failed routing shield checks: {error}"
            ))
        })?;
    if sanitized.len() > REMOTE_REQUEST_BYTE_LIMIT {
        return Err(McpClientError::protocol(format!(
            "MCP request for '{server_name}' exceeded the outbound byte limit after redaction."
        )));
    }
    Ok(sanitized)
}

fn parse_json_rpc_or_sse_text_bounded(
    server_name: &str,
    body: &str,
    request_id: &Value,
) -> Result<JsonRpcMessage, McpClientError> {
    if let Ok(message) = parse_json_rpc_message(body) {
        return Ok(message);
    }
    parse_sse_text_bounded(server_name, body, request_id)
}

fn parse_sse_text_bounded(
    server_name: &str,
    body: &str,
    request_id: &Value,
) -> Result<JsonRpcMessage, McpClientError> {
    for event in body.replace("\r\n", "\n").split("\n\n") {
        if event.len() > REMOTE_SSE_EVENT_BYTE_LIMIT {
            return Err(McpClientError::protocol(format!(
                "MCP SSE event from '{server_name}' exceeded the event size limit."
            )));
        }
        if let Some(message) = parse_sse_event(server_name, event, request_id)? {
            return Ok(message);
        }
    }
    Err(McpClientError::protocol(format!(
        "MCP response from '{server_name}' did not contain a matching JSON-RPC response."
    )))
}

fn parse_sse_event(
    server_name: &str,
    event: &str,
    request_id: &Value,
) -> Result<Option<JsonRpcMessage>, McpClientError> {
    let data = event
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("data:").map(str::trim))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() || data == "[DONE]" {
        return Ok(None);
    }
    match parse_json_rpc_message(&data).map_err(|_| {
        McpClientError::protocol(format!(
            "MCP SSE event from '{server_name}' was not a valid bounded JSON-RPC message."
        ))
    })? {
        JsonRpcMessage::Response(response) if response.id == *request_id => {
            Ok(Some(JsonRpcMessage::Response(response)))
        }
        JsonRpcMessage::Response(_) => Err(McpClientError::protocol(format!(
            "MCP SSE response id from '{server_name}' did not match the request id."
        ))),
        // Valid server requests and notifications are not the response to this
        // call. They remain bounded by the common JSON validator and may be
        // skipped while waiting for the matching response event.
        JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) => Ok(None),
    }
}

fn response_is_sse(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains(SSE_CONTENT_TYPE))
}

fn observed_certificate_binding(
    scheme: &str,
    response: &reqwest::Response,
) -> Result<ObservedCertificate, McpClientError> {
    if scheme == "http" {
        return Ok(ObservedCertificate {
            binding: "no_tls_exact_loopback".to_string(),
            leaf_der: None,
        });
    }
    if scheme != "https" {
        return Err(McpClientError::permission(
            "Remote MCP response used an unsupported transport scheme.".to_string(),
        ));
    }
    let certificate = response
        .extensions()
        .get::<reqwest::tls::TlsInfo>()
        .and_then(reqwest::tls::TlsInfo::peer_certificate)
        .ok_or_else(|| {
            McpClientError::permission(
                "Remote MCP HTTPS response did not expose a peer certificate for binding."
                    .to_string(),
            )
        })?;
    let mut hasher = Sha256::new();
    hasher.update(certificate);
    Ok(ObservedCertificate {
        binding: format!("sha256:{}", hex::encode(hasher.finalize())),
        leaf_der: Some(certificate.to_vec()),
    })
}

fn network_policy_error(error: crate::network_policy::NetworkPolicyError) -> McpClientError {
    McpClientError::permission(format!(
        "Native destination policy blocked the MCP route: {}",
        error.message
    ))
}

fn log_remote_security_event(
    server_name: &str,
    destination: &CanonicalDestination,
    operation: &str,
    response_bytes: usize,
    decision: &str,
) {
    eprintln!(
        "MCP_REMOTE_SECURITY_EVENT server={} operation={} destination_binding={} response_bytes={} decision={}",
        crate::redaction::redacted_log_text(server_name),
        operation,
        destination.binding_fingerprint(),
        response_bytes,
        decision
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_policy::LocalOriginGrant;
    use base64::Engine;
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    use std::net::Ipv4Addr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio_rustls::TlsAcceptor;

    struct TestServer {
        endpoint: String,
        port: u16,
        handle: tokio::task::JoinHandle<()>,
    }

    const TEST_CA_DER_B64: &str = include_str!("testdata/ca_cert.der.b64");
    const TEST_CERT_A_DER_B64: &str = include_str!("testdata/cert_a.der.b64");
    const TEST_KEY_A_DER_B64: &str = include_str!("testdata/key_a.der.b64");
    const TEST_CERT_B_DER_B64: &str = include_str!("testdata/cert_b.der.b64");
    const TEST_KEY_B_DER_B64: &str = include_str!("testdata/key_b.der.b64");

    fn decode_test_der(encoded: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .expect("embedded TLS test fixture is valid base64")
    }

    fn test_tls_acceptor(cert_b64: &str, key_b64: &str) -> TlsAcceptor {
        let certificate = CertificateDer::from(decode_test_der(cert_b64));
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(decode_test_der(key_b64)));
        let mut config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key)
            .expect("embedded TLS certificate and key match");
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        TlsAcceptor::from(Arc::new(config))
    }

    async fn read_complete_http_request<S>(stream: &mut S) -> Vec<u8>
    where
        S: tokio::io::AsyncRead + Unpin,
    {
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let count = stream.read(&mut chunk).await.unwrap_or(0);
            if count == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..count]);
            if request.len() > REMOTE_REQUEST_BYTE_LIMIT + 16 * 1024 {
                break;
            }
            let Some(header_end) = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| position + 4)
            else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if request.len() >= header_end.saturating_add(content_length) {
                break;
            }
        }
        request
    }

    async fn spawn_server(response: Vec<u8>, response_delay: Duration) -> TestServer {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 16 * 1024];
            let _ = socket.read(&mut request).await;
            if !response_delay.is_zero() {
                sleep(response_delay).await;
            }
            let _ = socket.write_all(&response).await;
            let _ = socket.shutdown().await;
        });
        TestServer {
            endpoint: format!("http://127.0.0.1:{port}/mcp"),
            port,
            handle,
        }
    }

    fn local_transport(server: &TestServer, sse: bool) -> McpTransportConfig {
        let grant = Some(LocalOriginGrant {
            exact_loopback_port: server.port,
        });
        if sse {
            McpTransportConfig::Sse {
                url: server.endpoint.clone(),
                local_origin_grant: grant,
            }
        } else {
            McpTransportConfig::Http {
                url: server.endpoint.clone(),
                local_origin_grant: grant,
            }
        }
    }

    async fn test_request(
        _server: &TestServer,
        transport: &McpTransportConfig,
        cancellation: Arc<AtomicBool>,
    ) -> Result<JsonRpcResponse, McpClientError> {
        let client = build_test_remote_transport_client(transport).await?;
        send_remote_request(
            "real_loopback_test",
            transport,
            &client,
            JsonRpcRequest::new(
                "tools/list",
                serde_json::json!({}),
                serde_json::json!("test-id"),
            ),
            &cancellation,
        )
        .await
    }

    #[tokio::test]
    async fn real_socket_json_and_sse_responses_are_peer_checked_and_bounded() {
        for (content_type, body, sse) in [
            (
                "application/json",
                r#"{"jsonrpc":"2.0","result":{"ok":true},"id":"test-id"}"#,
                false,
            ),
            (
                "text/event-stream",
                "data: {\"jsonrpc\":\"2.0\",\"result\":{\"ok\":true},\"id\":\"test-id\"}\n\n",
                true,
            ),
        ] {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .into_bytes();
            let server = spawn_server(response, Duration::ZERO).await;
            let transport = local_transport(&server, sse);
            let result = test_request(&server, &transport, Arc::new(AtomicBool::new(false)))
                .await
                .unwrap();
            assert_eq!(result.result, Some(serde_json::json!({"ok": true})));
            server.handle.await.unwrap();
        }
    }

    #[tokio::test]
    async fn delayed_headers_hit_the_first_byte_deadline() {
        let body = r#"{"jsonrpc":"2.0","result":{},"id":"test-id"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes();
        let server = spawn_server(
            response,
            REMOTE_FIRST_BYTE_TIMEOUT + Duration::from_millis(150),
        )
        .await;
        let error = test_request(
            &server,
            &local_transport(&server, false),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .expect_err("delayed headers must time out");
        assert!(error.message.contains("first response byte"));
        server.handle.await.unwrap();
    }

    #[tokio::test]
    async fn content_length_and_chunked_stream_caps_apply_to_every_mime_type() {
        let oversized_header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
            REMOTE_RESPONSE_BYTE_LIMIT + 1
        )
        .into_bytes();
        let server = spawn_server(oversized_header, Duration::ZERO).await;
        let error = test_request(
            &server,
            &local_transport(&server, false),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .expect_err("oversized Content-Length must fail before buffering");
        assert!(error.message.contains("before buffering"));
        server.handle.await.unwrap();

        let chunk = "a".repeat(64 * 1024);
        let mut response = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for _ in 0..17 {
            response.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
            response.extend_from_slice(chunk.as_bytes());
            response.extend_from_slice(b"\r\n");
        }
        response.extend_from_slice(b"0\r\n\r\n");
        let server = spawn_server(response, Duration::ZERO).await;
        let error = test_request(
            &server,
            &local_transport(&server, false),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .expect_err("chunked body must be stream-counted");
        assert!(error.message.contains("streaming byte limit"));
        server.handle.await.unwrap();
    }

    #[tokio::test]
    async fn sse_event_limit_is_independent_of_total_response_limit() {
        let data = "x".repeat(REMOTE_SSE_EVENT_BYTE_LIMIT + 1);
        let body = format!("data: {data}\n\n");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes();
        let server = spawn_server(response, Duration::ZERO).await;
        let error = test_request(
            &server,
            &local_transport(&server, true),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .expect_err("oversized SSE event must fail");
        assert!(error.message.contains("event size limit"));
        server.handle.await.unwrap();
    }

    #[test]
    fn malformed_or_mismatched_sse_responses_fail_closed() {
        let request_id = serde_json::json!(7);
        let valid = r#"data: {"jsonrpc":"2.0","id":7,"result":{}}"#;
        let malformed = format!("data: not-json\n\n{valid}\n\n");
        let malformed_error = parse_sse_text_bounded("sse_protocol_test", &malformed, &request_id)
            .expect_err("a malformed prelude must terminate the response");
        assert!(malformed_error.message.contains("valid bounded JSON-RPC"));

        let mismatched =
            format!("data: {{\"jsonrpc\":\"2.0\",\"id\":8,\"result\":{{}}}}\n\n{valid}\n\n");
        let mismatch_error = parse_sse_text_bounded("sse_protocol_test", &mismatched, &request_id)
            .expect_err("a mismatched response id must terminate the response");
        assert!(mismatch_error.message.contains("did not match"));

        let notification_then_response = format!(
            "data: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{{}}}}\n\n{valid}\n\n"
        );
        assert!(matches!(
            parse_sse_text_bounded(
                "sse_protocol_test",
                &notification_then_response,
                &request_id
            ),
            Ok(JsonRpcMessage::Response(response)) if response.id == request_id
        ));
    }

    #[tokio::test]
    async fn redirect_pivot_into_wildcard_is_rejected_before_following() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let response = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://0.0.0.0:{port}/metadata\r\nContent-Length: 0\r\n\r\n"
        )
        .into_bytes();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = socket.read(&mut request).await;
            let _ = socket.write_all(&response).await;
        });
        let server = TestServer {
            endpoint: format!("http://127.0.0.1:{port}/mcp"),
            port,
            handle,
        };
        let error = test_request(
            &server,
            &local_transport(&server, false),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .expect_err("redirect address-class pivot must fail");
        assert!(error.message.contains("destination policy") || error.message.contains("blocked"));
        server.handle.await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_stalled_body() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = socket.read(&mut request).await;
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await;
            sleep(Duration::from_secs(2)).await;
        });
        let server = TestServer {
            endpoint: format!("http://127.0.0.1:{port}/mcp"),
            port,
            handle,
        };
        let transport = local_transport(&server, false);
        let cancellation = Arc::new(AtomicBool::new(false));
        let cancel = cancellation.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(100)).await;
            cancel.store(true, Ordering::Release);
        });
        let error = test_request(&server, &transport, cancellation)
            .await
            .expect_err("cancel signal must terminate body consumption");
        assert_eq!(error.code, "mcp_cancelled");
        server.handle.abort();
    }

    #[tokio::test]
    async fn certificate_rotation_is_rejected_before_the_new_peer_receives_http_bytes() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor_a = test_tls_acceptor(TEST_CERT_A_DER_B64, TEST_KEY_A_DER_B64);
        let acceptor_b = test_tls_acceptor(TEST_CERT_B_DER_B64, TEST_KEY_B_DER_B64);
        let (rotation_observation_tx, rotation_observation_rx) = oneshot::channel();
        let (post_revocation_tx, post_revocation_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket_a, _) = listener.accept().await.unwrap();
            let mut tls_a = acceptor_a.accept(socket_a).await.unwrap();
            let request_a = timeout(
                Duration::from_secs(2),
                read_complete_http_request(&mut tls_a),
            )
            .await
            .expect("certificate A receives the bootstrap initialize request");
            assert!(String::from_utf8_lossy(&request_a).contains("initialize"));
            let body_a = r#"{"jsonrpc":"2.0","result":{},"id":"init-id"}"#;
            let response_a = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_a}",
                body_a.len()
            );
            tls_a.write_all(response_a.as_bytes()).await.unwrap();
            tls_a.shutdown().await.unwrap();

            let (socket_b, _) = listener.accept().await.unwrap();
            let application_bytes = match timeout(
                Duration::from_secs(2),
                acceptor_b.accept(socket_b),
            )
            .await
            {
                Ok(Ok(mut tls_b)) => {
                    let request_b = timeout(
                        Duration::from_millis(500),
                        read_complete_http_request(&mut tls_b),
                    )
                    .await
                    .unwrap_or_default();
                    let body_b = r#"{"jsonrpc":"2.0","result":{},"id":"call-id"}"#;
                    let response_b = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_b}",
                        body_b.len()
                    );
                    let _ = tls_b.write_all(response_b.as_bytes()).await;
                    request_b.len()
                }
                Ok(Err(_)) | Err(_) => 0,
            };
            let _ = rotation_observation_tx.send(application_bytes);

            let accepted_after_revocation = timeout(Duration::from_millis(500), listener.accept())
                .await
                .is_ok();
            let _ = post_revocation_tx.send(accepted_after_revocation);
        });

        let endpoint = format!("https://127.0.0.1:{port}/mcp");
        let transport = McpTransportConfig::Http {
            url: endpoint,
            local_origin_grant: Some(LocalOriginGrant {
                exact_loopback_port: port,
            }),
        };
        let remote = build_test_remote_transport_client_with_roots(
            &transport,
            vec![CertificateDer::from(decode_test_der(TEST_CA_DER_B64))],
        )
        .await
        .unwrap();
        let cancellation = Arc::new(AtomicBool::new(false));

        let premature = send_remote_request(
            "tls_rotation_test",
            &transport,
            &remote,
            JsonRpcRequest::new(
                "tools/call",
                serde_json::json!({"arguments": {"query": "premature-canary"}}),
                "premature-id".into(),
            ),
            &cancellation,
        )
        .await
        .expect_err("only initialize may use the unpinned bootstrap client");
        assert_eq!(premature.code, "mcp_permission_required");

        send_remote_request(
            "tls_rotation_test",
            &transport,
            &remote,
            JsonRpcRequest::new("initialize", serde_json::json!({}), "init-id".into()),
            &cancellation,
        )
        .await
        .expect("certificate A establishes the post-bootstrap pin");
        let expected_a = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(decode_test_der(TEST_CERT_A_DER_B64)))
        );
        assert_eq!(remote.certificate_binding().unwrap(), expected_a);

        let rotation = send_remote_request(
            "tls_rotation_test",
            &transport,
            &remote,
            JsonRpcRequest::new(
                "tools/call",
                serde_json::json!({"arguments": {"query": "argument-canary"}}),
                "call-id".into(),
            ),
            &cancellation,
        )
        .await
        .expect_err("certificate B must fail during the TLS handshake");
        assert_eq!(rotation.code, "mcp_permission_required");
        assert_eq!(
            rotation_observation_rx.await.unwrap(),
            0,
            "certificate B must receive no application-layer request bytes"
        );
        assert!(remote.certificate_binding().is_err());

        let reused = send_remote_request(
            "tls_rotation_test",
            &transport,
            &remote,
            JsonRpcRequest::new(
                "tools/call",
                serde_json::json!({"arguments": {"query": "second-canary"}}),
                "reused-id".into(),
            ),
            &cancellation,
        )
        .await
        .expect_err("terminal certificate failure revokes all prior transport authority");
        assert_eq!(reused.code, "mcp_permission_required");
        assert!(!post_revocation_rx.await.unwrap());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_preempts_authority_dns_revalidation() {
        let port = 48191;
        let destination = resolve_destination(
            &format!("http://127.0.0.1:{port}/mcp"),
            crate::network_policy::DestinationTransport::RemoteMcpHttp,
            Some(LocalOriginGrant {
                exact_loopback_port: port,
            }),
        )
        .await
        .unwrap();
        let cancellation = Arc::new(AtomicBool::new(true));
        let error = revalidate_remote_destination(&destination, &cancellation)
            .await
            .expect_err("cancelled approval consumption must not wait for DNS revalidation");
        assert_eq!(error.code, "mcp_cancelled");
    }
}
