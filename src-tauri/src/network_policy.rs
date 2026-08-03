use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

#[cfg(not(test))]
const DNS_DEADLINE: Duration = Duration::from_secs(5);
#[cfg(test)]
const DNS_DEADLINE: Duration = Duration::from_millis(250);
const MAX_DESTINATION_URL_BYTES: usize = 8 * 1024;
const MAX_DESTINATION_PATH_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationTransport {
    RemoteMcpHttp,
    RemoteMcpSse,
    NativeBrowser,
}

impl DestinationTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RemoteMcpHttp => "remote_mcp_http",
            Self::RemoteMcpSse => "remote_mcp_sse",
            Self::NativeBrowser => "native_browser",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOriginGrant {
    /// A local MCP grant is deliberately narrower than a hostname grant. It
    /// authorizes one numeric loopback listener and never a wildcard address.
    pub exact_loopback_port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedDestinationClass {
    Public,
    ExactLoopback,
}

impl ResolvedDestinationClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::ExactLoopback => "exact_loopback",
        }
    }
}

/// Canonical, resolved authority used by every native remote-MCP and browser
/// boundary. The URL and complete DNS answer set are immutable after approval.
#[derive(Debug, Clone)]
pub struct CanonicalDestination {
    url: Url,
    canonical_url: String,
    canonical_origin: String,
    host: String,
    port: u16,
    transport: DestinationTransport,
    resolved_addresses: Vec<IpAddr>,
    destination_class: ResolvedDestinationClass,
    local_origin_grant: Option<LocalOriginGrant>,
    binding_fingerprint: String,
}

impl CanonicalDestination {
    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn canonical_url(&self) -> &str {
        &self.canonical_url
    }

    pub fn canonical_origin(&self) -> &str {
        &self.canonical_origin
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn transport(&self) -> DestinationTransport {
        self.transport
    }

    pub fn resolved_addresses(&self) -> &[IpAddr] {
        &self.resolved_addresses
    }

    pub fn resolved_socket_addresses(&self) -> Vec<SocketAddr> {
        self.resolved_addresses
            .iter()
            .copied()
            .map(|address| SocketAddr::new(address, self.port))
            .collect()
    }

    pub fn destination_class(&self) -> ResolvedDestinationClass {
        self.destination_class
    }

    pub fn binding_fingerprint(&self) -> &str {
        &self.binding_fingerprint
    }

    pub fn local_origin_grant(&self) -> Option<&LocalOriginGrant> {
        self.local_origin_grant.as_ref()
    }

    pub fn redacted_summary(&self) -> String {
        format!(
            "origin={} transport={} destination_class={} address_count={} binding={}",
            self.canonical_origin,
            self.transport.as_str(),
            self.destination_class.as_str(),
            self.resolved_addresses.len(),
            self.binding_fingerprint
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkPolicyError {
    pub code: &'static str,
    pub message: String,
}

impl NetworkPolicyError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "destination_invalid",
            message: message.into(),
        }
    }

    fn blocked(message: impl Into<String>) -> Self {
        Self {
            code: "destination_blocked",
            message: message.into(),
        }
    }

    fn changed(message: impl Into<String>) -> Self {
        Self {
            code: "destination_policy_changed",
            message: message.into(),
        }
    }
}

impl fmt::Display for NetworkPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for NetworkPolicyError {}

pub async fn resolve_destination(
    endpoint: &str,
    transport: DestinationTransport,
    local_origin_grant: Option<LocalOriginGrant>,
) -> Result<CanonicalDestination, NetworkPolicyError> {
    let canonical = canonicalize_destination(endpoint, transport, local_origin_grant)?;
    let addresses = tokio::time::timeout(
        DNS_DEADLINE,
        tokio::net::lookup_host((canonical.host.as_str(), canonical.port)),
    )
    .await
    .map_err(|_| {
        NetworkPolicyError::blocked(format!(
            "Destination DNS resolution exceeded the {} second deadline.",
            DNS_DEADLINE.as_secs()
        ))
    })?
    .map_err(|error| {
        NetworkPolicyError::blocked(format!("Destination DNS resolution failed: {error}"))
    })?
    .map(|address| address.ip())
    .collect::<Vec<_>>();

    finish_resolution(canonical, addresses.into_iter())
}

pub fn resolve_destination_blocking(
    endpoint: &str,
    transport: DestinationTransport,
    local_origin_grant: Option<LocalOriginGrant>,
) -> Result<CanonicalDestination, NetworkPolicyError> {
    let canonical = canonicalize_destination(endpoint, transport, local_origin_grant)?;
    let host = canonical.host.clone();
    let port = canonical.port;
    let addresses = resolve_blocking_with_deadline(move || {
        (host.as_str(), port)
            .to_socket_addrs()
            .map(|addresses| addresses.map(|address| address.ip()).collect::<Vec<_>>())
            .map_err(|error| error.to_string())
    })?;
    finish_resolution(canonical, addresses.into_iter())
}

fn resolve_blocking_with_deadline<F>(resolver: F) -> Result<Vec<IpAddr>, NetworkPolicyError>
where
    F: FnOnce() -> Result<Vec<IpAddr>, String> + Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(resolver());
    });
    receiver
        .recv_timeout(DNS_DEADLINE)
        .map_err(|error| match error {
            std::sync::mpsc::RecvTimeoutError::Timeout => NetworkPolicyError::blocked(format!(
                "Destination DNS resolution exceeded the {} millisecond deadline.",
                DNS_DEADLINE.as_millis()
            )),
            std::sync::mpsc::RecvTimeoutError::Disconnected => {
                NetworkPolicyError::blocked("Destination DNS resolver terminated unexpectedly.")
            }
        })?
        .map_err(|error| {
            NetworkPolicyError::blocked(format!("Destination DNS resolution failed: {error}"))
        })
}

pub async fn revalidate_destination(
    approved: &CanonicalDestination,
) -> Result<CanonicalDestination, NetworkPolicyError> {
    let current = resolve_destination(
        approved.canonical_url(),
        approved.transport,
        approved.local_origin_grant.clone(),
    )
    .await?;
    ensure_same_policy_binding(approved, &current)?;
    Ok(current)
}

pub async fn validate_redirect_destination(
    approved: &CanonicalDestination,
    redirect_url: &str,
) -> Result<CanonicalDestination, NetworkPolicyError> {
    let redirected = resolve_destination(
        redirect_url,
        approved.transport,
        approved.local_origin_grant.clone(),
    )
    .await?;
    if redirected.canonical_origin != approved.canonical_origin {
        return Err(NetworkPolicyError::changed(
            "Redirect changed the approved canonical origin.",
        ));
    }
    ensure_same_policy_binding(approved, &redirected)?;
    Ok(redirected)
}

pub fn validate_browser_navigation_blocking(
    approved: &CanonicalDestination,
    navigation_url: &str,
) -> Result<CanonicalDestination, NetworkPolicyError> {
    if approved.transport != DestinationTransport::NativeBrowser {
        return Err(NetworkPolicyError::changed(
            "A non-browser destination grant cannot authorize browser navigation.",
        ));
    }
    let current =
        resolve_destination_blocking(navigation_url, DestinationTransport::NativeBrowser, None)?;
    if current.canonical_origin != approved.canonical_origin {
        return Err(NetworkPolicyError::changed(
            "Browser navigation changed the user-approved canonical origin.",
        ));
    }
    ensure_same_policy_binding(approved, &current)?;
    Ok(current)
}

pub fn validate_connected_peer(
    approved: &CanonicalDestination,
    peer: Option<SocketAddr>,
) -> Result<(), NetworkPolicyError> {
    let peer = peer.ok_or_else(|| {
        NetworkPolicyError::blocked(
            "The connected socket did not expose a peer address for native policy verification.",
        )
    })?;
    if peer.port() != approved.port || !approved.resolved_addresses.contains(&peer.ip()) {
        return Err(NetworkPolicyError::changed(format!(
            "The connected socket peer did not match the approved destination binding (peer class {}).",
            classify_ip(peer.ip()).label()
        )));
    }
    ensure_address_allowed(
        peer.ip(),
        approved.port,
        approved.local_origin_grant.as_ref(),
    )?;
    Ok(())
}

fn ensure_same_policy_binding(
    approved: &CanonicalDestination,
    current: &CanonicalDestination,
) -> Result<(), NetworkPolicyError> {
    if approved.canonical_origin != current.canonical_origin
        || approved.transport != current.transport
        || approved.destination_class != current.destination_class
        || approved.resolved_addresses != current.resolved_addresses
        || approved.binding_fingerprint != current.binding_fingerprint
    {
        return Err(NetworkPolicyError::changed(
            "Destination origin, transport, address class, or DNS answer set changed after approval.",
        ));
    }
    Ok(())
}

struct CanonicalParts {
    url: Url,
    canonical_url: String,
    canonical_origin: String,
    host: String,
    port: u16,
    transport: DestinationTransport,
    local_origin_grant: Option<LocalOriginGrant>,
}

fn canonicalize_destination(
    endpoint: &str,
    transport: DestinationTransport,
    local_origin_grant: Option<LocalOriginGrant>,
) -> Result<CanonicalParts, NetworkPolicyError> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() || endpoint.len() > MAX_DESTINATION_URL_BYTES {
        return Err(NetworkPolicyError::invalid(
            "Destination URL is empty or exceeds the native URL limit.",
        ));
    }
    let raw_authority = raw_authority(endpoint)?;
    if !raw_authority.is_ascii() || raw_authority.contains('%') || raw_authority.contains('\\') {
        return Err(NetworkPolicyError::invalid(
            "Destination authority contains an ambiguous encoding.",
        ));
    }

    let mut url = Url::parse(endpoint).map_err(|error| {
        NetworkPolicyError::invalid(format!("Destination URL is malformed: {error}"))
    })?;
    if !url.username().is_empty() || url.password().is_some() || raw_authority.contains('@') {
        return Err(NetworkPolicyError::invalid(
            "Destination URLs may not contain embedded credentials.",
        ));
    }
    if url.fragment().is_some() {
        return Err(NetworkPolicyError::invalid(
            "Destination URLs may not contain fragments at an approval boundary.",
        ));
    }
    if url.path().len() > MAX_DESTINATION_PATH_BYTES {
        return Err(NetworkPolicyError::invalid(
            "Destination path exceeds the native path limit.",
        ));
    }

    let scheme = url.scheme();
    if !matches!(scheme, "http" | "https") {
        return Err(NetworkPolicyError::invalid(
            "Only canonical HTTP and HTTPS destinations are supported.",
        ));
    }
    if transport == DestinationTransport::NativeBrowser && scheme != "https" {
        return Err(NetworkPolicyError::blocked(
            "Native browser navigation requires HTTPS.",
        ));
    }
    if scheme == "http" && local_origin_grant.is_none() {
        return Err(NetworkPolicyError::blocked(
            "Plain HTTP requires an explicit exact-loopback local-origin grant.",
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| NetworkPolicyError::invalid("Destination URL must contain a host."))?
        .to_ascii_lowercase();
    validate_canonical_host(raw_host(raw_authority)?, &host)?;
    if is_metadata_hostname(&host) || host == "localhost" || host.ends_with(".localhost") {
        return Err(NetworkPolicyError::blocked(
            "Cloud metadata and localhost hostnames are blocked by native destination policy.",
        ));
    }

    let port = url.port_or_known_default().ok_or_else(|| {
        NetworkPolicyError::invalid("Destination URL does not have a canonical port.")
    })?;
    if port == 0 {
        return Err(NetworkPolicyError::invalid(
            "Destination port zero is not routable.",
        ));
    }
    if let Some(grant) = local_origin_grant.as_ref() {
        if grant.exact_loopback_port == 0 || grant.exact_loopback_port != port {
            return Err(NetworkPolicyError::blocked(
                "Local-origin grant does not match the destination's exact loopback port.",
            ));
        }
    }

    if (scheme == "https" && url.port() == Some(443))
        || (scheme == "http" && url.port() == Some(80))
    {
        url.set_port(None).map_err(|_| {
            NetworkPolicyError::invalid("Destination default port could not be canonicalized.")
        })?;
    }
    let canonical_url = url.as_str().to_string();
    let canonical_origin = url.origin().ascii_serialization();

    Ok(CanonicalParts {
        url,
        canonical_url,
        canonical_origin,
        host,
        port,
        transport,
        local_origin_grant,
    })
}

fn finish_resolution(
    canonical: CanonicalParts,
    addresses: impl Iterator<Item = IpAddr>,
) -> Result<CanonicalDestination, NetworkPolicyError> {
    let addresses = addresses
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(NetworkPolicyError::blocked(
            "Destination DNS resolution returned no address records.",
        ));
    }

    let mut destination_class = None;
    for address in &addresses {
        let class = ensure_address_allowed(
            *address,
            canonical.port,
            canonical.local_origin_grant.as_ref(),
        )?;
        if destination_class.is_some_and(|existing| existing != class) {
            return Err(NetworkPolicyError::blocked(
                "Destination DNS records span multiple address classes.",
            ));
        }
        destination_class = Some(class);
    }
    let destination_class = destination_class.expect("non-empty address list has a class");
    let binding_fingerprint = destination_binding_fingerprint(
        &canonical.canonical_origin,
        canonical.transport,
        destination_class,
        &addresses,
    );

    Ok(CanonicalDestination {
        url: canonical.url,
        canonical_url: canonical.canonical_url,
        canonical_origin: canonical.canonical_origin,
        host: canonical.host,
        port: canonical.port,
        transport: canonical.transport,
        resolved_addresses: addresses,
        destination_class,
        local_origin_grant: canonical.local_origin_grant,
        binding_fingerprint,
    })
}

fn destination_binding_fingerprint(
    origin: &str,
    transport: DestinationTransport,
    class: ResolvedDestinationClass,
    addresses: &[IpAddr],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(origin.as_bytes());
    hasher.update([0]);
    hasher.update(transport.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(class.as_str().as_bytes());
    for address in addresses {
        hasher.update([0]);
        hasher.update(address.to_string().as_bytes());
    }
    hex::encode(hasher.finalize())
}

fn raw_authority(endpoint: &str) -> Result<&str, NetworkPolicyError> {
    let (_, remainder) = endpoint.split_once("://").ok_or_else(|| {
        NetworkPolicyError::invalid("Destination URL must include an explicit scheme.")
    })?;
    let end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    let authority = &remainder[..end];
    if authority.is_empty() {
        return Err(NetworkPolicyError::invalid(
            "Destination URL must include an authority.",
        ));
    }
    Ok(authority)
}

fn raw_host(authority: &str) -> Result<&str, NetworkPolicyError> {
    if authority.starts_with('[') {
        let end = authority.find(']').ok_or_else(|| {
            NetworkPolicyError::invalid("IPv6 destination authority is missing a closing bracket.")
        })?;
        return Ok(&authority[1..end]);
    }
    Ok(authority.split(':').next().unwrap_or_default())
}

fn validate_canonical_host(raw: &str, canonical: &str) -> Result<(), NetworkPolicyError> {
    if raw.is_empty()
        || raw.ends_with('.')
        || raw.starts_with('.')
        || raw.contains("..")
        || raw.contains('_')
    {
        return Err(NetworkPolicyError::invalid(
            "Destination hostname is empty, ambiguous, or non-canonical.",
        ));
    }
    if let Ok(ip) = canonical.parse::<IpAddr>() {
        if raw.to_ascii_lowercase() != ip.to_string().to_ascii_lowercase() {
            return Err(NetworkPolicyError::invalid(
                "Destination IP address must use its canonical textual form.",
            ));
        }
    }
    Ok(())
}

fn is_metadata_hostname(host: &str) -> bool {
    matches!(
        host,
        "metadata.google.internal"
            | "instance-data.ec2.internal"
            | "metadata.azure.internal"
            | "metadata.oraclecloud.com"
            | "metadata.packet.net"
            | "metadata.tencentyun.com"
    ) || host.ends_with(".metadata.google.internal")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IpSecurityClass {
    Public,
    Loopback,
    Unspecified,
    Private,
    LinkLocal,
    Multicast,
    CarrierGradeNat,
    Reserved,
}

impl IpSecurityClass {
    fn label(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Loopback => "loopback",
            Self::Unspecified => "unspecified",
            Self::Private => "private",
            Self::LinkLocal => "link_local",
            Self::Multicast => "multicast",
            Self::CarrierGradeNat => "carrier_grade_nat",
            Self::Reserved => "reserved_or_metadata",
        }
    }
}

fn ensure_address_allowed(
    address: IpAddr,
    port: u16,
    local_origin_grant: Option<&LocalOriginGrant>,
) -> Result<ResolvedDestinationClass, NetworkPolicyError> {
    let class = classify_ip(address);
    if class == IpSecurityClass::Public {
        if local_origin_grant.is_some() {
            return Err(NetworkPolicyError::blocked(
                "A local-origin grant cannot authorize a public destination.",
            ));
        }
        return Ok(ResolvedDestinationClass::Public);
    }
    if class == IpSecurityClass::Loopback
        && local_origin_grant.is_some_and(|grant| grant.exact_loopback_port == port)
    {
        return Ok(ResolvedDestinationClass::ExactLoopback);
    }
    Err(NetworkPolicyError::blocked(format!(
        "Destination resolved to a blocked {} address class.",
        class.label()
    )))
}

fn classify_ip(address: IpAddr) -> IpSecurityClass {
    match address {
        IpAddr::V4(address) => classify_ipv4(address),
        IpAddr::V6(address) => classify_ipv6(address),
    }
}

fn classify_ipv4(address: Ipv4Addr) -> IpSecurityClass {
    let octets = address.octets();
    if address.is_unspecified() {
        IpSecurityClass::Unspecified
    } else if address.is_loopback() {
        IpSecurityClass::Loopback
    } else if address.is_private() {
        IpSecurityClass::Private
    } else if address.is_link_local() {
        IpSecurityClass::LinkLocal
    } else if address.is_multicast() {
        IpSecurityClass::Multicast
    } else if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        IpSecurityClass::CarrierGradeNat
    } else if octets[0] == 0
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 192 && octets[1] == 88 && octets[2] == 99)
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        || octets[0] >= 240
    {
        IpSecurityClass::Reserved
    } else {
        IpSecurityClass::Public
    }
}

fn classify_ipv6(address: Ipv6Addr) -> IpSecurityClass {
    if address.is_unspecified() {
        return IpSecurityClass::Unspecified;
    }
    if address.is_loopback() {
        return IpSecurityClass::Loopback;
    }
    if address.is_multicast() {
        return IpSecurityClass::Multicast;
    }
    if let Some(mapped) = address.to_ipv4_mapped() {
        return classify_ipv4(mapped);
    }
    let segments = address.segments();
    if segments[0] & 0xfe00 == 0xfc00 {
        IpSecurityClass::Private
    } else if segments[0] & 0xffc0 == 0xfe80 {
        IpSecurityClass::LinkLocal
    } else if (segments[0] == 0x0064
        && segments[1] == 0xff9b
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0
        && segments[5] == 0)
        || (segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2] == 1)
        || (segments[0] == 0x2001 && segments[1] & 0xfe00 == 0)
        || segments[0] == 0x2002
        // IANA currently reserves the surrounding 3f00::/8 allocation space;
        // this includes returned 6bone 3ffe::/16 and documentation 3fff::/20.
        || segments[0] & 0xff00 == 0x3f00
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
    {
        IpSecurityClass::Reserved
    } else if segments[0] & 0xe000 != 0x2000 {
        // Public IPv6 unicast is currently allocated from 2000::/3. Reject
        // everything else by default instead of guessing about future ranges.
        IpSecurityClass::Reserved
    } else {
        IpSecurityClass::Public
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[tokio::test]
    async fn real_loopback_listener_requires_an_exact_local_origin_grant() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let endpoint = format!("http://127.0.0.1:{port}/mcp");

        let blocked = resolve_destination(&endpoint, DestinationTransport::RemoteMcpHttp, None)
            .await
            .expect_err("loopback must be blocked without a grant");
        assert_eq!(blocked.code, "destination_blocked");

        let allowed = resolve_destination(
            &endpoint,
            DestinationTransport::RemoteMcpHttp,
            Some(LocalOriginGrant {
                exact_loopback_port: port,
            }),
        )
        .await
        .expect("the exact live loopback listener is allowed");
        assert_eq!(
            allowed.destination_class(),
            ResolvedDestinationClass::ExactLoopback
        );
        assert_eq!(
            allowed.resolved_addresses(),
            &[IpAddr::V4(Ipv4Addr::LOCALHOST)]
        );
        drop(listener);
    }

    #[test]
    fn wildcard_and_noncanonical_loopback_destinations_are_rejected() {
        let wildcard = resolve_destination_blocking(
            "http://0.0.0.0:8080/mcp",
            DestinationTransport::RemoteMcpHttp,
            Some(LocalOriginGrant {
                exact_loopback_port: 8080,
            }),
        )
        .expect_err("wildcard is not loopback authority");
        assert_eq!(wildcard.code, "destination_blocked");

        let shortened = resolve_destination_blocking(
            "http://127.1:8080/mcp",
            DestinationTransport::RemoteMcpHttp,
            Some(LocalOriginGrant {
                exact_loopback_port: 8080,
            }),
        )
        .expect_err("non-canonical IPv4 is rejected");
        assert_eq!(shortened.code, "destination_invalid");
    }

    #[test]
    fn unsafe_schemes_credentials_metadata_and_browser_http_fail_closed() {
        for endpoint in [
            "file:///etc/passwd",
            "https://user:pass@example.com/",
            "https://metadata.google.internal/computeMetadata/v1/",
            "https://127%2e0%2e0%2e1/",
        ] {
            assert!(
                canonicalize_destination(endpoint, DestinationTransport::NativeBrowser, None)
                    .is_err()
            );
        }
        assert!(canonicalize_destination(
            "http://example.com/",
            DestinationTransport::NativeBrowser,
            None
        )
        .is_err());
    }

    #[test]
    fn private_cgnat_link_local_multicast_and_reserved_classes_are_blocked() {
        for address in [
            "10.0.0.1",
            "100.64.0.1",
            "169.254.169.254",
            "224.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "203.0.113.1",
            "255.255.255.255",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "64:ff9b::c0a8:1",
            "64:ff9b:1::1",
            "2001::1",
            "2002:c0a8:101::1",
            "3f00::1",
            "3ffe::1",
            "3fff::1",
            "3fff:1000::1",
        ] {
            let address = address.parse::<IpAddr>().unwrap();
            let error = ensure_address_allowed(address, 443, None)
                .expect_err("non-public address must fail closed");
            assert_eq!(error.code, "destination_blocked");
        }
    }

    #[test]
    fn encoded_private_and_transition_destinations_fail_before_connection() {
        for endpoint in [
            "https://0x7f000001/",
            "https://0177.0.0.1/",
            "https://[64:ff9b::c0a8:1]/",
            "https://[2002:c0a8:101::1]/",
        ] {
            let error =
                resolve_destination_blocking(endpoint, DestinationTransport::NativeBrowser, None)
                    .expect_err("encoded or transition destination must fail closed");
            assert!(matches!(
                error.code,
                "destination_invalid" | "destination_blocked"
            ));
        }
    }

    #[test]
    fn peer_must_match_the_complete_approved_socket_binding() {
        let approved = resolve_destination_blocking(
            "http://127.0.0.1:48123/mcp",
            DestinationTransport::RemoteMcpHttp,
            Some(LocalOriginGrant {
                exact_loopback_port: 48123,
            }),
        )
        .unwrap();
        validate_connected_peer(
            &approved,
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 48123)),
        )
        .unwrap();
        assert!(validate_connected_peer(
            &approved,
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 48124))
        )
        .is_err());
        assert!(validate_connected_peer(&approved, None).is_err());
    }

    #[test]
    fn blocking_resolver_has_a_hard_deadline() {
        let started = std::time::Instant::now();
        let error = resolve_blocking_with_deadline(|| {
            std::thread::sleep(DNS_DEADLINE + Duration::from_millis(200));
            Ok(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))])
        })
        .expect_err("stalled resolver must time out");
        assert_eq!(error.code, "destination_blocked");
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
