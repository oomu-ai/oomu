use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::signature::{RsaPublicKeyComponents, RSA_PKCS1_2048_8192_SHA256};
use serde_json::Value;
use std::{
    io::Read,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use url::Url;

const DISCOVERY_ENDPOINT: &str =
    "https://login.microsoftonline.com/common/v2.0/.well-known/openid-configuration";
const ISSUER_TEMPLATE: &str = "https://login.microsoftonline.com/{tenantid}/v2.0";
const JWKS_PATH: &str = "/common/discovery/v2.0/keys";
const CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const CLOCK_SKEW_SECONDS: i64 = 5 * 60;
const MAX_DOCUMENT_BYTES: u64 = 512 * 1024;
const MAX_TOKEN_BYTES: usize = 64 * 1024;

#[derive(Clone)]
struct OidcDocuments {
    issuer_template: String,
    jwks: Value,
}

#[derive(Clone, Debug)]
pub(super) struct TokenIdentityEvidence {
    pub(super) tenant_id: String,
    pub(super) account_hint: Option<String>,
    pub(super) tenant_hint: Option<String>,
}

static CACHE: OnceLock<Mutex<Option<(Instant, OidcDocuments)>>> = OnceLock::new();

pub(super) fn valid_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'@'))
}

fn canonical_guid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn bounded_json_get(url: &str, failure_code: &str) -> Result<Value, String> {
    let mut response = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| failure_code.to_string())?
        .get(url)
        .send()
        .map_err(|_| failure_code.to_string())?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_DOCUMENT_BYTES)
    {
        return Err(failure_code.to_string());
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(MAX_DOCUMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failure_code.to_string())?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(failure_code.to_string());
    }
    serde_json::from_slice(&bytes).map_err(|_| failure_code.to_string())
}

fn validate_discovery(discovery: &Value) -> Result<(String, String), String> {
    let issuer = discovery
        .get("issuer")
        .and_then(Value::as_str)
        .filter(|issuer| *issuer == ISSUER_TEMPLATE)
        .ok_or_else(|| "microsoft_oidc_discovery_invalid".to_string())?;
    let jwks_uri = discovery
        .get("jwks_uri")
        .and_then(Value::as_str)
        .ok_or_else(|| "microsoft_oidc_discovery_invalid".to_string())?;
    let url = Url::parse(jwks_uri).map_err(|_| "microsoft_oidc_discovery_invalid".to_string())?;
    if url.scheme() != "https"
        || url.host_str() != Some("login.microsoftonline.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != JWKS_PATH
    {
        return Err("microsoft_oidc_discovery_invalid".to_string());
    }
    Ok((issuer.to_string(), jwks_uri.to_string()))
}

fn oidc_documents(force_refresh: bool) -> Result<OidcDocuments, String> {
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if !force_refresh {
        let guard = cache
            .lock()
            .map_err(|_| "microsoft_oidc_cache_unavailable".to_string())?;
        if let Some((loaded_at, documents)) = guard.as_ref() {
            if loaded_at.elapsed() < CACHE_TTL {
                return Ok(documents.clone());
            }
        }
    }
    let discovery = bounded_json_get(DISCOVERY_ENDPOINT, "microsoft_oidc_discovery_unavailable")?;
    let (issuer_template, jwks_uri) = validate_discovery(&discovery)?;
    let jwks = bounded_json_get(&jwks_uri, "microsoft_oidc_keys_unavailable")?;
    if jwks
        .get("keys")
        .and_then(Value::as_array)
        .is_none_or(|keys| keys.is_empty() || keys.len() > 64)
    {
        return Err("microsoft_oidc_keys_invalid".to_string());
    }
    let documents = OidcDocuments {
        issuer_template,
        jwks,
    };
    *cache
        .lock()
        .map_err(|_| "microsoft_oidc_cache_unavailable".to_string())? =
        Some((Instant::now(), documents.clone()));
    Ok(documents)
}

fn verified_claims(id_token: &str, jwks: &Value) -> Result<Value, String> {
    if id_token.len() > MAX_TOKEN_BYTES {
        return Err("microsoft_id_token_invalid".to_string());
    }
    let parts: Vec<&str> = id_token.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return Err("microsoft_id_token_invalid".to_string());
    }
    let header: Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(parts[0])
            .map_err(|_| "microsoft_id_token_invalid".to_string())?,
    )
    .map_err(|_| "microsoft_id_token_invalid".to_string())?;
    if header.get("alg").and_then(Value::as_str) != Some("RS256") {
        return Err("microsoft_id_token_algorithm_rejected".to_string());
    }
    let kid = header
        .get("kid")
        .and_then(Value::as_str)
        .filter(|kid| valid_identifier(kid, 256))
        .ok_or_else(|| "microsoft_id_token_kid_missing".to_string())?;
    let keys = jwks
        .get("keys")
        .and_then(Value::as_array)
        .ok_or_else(|| "microsoft_oidc_keys_invalid".to_string())?;
    let mut matches = keys.iter().filter(|key| {
        key.get("kid").and_then(Value::as_str) == Some(kid)
            && key.get("kty").and_then(Value::as_str) == Some("RSA")
            && key
                .get("use")
                .and_then(Value::as_str)
                .is_none_or(|value| value == "sig")
            && key
                .get("alg")
                .and_then(Value::as_str)
                .is_none_or(|value| value == "RS256")
    });
    let key = matches
        .next()
        .ok_or_else(|| "microsoft_id_token_unknown_kid".to_string())?;
    if matches.next().is_some() {
        return Err("microsoft_id_token_duplicate_kid".to_string());
    }
    let modulus = key
        .get("n")
        .and_then(Value::as_str)
        .and_then(|value| URL_SAFE_NO_PAD.decode(value).ok())
        .filter(|value| (256..=1024).contains(&value.len()))
        .ok_or_else(|| "microsoft_id_token_key_invalid".to_string())?;
    let exponent = key
        .get("e")
        .and_then(Value::as_str)
        .and_then(|value| URL_SAFE_NO_PAD.decode(value).ok())
        .filter(|value| !value.is_empty() && value.len() <= 8)
        .ok_or_else(|| "microsoft_id_token_key_invalid".to_string())?;
    let signature = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| "microsoft_id_token_signature_invalid".to_string())?;
    RsaPublicKeyComponents {
        n: &modulus,
        e: &exponent,
    }
    .verify(
        &RSA_PKCS1_2048_8192_SHA256,
        format!("{}.{}", parts[0], parts[1]).as_bytes(),
        &signature,
    )
    .map_err(|_| "microsoft_id_token_signature_invalid".to_string())?;
    serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| "microsoft_id_token_invalid".to_string())?,
    )
    .map_err(|_| "microsoft_id_token_invalid".to_string())
}

fn claim_string<'a>(claims: &'a Value, key: &str) -> Option<&'a str> {
    claims
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn validate_audience(claims: &Value, client_id: &str) -> bool {
    let Some(audience) = claims.get("aud") else {
        return false;
    };
    if audience.as_str() == Some(client_id) {
        return true;
    }
    let Some(items) = audience.as_array() else {
        return false;
    };
    !items.is_empty()
        && items.iter().all(Value::is_string)
        && items.iter().any(|item| item.as_str() == Some(client_id))
        && (items.len() == 1 || claim_string(claims, "azp") == Some(client_id))
}

fn validate_time_claims(claims: &Value, now_seconds: i64) -> Result<(), String> {
    if claims
        .get("exp")
        .and_then(Value::as_i64)
        .and_then(|expires| expires.checked_add(CLOCK_SKEW_SECONDS))
        .is_none_or(|expires| expires <= now_seconds)
    {
        return Err("microsoft_id_token_expired".to_string());
    }
    if claims
        .get("nbf")
        .and_then(Value::as_i64)
        .is_some_and(|not_before| {
            now_seconds
                .checked_add(CLOCK_SKEW_SECONDS)
                .is_none_or(|latest| not_before > latest)
        })
    {
        return Err("microsoft_id_token_not_yet_valid".to_string());
    }
    Ok(())
}

fn parse_id_token_evidence(
    id_token: &str,
    expected_client_id: &str,
    expected_nonce: &str,
    now_seconds: i64,
    issuer_template: &str,
    jwks: &Value,
) -> Result<TokenIdentityEvidence, String> {
    let claims = verified_claims(id_token, jwks)?;
    if !validate_audience(&claims, expected_client_id) {
        return Err("microsoft_id_token_audience_mismatch".to_string());
    }
    if claim_string(&claims, "nonce") != Some(expected_nonce) {
        return Err("microsoft_id_token_nonce_mismatch".to_string());
    }
    validate_time_claims(&claims, now_seconds)?;
    let tenant_id = claim_string(&claims, "tid")
        .filter(|tenant| canonical_guid(tenant))
        .ok_or_else(|| "microsoft_tenant_identity_missing".to_string())?
        .to_string();
    let expected_issuer = issuer_template.replace("{tenantid}", &tenant_id);
    if claim_string(&claims, "iss") != Some(expected_issuer.as_str()) {
        return Err("microsoft_id_token_issuer_mismatch".to_string());
    }
    Ok(TokenIdentityEvidence {
        tenant_id,
        account_hint: claim_string(&claims, "preferred_username")
            .or_else(|| claim_string(&claims, "email"))
            .map(str::to_string),
        tenant_hint: claim_string(&claims, "tenant_name").map(str::to_string),
    })
}

pub(super) fn validated_id_token_evidence(
    id_token: &str,
    expected_client_id: &str,
    expected_nonce: &str,
    now_seconds: i64,
) -> Result<TokenIdentityEvidence, String> {
    let documents = oidc_documents(false)?;
    match parse_id_token_evidence(
        id_token,
        expected_client_id,
        expected_nonce,
        now_seconds,
        &documents.issuer_template,
        &documents.jwks,
    ) {
        Err(code) if code == "microsoft_id_token_unknown_kid" => {
            let refreshed = oidc_documents(true)?;
            parse_id_token_evidence(
                id_token,
                expected_client_id,
                expected_nonce,
                now_seconds,
                &refreshed.issuer_template,
                &refreshed.jwks,
            )
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNED_TOKEN: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InRlc3Qta2V5IiwidHlwIjoiSldUIn0.eyJhdWQiOiJjbGllbnQiLCJub25jZSI6Im5vbmNlIiwiZXhwIjoyMDAwMDAwMDAwLCJ0aWQiOiIxMTExMTExMS0yMjIyLTMzMzMtNDQ0NC01NTU1NTU1NTU1NTUiLCJpc3MiOiJodHRwczovL2xvZ2luLm1pY3Jvc29mdG9ubGluZS5jb20vMTExMTExMTEtMjIyMi0zMzMzLTQ0NDQtNTU1NTU1NTU1NTU1L3YyLjAifQ.YOLQk8bsrJGI0PHR9RlPCaUzgy-zigcQiZ6B8U3PiNPB56LhrgtwGJGNHl1ef9z4UfvNW3hpRIX_Bp55AA4965hsYv0_zBOsWjDBY5ichTx7is2YutGW2uTOD-FdAbeiumYNDoVlRfoS3Lilot1PuLu4hB9NH6yUQMkX-VSQ9uioXWsKOnNVyNiLHNfcFpLsy6xAaHE34keytSlDwuOuC7ftr7Gm1c9V3w6Z9kqv_bzbR-GvyjnYfKVkWff04khULHm7AXzys75lYT5GWuKMzqr4vmZo3lVTHsuWaPj6S1Kn9McwVGIEGE08xF4n0_vw8aXImy0FsmRuiLl0HppESQ";
    const MODULUS: &str = "sNHdwnvC12deiKngdnQBn0lnDBl1VHQk4TXz-2E03QQlla-teaMLFrxb-bVnmLcceBmSQEz032M14ucgV2cuXuotVFw9CYEE11wpXvV_NqhwSeMXVWB6RwEzligVgLWkpgi6Zmq0_xJBuiRZMI7iQcK7QhL4dbQo-ucAUZI0w3gkI1I4eGoySbMlD0fKHwVNHCskVuSnhTEgdqq_NSIwk0k5HSbq4ctt7m5vlvPeiF9Z8pj3tvLrk6lJ9qQjMW9DP7ZARB1F-sy_6TfrHK8XWQECvYYV12Cpt5VVbJUw8gVvm27kPSBLcJ5GL3xnFkow5A5x8A_-HVDfrzdRt2_HMw";

    fn jwks() -> Value {
        serde_json::json!({"keys":[{
            "kty":"RSA", "kid":"test-key", "use":"sig", "alg":"RS256",
            "n":MODULUS, "e":"AQAB"
        }]})
    }

    fn parse(client: &str, nonce: &str, issuer: &str) -> Result<TokenIdentityEvidence, String> {
        parse_id_token_evidence(SIGNED_TOKEN, client, nonce, 1, issuer, &jwks())
    }

    #[test]
    fn real_rs256_signature_and_bound_claims_are_required() {
        let evidence = parse("client", "nonce", ISSUER_TEMPLATE).unwrap();
        assert_eq!(evidence.tenant_id, "11111111-2222-3333-4444-555555555555");
        assert_eq!(
            parse("other-client", "nonce", ISSUER_TEMPLATE).unwrap_err(),
            "microsoft_id_token_audience_mismatch"
        );
        assert_eq!(
            parse("client", "other-nonce", ISSUER_TEMPLATE).unwrap_err(),
            "microsoft_id_token_nonce_mismatch"
        );
        assert_eq!(
            parse("client", "nonce", "https://invalid/{tenantid}").unwrap_err(),
            "microsoft_id_token_issuer_mismatch"
        );
    }

    #[test]
    fn tampered_signature_and_unknown_kid_fail_closed() {
        let mut parts: Vec<String> = SIGNED_TOKEN.split('.').map(str::to_string).collect();
        parts[2].replace_range(..1, "A");
        let tampered = parts.join(".");
        assert_eq!(
            verified_claims(&tampered, &jwks()).unwrap_err(),
            "microsoft_id_token_signature_invalid"
        );
        parts = SIGNED_TOKEN.split('.').map(str::to_string).collect();
        let mut claims: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(&parts[1]).unwrap()).unwrap();
        claims["aud"] = Value::String("attacker-client".to_string());
        parts[1] = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        assert_eq!(
            verified_claims(&parts.join("."), &jwks()).unwrap_err(),
            "microsoft_id_token_signature_invalid"
        );
        assert_eq!(
            verified_claims(SIGNED_TOKEN, &serde_json::json!({"keys":[]})).unwrap_err(),
            "microsoft_id_token_unknown_kid"
        );
        let unknown_header = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({
                "alg":"RS256", "kid":"rotated-key", "typ":"JWT"
            }))
            .unwrap(),
        );
        parts = SIGNED_TOKEN.split('.').map(str::to_string).collect();
        parts[0] = unknown_header;
        assert_eq!(
            verified_claims(&parts.join("."), &jwks()).unwrap_err(),
            "microsoft_id_token_unknown_kid"
        );
    }

    #[test]
    fn discovery_is_pinned_to_the_common_tenant_key_endpoint() {
        assert!(validate_discovery(&serde_json::json!({
            "issuer": ISSUER_TEMPLATE,
            "jwks_uri":"https://login.microsoftonline.com/common/discovery/v2.0/keys"
        }))
        .is_ok());
        assert!(validate_discovery(&serde_json::json!({
            "issuer": ISSUER_TEMPLATE,
            "jwks_uri":"https://example.com/common/discovery/v2.0/keys"
        }))
        .is_err());
    }

    #[test]
    fn tenant_identifier_must_be_a_canonical_guid() {
        assert!(canonical_guid("11111111-2222-3333-4444-555555555555"));
        assert!(!canonical_guid("organizations"));
        assert!(!canonical_guid("11111111222233334444555555555555"));
    }

    #[test]
    fn multiple_audiences_require_authorized_party_and_nbf_allows_only_clock_skew() {
        let mut claims = serde_json::json!({"aud":["client","api"],"azp":"client"});
        assert!(validate_audience(&claims, "client"));
        claims.as_object_mut().unwrap().remove("azp");
        assert!(!validate_audience(&claims, "client"));

        assert!(validate_time_claims(&serde_json::json!({"exp":1_001,"nbf":1_300}), 1_000).is_ok());
        assert_eq!(
            validate_time_claims(
                &serde_json::json!({"exp":1_000 - CLOCK_SKEW_SECONDS}),
                1_000
            )
            .unwrap_err(),
            "microsoft_id_token_expired"
        );
        assert_eq!(
            validate_time_claims(&serde_json::json!({"exp":2_000,"nbf":1_301}), 1_000).unwrap_err(),
            "microsoft_id_token_not_yet_valid"
        );
    }
}
