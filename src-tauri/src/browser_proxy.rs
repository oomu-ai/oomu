use crate::network_policy::{
    resolve_destination, revalidate_destination, validate_connected_peer, CanonicalDestination,
    DestinationTransport,
};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BROWSER_PROXY_HEADER_LIMIT: usize = 16 * 1024;
const BROWSER_PROXY_HEADER_TIMEOUT: Duration = Duration::from_secs(5);
const BROWSER_PROXY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const BROWSER_PROXY_TUNNEL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const BROWSER_PROXY_MAX_CONNECTIONS: usize = 32;
const BROWSER_PROXY_MAX_HTTPS_SUBRESOURCE_HOSTS: usize = 8;

pub(crate) struct BrowserProxyHandle {
    proxy_url: tauri::Url,
    shutdown: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl BrowserProxyHandle {
    pub(crate) fn proxy_url(&self) -> tauri::Url {
        self.proxy_url.clone()
    }
}

impl Drop for BrowserProxyHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.task.abort();
    }
}

pub(crate) async fn start_browser_connect_proxy(
    destination: CanonicalDestination,
) -> Result<BrowserProxyHandle, String> {
    start_browser_connect_proxy_for_destinations(vec![destination]).await
}

/// Starts the proxy used only by an invisible, incognito DOM renderer. The primary
/// destination remains the sole top-level navigation grant; these extra destinations
/// authorize TLS CONNECT tunnels for explicitly declared page dependencies only.
pub(crate) async fn start_hidden_browser_connect_proxy(
    destination: CanonicalDestination,
    declared_hosts: &[String],
) -> Result<BrowserProxyHandle, String> {
    let subresource_urls = exact_https_subresource_urls(declared_hosts)?;
    let resolved =
        futures_util::future::join_all(subresource_urls.into_iter().map(|url| async move {
            resolve_destination(&url, DestinationTransport::NativeBrowser, None)
                .await
                .map_err(|error| {
                    format!(
                        "Hidden browser subresource destination could not be pinned: {}",
                        error.message
                    )
                })
        }))
        .await;

    let mut approved = Vec::with_capacity(resolved.len().saturating_add(1));
    let primary_authority = expected_connect_authority(&destination);
    approved.push(destination);
    for resolved_destination in resolved {
        let resolved_destination = resolved_destination?;
        if resolved_destination.port() != 443 {
            return Err(
                "Hidden browser subresource destinations must use canonical HTTPS port 443."
                    .to_string(),
            );
        }
        if expected_connect_authority(&resolved_destination) != primary_authority {
            approved.push(resolved_destination);
        }
    }

    start_browser_connect_proxy_for_destinations(approved).await
}

async fn start_browser_connect_proxy_for_destinations(
    destinations: Vec<CanonicalDestination>,
) -> Result<BrowserProxyHandle, String> {
    if destinations.is_empty() {
        return Err("Browser proxy requires an approved destination.".to_string());
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| format!("Failed to bind the native browser security proxy: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("Failed to inspect the native browser security proxy: {error}"))?;
    let proxy_url = format!("http://{address}")
        .parse::<tauri::Url>()
        .map_err(|_| "Failed to construct the native browser security proxy URL.".to_string())?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let task_shutdown = shutdown.clone();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(BROWSER_PROXY_MAX_CONNECTIONS));
    let destinations = Arc::new(destinations);
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((mut inbound, _)) = accepted else { break };
                    let Ok(permit) = semaphore.clone().try_acquire_owned() else {
                        let _ = inbound.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                        continue;
                    };
                    let connection_destinations = destinations.clone();
                    let connection_shutdown = task_shutdown.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let _ = handle_browser_proxy_connection(
                            &mut inbound,
                            connection_destinations.as_slice(),
                            &connection_shutdown,
                        )
                        .await;
                    });
                }
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    if task_shutdown.load(Ordering::Acquire) {
                        break;
                    }
                }
            }
        }
    });
    Ok(BrowserProxyHandle {
        proxy_url,
        shutdown,
        task,
    })
}

async fn handle_browser_proxy_connection(
    inbound: &mut tokio::net::TcpStream,
    approved_destinations: &[CanonicalDestination],
    shutdown: &Arc<AtomicBool>,
) -> Result<(), String> {
    let (header, buffered_after_header) = read_proxy_header(inbound).await?;
    let request_line = header
        .lines()
        .next()
        .ok_or_else(|| "Browser proxy request was empty.".to_string())?;
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next().unwrap_or_default();
    let authority = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    let approved = if method == "CONNECT"
        && matches!(version, "HTTP/1.1" | "HTTP/1.0")
        && parts.next().is_none()
    {
        let authority = authority.to_ascii_lowercase();
        approved_destinations
            .iter()
            .find(|destination| expected_connect_authority(destination) == authority)
    } else {
        None
    };
    let Some(approved) = approved else {
        write_proxy_rejection(inbound).await;
        return Err("Browser proxy denied an authority outside the approved origin.".to_string());
    };

    let current = revalidate_destination(approved)
        .await
        .map_err(|error| format!("Browser proxy destination changed: {}", error.message))?;
    let mut upstream = connect_pinned_browser_peer(&current).await?;
    inbound
        .write_all(
            b"HTTP/1.1 200 Connection Established\r\nProxy-Agent: OOMU-Native-Boundary\r\n\r\n",
        )
        .await
        .map_err(|error| format!("Browser proxy handshake failed: {error}"))?;
    if !buffered_after_header.is_empty() {
        upstream
            .write_all(&buffered_after_header)
            .await
            .map_err(|error| format!("Browser proxy buffered request failed: {error}"))?;
    }

    tokio::select! {
        copied = tokio::time::timeout(
            BROWSER_PROXY_TUNNEL_TIMEOUT,
            tokio::io::copy_bidirectional(inbound, &mut upstream),
        ) => {
            copied
                .map_err(|_| "Browser proxy tunnel exceeded its total deadline.".to_string())?
                .map_err(|error| format!("Browser proxy tunnel failed: {error}"))?;
        }
        _ = wait_for_proxy_shutdown(shutdown) => {}
    }
    Ok(())
}

async fn read_proxy_header(
    inbound: &mut tokio::net::TcpStream,
) -> Result<(String, Vec<u8>), String> {
    let read = async {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 2048];
        loop {
            let count = inbound
                .read(&mut buffer)
                .await
                .map_err(|error| format!("Browser proxy header read failed: {error}"))?;
            if count == 0 {
                return Err("Browser proxy client closed before CONNECT.".to_string());
            }
            bytes.extend_from_slice(&buffer[..count]);
            if bytes.len() > BROWSER_PROXY_HEADER_LIMIT {
                return Err("Browser proxy header exceeded the native limit.".to_string());
            }
            if let Some(boundary) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let header_end = boundary + 4;
                let header = String::from_utf8(bytes[..boundary].to_vec())
                    .map_err(|_| "Browser proxy header was not valid UTF-8.".to_string())?;
                return Ok((header, bytes[header_end..].to_vec()));
            }
        }
    };
    tokio::time::timeout(BROWSER_PROXY_HEADER_TIMEOUT, read)
        .await
        .map_err(|_| "Browser proxy CONNECT header timed out.".to_string())?
}

async fn connect_pinned_browser_peer(
    destination: &CanonicalDestination,
) -> Result<tokio::net::TcpStream, String> {
    for address in destination.resolved_socket_addresses() {
        let connected = tokio::time::timeout(
            BROWSER_PROXY_CONNECT_TIMEOUT,
            tokio::net::TcpStream::connect(address),
        )
        .await;
        let Ok(Ok(stream)) = connected else {
            continue;
        };
        validate_browser_proxy_peer(destination, stream.peer_addr().ok())?;
        return Ok(stream);
    }
    Err("Browser proxy could not connect to any approved destination address.".to_string())
}

fn validate_browser_proxy_peer(
    destination: &CanonicalDestination,
    peer: Option<SocketAddr>,
) -> Result<(), String> {
    validate_connected_peer(destination, peer)
        .map_err(|error| format!("Browser proxy peer validation failed: {}", error.message))
}

fn expected_connect_authority(destination: &CanonicalDestination) -> String {
    if destination.host().contains(':') {
        format!("[{}]:{}", destination.host(), destination.port()).to_ascii_lowercase()
    } else {
        format!("{}:{}", destination.host(), destination.port()).to_ascii_lowercase()
    }
}

fn exact_https_subresource_urls(declared_hosts: &[String]) -> Result<Vec<String>, String> {
    let mut urls = Vec::new();
    let mut seen = HashSet::new();
    for raw_host in declared_hosts {
        let raw_host = raw_host.trim();
        if raw_host.is_empty() || raw_host == "*" || raw_host.starts_with("*.") {
            continue;
        }
        let candidate = if raw_host.contains("://") {
            raw_host.to_string()
        } else {
            format!("https://{raw_host}/")
        };
        let Ok(mut url) = reqwest::Url::parse(&candidate) else {
            continue;
        };
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port_or_known_default() != Some(443)
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || url.host_str().is_none()
        {
            continue;
        }
        // Normalize an explicit default port so deduplication and CONNECT matching use
        // exactly the same canonical authority as network_policy.
        if url.port() == Some(443) {
            let _ = url.set_port(None);
        }
        let canonical = url.to_string();
        if seen.insert(canonical.clone()) {
            urls.push(canonical);
        }
    }
    if urls.len() > BROWSER_PROXY_MAX_HTTPS_SUBRESOURCE_HOSTS {
        return Err(format!(
            "Hidden browser subresource policy permits at most {BROWSER_PROXY_MAX_HTTPS_SUBRESOURCE_HOSTS} exact HTTPS hosts."
        ));
    }
    Ok(urls)
}

async fn write_proxy_rejection(inbound: &mut tokio::net::TcpStream) {
    let _ = inbound
        .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .await;
}

async fn wait_for_proxy_shutdown(shutdown: &Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network_policy::{resolve_destination, DestinationTransport, LocalOriginGrant};

    async fn loopback_destination(port: u16) -> CanonicalDestination {
        resolve_destination(
            &format!("http://127.0.0.1:{port}/mcp"),
            DestinationTransport::RemoteMcpHttp,
            Some(LocalOriginGrant {
                exact_loopback_port: port,
            }),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn connect_proxy_rejects_authority_pivots_and_tunnels_only_to_the_pinned_peer() {
        let upstream = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        let destination = loopback_destination(upstream_port).await;
        let upstream_task = tokio::spawn(async move {
            let (mut peer, address) = upstream.accept().await.unwrap();
            assert_eq!(
                address.ip(),
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
            );
            let mut request = [0_u8; 4];
            peer.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"PING");
            peer.write_all(b"PONG").await.unwrap();
        });

        let proxy = start_browser_connect_proxy(destination).await.unwrap();
        let proxy_port = proxy.proxy_url.port_or_known_default().unwrap();

        let mut rejected =
            tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, proxy_port))
                .await
                .unwrap();
        rejected
            .write_all(
                format!(
                    "CONNECT 0.0.0.0:{upstream_port} HTTP/1.1\r\nHost: 0.0.0.0:{upstream_port}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut rejection = [0_u8; 256];
        let rejection_bytes = rejected.read(&mut rejection).await.unwrap();
        assert!(String::from_utf8_lossy(&rejection[..rejection_bytes])
            .starts_with("HTTP/1.1 403 Forbidden"));

        let mut approved =
            tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, proxy_port))
                .await
                .unwrap();
        approved
            .write_all(
                format!(
                    "CONNECT 127.0.0.1:{upstream_port} HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut handshake = [0_u8; 256];
        let handshake_bytes = approved.read(&mut handshake).await.unwrap();
        assert!(String::from_utf8_lossy(&handshake[..handshake_bytes])
            .starts_with("HTTP/1.1 200 Connection Established"));

        approved.write_all(b"PING").await.unwrap();
        let mut response = [0_u8; 4];
        approved.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"PONG");
        upstream_task.await.unwrap();
        drop(proxy);
    }

    #[tokio::test]
    async fn connect_proxy_tunnels_to_each_pinned_destination_and_rejects_undeclared_peers() {
        let primary = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let primary_port = primary.local_addr().unwrap().port();
        let subresource = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let subresource_port = subresource.local_addr().unwrap().port();
        let undeclared = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let undeclared_port = undeclared.local_addr().unwrap().port();
        let destinations = vec![
            loopback_destination(primary_port).await,
            loopback_destination(subresource_port).await,
        ];
        let upstream_task = tokio::spawn(async move {
            let (mut peer, _) = subresource.accept().await.unwrap();
            let mut request = [0_u8; 4];
            peer.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"PING");
            peer.write_all(b"PONG").await.unwrap();
        });

        let proxy = start_browser_connect_proxy_for_destinations(destinations)
            .await
            .unwrap();
        let proxy_port = proxy.proxy_url.port_or_known_default().unwrap();

        let mut rejected =
            tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, proxy_port))
                .await
                .unwrap();
        rejected
            .write_all(
                format!(
                    "CONNECT 127.0.0.1:{undeclared_port} HTTP/1.1\r\nHost: 127.0.0.1:{undeclared_port}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut rejection = [0_u8; 256];
        let rejection_bytes = rejected.read(&mut rejection).await.unwrap();
        assert!(String::from_utf8_lossy(&rejection[..rejection_bytes])
            .starts_with("HTTP/1.1 403 Forbidden"));

        let mut approved =
            tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, proxy_port))
                .await
                .unwrap();
        approved
            .write_all(
                format!(
                    "CONNECT 127.0.0.1:{subresource_port} HTTP/1.1\r\nHost: 127.0.0.1:{subresource_port}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut handshake = [0_u8; 256];
        let handshake_bytes = approved.read(&mut handshake).await.unwrap();
        assert!(String::from_utf8_lossy(&handshake[..handshake_bytes])
            .starts_with("HTTP/1.1 200 Connection Established"));
        approved.write_all(b"PING").await.unwrap();
        let mut response = [0_u8; 4];
        approved.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"PONG");

        upstream_task.await.unwrap();
        drop(primary);
        drop(undeclared);
        drop(proxy);
    }

    #[test]
    fn hidden_subresources_accept_only_bounded_exact_https_port_443_hosts() {
        let hosts = vec![
            "*.kayak.com".to_string(),
            "*".to_string(),
            "content.r9cdn.net".to_string(),
            "https://content.r9cdn.net:443".to_string(),
            "http://insecure.example".to_string(),
            "https://wrong-port.example:444".to_string(),
            "https://path.example/not-an-origin".to_string(),
            "https://user:secret@credentials.example".to_string(),
        ];

        assert_eq!(
            exact_https_subresource_urls(&hosts).unwrap(),
            vec!["https://content.r9cdn.net/".to_string()]
        );

        let too_many = (0..=BROWSER_PROXY_MAX_HTTPS_SUBRESOURCE_HOSTS)
            .map(|index| format!("cdn-{index}.example"))
            .collect::<Vec<_>>();
        assert!(exact_https_subresource_urls(&too_many).is_err());
    }
}
