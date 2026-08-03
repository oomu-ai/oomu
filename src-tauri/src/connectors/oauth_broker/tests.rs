use super::*;

#[test]
fn oauth_public_client_broker_configuration_is_fail_closed() {
    if BROKER_URL.is_none() || BROKER_CERT_SHA256.is_none() {
        assert_eq!(
            broker_configuration().unwrap_err(),
            "oauth_broker_unconfigured"
        );
    }
}

#[test]
fn oauth_secret_absence_broker_schema_has_no_shared_secret_field() {
    let source = include_str!("../oauth_broker.rs");
    assert!(!source.contains("client_\u{73}ecret"));
    assert!(!source.contains("bearer_\u{73}ecret"));
}

#[test]
fn oauth_public_client_broker_response_is_bounded_and_length_delimited() {
    let complete = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
    assert_eq!(
        complete_http_response_length(complete).unwrap(),
        Some(complete.len())
    );
    assert_eq!(
        complete_http_response_length(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n"
        )
        .unwrap_err(),
        "oauth_broker_response_invalid"
    );
    assert_eq!(
        complete_http_response_length(b"HTTP/1.1 200 OK\r\n\r\n{}").unwrap_err(),
        "oauth_broker_response_invalid"
    );
}
