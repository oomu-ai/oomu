use reqwest::header::{HeaderMap, CONTENT_LENGTH, CONTENT_RANGE};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use tokio::net::lookup_host;
use url::{Host, Url};

use super::state::InstallError;

const MAX_RESPONSE_HEADER_BYTES: usize = 64 * 1024;

pub(crate) async fn validate_network_destination(
    url: &Url,
    allow_local_fixture: bool,
) -> Result<(), InstallError> {
    if url.scheme() != "https" && !(allow_local_fixture && url.scheme() == "http") {
        return Err(InstallError::new(
            "model_install_transport_refused",
            false,
            "download destination was not HTTPS",
        ));
    }
    let host = url.host().ok_or_else(|| {
        InstallError::new(
            "model_install_transport_refused",
            false,
            "download destination omitted a host",
        )
    })?;
    let port = url.port_or_known_default().ok_or_else(|| {
        InstallError::new(
            "model_install_transport_refused",
            false,
            "download destination omitted a usable port",
        )
    })?;
    match host {
        Host::Ipv4(address) => reject_non_public_ip(IpAddr::V4(address), allow_local_fixture),
        Host::Ipv6(address) => reject_non_public_ip(IpAddr::V6(address), allow_local_fixture),
        Host::Domain(domain) => {
            if domain.eq_ignore_ascii_case("localhost") && !allow_local_fixture {
                return Err(private_network_error());
            }
            let resolved = lookup_host((domain, port)).await.map_err(|error| {
                InstallError::new("model_install_dns_failed", true, error.to_string())
            })?;
            let mut count = 0_usize;
            for address in resolved {
                count += 1;
                reject_non_public_ip(address.ip(), allow_local_fixture)?;
            }
            if count == 0 {
                return Err(InstallError::new(
                    "model_install_dns_failed",
                    true,
                    "download host did not resolve",
                ));
            }
            Ok(())
        }
    }
}

pub(crate) fn validate_content_range(
    headers: &HeaderMap,
    expected_start: u64,
    expected_total: u64,
) -> Result<(), InstallError> {
    let value = headers
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            InstallError::new(
                "model_install_content_range_invalid",
                true,
                "206 response omitted Content-Range",
            )
        })?;
    let (range, total) = value
        .strip_prefix("bytes ")
        .and_then(|value| value.split_once('/'))
        .ok_or_else(|| {
            InstallError::new(
                "model_install_content_range_invalid",
                true,
                "Content-Range was malformed",
            )
        })?;
    let (start, end) = range.split_once('-').ok_or_else(|| {
        InstallError::new(
            "model_install_content_range_invalid",
            true,
            "Content-Range interval was malformed",
        )
    })?;
    let start = start.parse::<u64>().ok();
    let end = end.parse::<u64>().ok();
    let total = total.parse::<u64>().ok();
    if start != Some(expected_start)
        || total != Some(expected_total)
        || end.is_none()
        || end.is_some_and(|end| end < expected_start || end >= expected_total)
    {
        return Err(InstallError::new(
            "model_install_content_range_invalid",
            true,
            "Content-Range did not match the immutable asset",
        ));
    }
    Ok(())
}

pub(crate) fn validate_declared_length(
    headers: &HeaderMap,
    expected: u64,
) -> Result<(), InstallError> {
    if let Some(length) = headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        if length != expected {
            return Err(InstallError::new(
                "model_install_size_mismatch",
                false,
                "Content-Length did not match the immutable remaining byte count",
            ));
        }
    }
    Ok(())
}

pub(crate) fn enforce_header_ceiling(headers: &HeaderMap) -> Result<(), InstallError> {
    let bytes = headers
        .iter()
        .map(|(name, value)| name.as_str().len() + value.as_bytes().len())
        .sum::<usize>();
    if bytes > MAX_RESPONSE_HEADER_BYTES {
        return Err(InstallError::new(
            "model_install_headers_too_large",
            false,
            "response headers exceeded the native byte ceiling",
        ));
    }
    Ok(())
}

pub(crate) fn sanitize_etag(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| !value.is_empty() && value.len() <= 512)
        .filter(|value| {
            value
                .bytes()
                .all(|byte| byte.is_ascii_graphic() || byte == b' ')
        })
        .map(str::to_string)
}

fn reject_non_public_ip(address: IpAddr, allow_local_fixture: bool) -> Result<(), InstallError> {
    if allow_local_fixture && address.is_loopback() {
        return Ok(());
    }
    let refused = match address {
        IpAddr::V4(address) => ipv4_is_non_public(address),
        IpAddr::V6(address) => ipv6_is_non_public(address),
    };
    if refused {
        Err(private_network_error())
    } else {
        Ok(())
    }
}

fn ipv4_is_non_public(address: Ipv4Addr) -> bool {
    let [a, b, _, _] = address.octets();
    address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_documentation()
        || address.is_unspecified()
        || address.is_multicast()
        || (a == 100 && (64..=127).contains(&b))
        || (a == 198 && matches!(b, 18 | 19))
        || a == 0
        || a >= 240
}

fn ipv6_is_non_public(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    address.is_loopback()
        || address.is_unspecified()
        || address.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || address.to_ipv4_mapped().is_some_and(ipv4_is_non_public)
}

fn private_network_error() -> InstallError {
    InstallError::new(
        "model_install_private_network_refused",
        false,
        "download destination resolved to a non-public network",
    )
}
