use crate::{
    foundation::clock::unix_time_ms_i64 as unix_time_ms,
    network_policy::{
        resolve_destination, revalidate_destination, validate_connected_peer, CanonicalDestination,
        DestinationTransport, ResolvedDestinationClass,
    },
    redaction::redact_network_error,
    shield_gate::{request_user_approval, ShieldApprovalManager, ShieldApprovalRequest},
};
use rand_core::{OsRng, RngCore};
use reqwest::{redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, UdpSocket},
    time::{Duration, Instant},
};

use super::ToolOutput;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_ENDPOINT_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkDiagnosticRequest {
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub check_local_route: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkDiagnosticState {
    NotChecked,
    LocalNetworkAvailable,
    ApprovedEndpointReachable,
    InternetNotEstablished,
    Failed,
}

impl NetworkDiagnosticState {
    pub fn as_str(&self) -> &'static str {
        diagnostic_state_label(self)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkDiagnosticCheck {
    pub check: String,
    pub outcome: String,
    pub duration_ms: u128,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkDiagnosticReport {
    pub state: NetworkDiagnosticState,
    pub destination: Option<String>,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub total_duration_ms: u128,
    pub internet_reachability_verified: bool,
    pub checks: Vec<NetworkDiagnosticCheck>,
    pub failure: Option<String>,
}

impl NetworkDiagnosticReport {
    pub fn not_checked() -> Self {
        let timestamp = unix_time_ms();
        Self {
            state: NetworkDiagnosticState::NotChecked,
            destination: None,
            started_at_ms: timestamp,
            completed_at_ms: timestamp,
            total_duration_ms: 0,
            internet_reachability_verified: false,
            checks: Vec::new(),
            failure: None,
        }
    }
}

pub struct NetworkDiagnosticTools;

impl NetworkDiagnosticTools {
    /// Inspect the kernel's route selection without sending a datagram. This
    /// proves only that a local route and interface are available; it never
    /// translates that observation into an internet-reachability claim.
    pub fn local_report() -> NetworkDiagnosticReport {
        let started_at_ms = unix_time_ms();
        let started = Instant::now();
        let route_started = Instant::now();
        let route = inspect_default_route();
        let (state, outcome, detail, failure) = match route {
            Ok(class) => (
                NetworkDiagnosticState::LocalNetworkAvailable,
                "observed".to_string(),
                format!("kernel_route_available local_address_class={class}"),
                None,
            ),
            Err(error) => (
                NetworkDiagnosticState::InternetNotEstablished,
                "unavailable".to_string(),
                "kernel_route_unavailable".to_string(),
                Some(redact_network_error(&error)),
            ),
        };
        NetworkDiagnosticReport {
            state,
            destination: None,
            started_at_ms,
            completed_at_ms: unix_time_ms(),
            total_duration_ms: started.elapsed().as_millis(),
            internet_reachability_verified: false,
            checks: vec![NetworkDiagnosticCheck {
                check: "local_interface_route".to_string(),
                outcome,
                duration_ms: route_started.elapsed().as_millis(),
                detail,
            }],
            failure,
        }
    }

    pub fn diagnostic(scope: &str) -> ToolOutput {
        let report = Self::local_report();
        let state = diagnostic_state_label(&report.state);
        ToolOutput {
            operation: "network_diagnostic".to_string(),
            message: format!(
                "Network diagnostic for {scope}: state={state}; internet_reachability_verified=false; checks_run={}",
                report.checks.len()
            ),
            claims: vec![format!(
                "CLAIM operation=network_diagnostic observed_state={state} internet_reachability_verified=false"
            )],
        }
    }
}

#[tauri::command]
pub async fn run_network_diagnostic(
    request: Option<NetworkDiagnosticRequest>,
    app: tauri::AppHandle,
    approvals: tauri::State<'_, ShieldApprovalManager>,
) -> Result<NetworkDiagnosticReport, String> {
    let check_local_route = request
        .as_ref()
        .and_then(|request| request.check_local_route)
        .unwrap_or(true);
    let endpoint = request
        .and_then(|request| request.endpoint)
        .map(|endpoint| endpoint.trim().to_string())
        .filter(|endpoint| !endpoint.is_empty());
    let Some(endpoint) = endpoint else {
        return Ok(if check_local_route {
            NetworkDiagnosticTools::local_report()
        } else {
            NetworkDiagnosticReport::not_checked()
        });
    };
    validate_requested_endpoint(&endpoint)?;

    let policy_started = Instant::now();
    let destination = resolve_destination(&endpoint, DestinationTransport::RemoteMcpHttp, None)
        .await
        .map_err(|error| error.message)?;
    if destination.destination_class() != ResolvedDestinationClass::Public {
        return Err("Network diagnostics require a public HTTPS destination.".to_string());
    }
    let policy_duration_ms = policy_started.elapsed().as_millis();

    request_user_approval(
        &app,
        approvals.inner(),
        network_diagnostic_approval(&destination),
    )
    .await
    .map_err(|error| error.message)?;

    probe_approved_endpoint(destination, policy_duration_ms).await
}

async fn probe_approved_endpoint(
    destination: CanonicalDestination,
    policy_duration_ms: u128,
) -> Result<NetworkDiagnosticReport, String> {
    let started_at_ms = unix_time_ms();
    let started = Instant::now();
    let mut checks = Vec::new();

    checks.push(NetworkDiagnosticCheck {
        check: "destination_policy_and_dns".to_string(),
        outcome: "allowed".to_string(),
        duration_ms: policy_duration_ms,
        detail: format!(
            "public_https_destination address_count={} binding={}",
            destination.resolved_addresses().len(),
            destination.binding_fingerprint()
        ),
    });

    let revalidation_started = Instant::now();
    let destination = revalidate_destination(&destination)
        .await
        .map_err(|error| error.message)?;
    checks.push(NetworkDiagnosticCheck {
        check: "preconnect_destination_revalidation".to_string(),
        outcome: "matched".to_string(),
        duration_ms: revalidation_started.elapsed().as_millis(),
        detail: format!("binding={}", destination.binding_fingerprint()),
    });

    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .https_only(true)
        .redirect(Policy::none())
        .tls_info(true)
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT);
    builder =
        builder.resolve_to_addrs(destination.host(), &destination.resolved_socket_addresses());
    let client = builder
        .build()
        .map_err(|error| redact_network_error(&error.to_string()))?;

    let connect_started = Instant::now();
    let response = client
        .head(destination.url().clone())
        .send()
        .await
        .map_err(|error| redact_network_error(&error.to_string()))?;
    validate_connected_peer(&destination, response.remote_addr()).map_err(|error| error.message)?;
    if response.status().is_redirection() {
        checks.push(NetworkDiagnosticCheck {
            check: "approved_endpoint_connect".to_string(),
            outcome: "redirect_rejected".to_string(),
            duration_ms: connect_started.elapsed().as_millis(),
            detail: format!("http_status={}", response.status().as_u16()),
        });
        return Ok(NetworkDiagnosticReport {
            state: NetworkDiagnosticState::Failed,
            destination: Some(destination.canonical_url().to_string()),
            started_at_ms,
            completed_at_ms: unix_time_ms(),
            total_duration_ms: started.elapsed().as_millis(),
            internet_reachability_verified: false,
            checks,
            failure: Some("approved_endpoint_redirect_rejected".to_string()),
        });
    }
    checks.push(NetworkDiagnosticCheck {
        check: "approved_endpoint_connect".to_string(),
        outcome: "reachable".to_string(),
        duration_ms: connect_started.elapsed().as_millis(),
        detail: format!(
            "tls_verified=true http_status={}",
            response.status().as_u16()
        ),
    });

    eprintln!(
        "NETWORK_DIAGNOSTIC destination_binding={} duration_ms={} decision=reachable",
        destination.binding_fingerprint(),
        started.elapsed().as_millis()
    );
    Ok(NetworkDiagnosticReport {
        state: NetworkDiagnosticState::ApprovedEndpointReachable,
        destination: Some(destination.canonical_url().to_string()),
        started_at_ms,
        completed_at_ms: unix_time_ms(),
        total_duration_ms: started.elapsed().as_millis(),
        internet_reachability_verified: true,
        checks,
        failure: None,
    })
}

fn validate_requested_endpoint(endpoint: &str) -> Result<(), String> {
    if endpoint.len() > MAX_ENDPOINT_BYTES {
        return Err("Network diagnostic endpoint exceeds the input limit.".to_string());
    }
    let url = Url::parse(endpoint).map_err(|_| "Network diagnostic endpoint is invalid.")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
        || url.fragment().is_some()
    {
        return Err(
            "Network diagnostics require an exact public HTTPS URL without credentials or fragments."
                .to_string(),
        );
    }
    Ok(())
}

fn network_diagnostic_approval(destination: &CanonicalDestination) -> ShieldApprovalRequest {
    let mut token = [0_u8; 24];
    OsRng.fill_bytes(&mut token);
    ShieldApprovalRequest {
        approval_token: hex::encode(token),
        session_id: None,
        turn_id: None,
        generation_token: None,
        action_type: "network_diagnostic".to_string(),
        action_label: "Connect to approved diagnostic endpoint".to_string(),
        target_path: Some(destination.canonical_url().to_string()),
        principal: Some("network_diagnostic".to_string()),
        risk_tier: "high".to_string(),
        reason: "Verify reachability only after explicit approval of this exact HTTPS destination."
            .to_string(),
        estimated_token_costs: None,
        requested_at_ms: unix_time_ms().max(0) as u64,
        preview: destination.canonical_url().to_string(),
        semantic_summary: "One bounded HTTPS HEAD request to the displayed destination."
            .to_string(),
        semantic_detail: format!(
            "DNS answers are policy-checked, the connection is pinned to binding {}, redirects are rejected, and no response body is read.",
            destination.binding_fingerprint()
        ),
        approval_tier: "explicit_once".to_string(),
        approval_mode: "network_diagnostic_exact_destination".to_string(),
        diff_preview: None,
        scope_trust_available: false,
        scope_trust_prefix: None,
        scope_trust_duration_ms: 0,
        project_id: None,
        task_run_id: None,
        action_class: "network_diagnostic".to_string(),
        argument_class: crate::approval_scopes::argument_class("network_diagnostic", destination.canonical_url()),
        canonical_resource: Some(destination.canonical_url().to_string()),
        mandatory_reconfirm: false,
        approval_scope_kinds: vec!["once".to_string()],
    }
}

fn inspect_default_route() -> Result<&'static str, String> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|error| error.to_string())?;
    // UDP connect performs local route selection but sends no datagram.
    socket
        .connect("192.0.2.1:9")
        .map_err(|error| error.to_string())?;
    let address = socket.local_addr().map_err(|error| error.to_string())?.ip();
    if address.is_unspecified() {
        return Err("route_selected_unspecified_address".to_string());
    }
    Ok(match address {
        IpAddr::V4(address) if address.is_loopback() => "loopback",
        IpAddr::V4(address) if address.is_private() => "private_ipv4",
        IpAddr::V4(_) => "public_ipv4",
        IpAddr::V6(address) if address.is_loopback() => "loopback",
        IpAddr::V6(address) if address.is_unique_local() => "unique_local_ipv6",
        IpAddr::V6(_) => "global_ipv6",
    })
}

fn diagnostic_state_label(state: &NetworkDiagnosticState) -> &'static str {
    match state {
        NetworkDiagnosticState::NotChecked => "not_checked",
        NetworkDiagnosticState::LocalNetworkAvailable => "local_network_available",
        NetworkDiagnosticState::ApprovedEndpointReachable => "approved_endpoint_reachable",
        NetworkDiagnosticState::InternetNotEstablished => "internet_not_established",
        NetworkDiagnosticState::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_probe_never_claims_internet_reachability() {
        let report = NetworkDiagnosticTools::local_report();
        assert!(!report.internet_reachability_verified);
        assert!(report.destination.is_none());
        assert_eq!(report.checks.len(), 1);
        assert!(matches!(
            report.state,
            NetworkDiagnosticState::LocalNetworkAvailable
                | NetworkDiagnosticState::InternetNotEstablished
        ));
    }

    #[test]
    fn endpoint_validation_rejects_ambiguous_or_unsafe_destinations() {
        for endpoint in [
            "http://example.com",
            "https://user@example.com",
            "https://example.com/#fragment",
            "not a url",
        ] {
            assert!(validate_requested_endpoint(endpoint).is_err(), "{endpoint}");
        }
        assert!(validate_requested_endpoint("https://example.com/health").is_ok());
    }

    #[test]
    fn audit_output_reports_observation_instead_of_completion() {
        let output = NetworkDiagnosticTools::diagnostic("runtime");
        assert!(output.claims[0].contains("observed_state="));
        assert!(output.claims[0].contains("internet_reachability_verified=false"));
        assert!(!output.claims[0].contains("status=completed"));
    }
}
