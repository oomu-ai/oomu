use super::super::{auth::ConnectorCredential, ConnectorIdentityMetadata};
use super::contract::{data_routing, COMMON_TOKEN_ENDPOINT, GRAPH_ROOT, MANIFEST_ID};
use super::http::{auth_client, graph_client};
use super::oidc::{valid_identifier, validated_id_token_evidence, TokenIdentityEvidence};
use crate::foundation::{clock::unix_time_ms_i64, digest::sha256_hex};
use reqwest::StatusCode;
use serde_json::Value;
use std::io::Read;

const PERSONAL_TENANT_ID: &str = "9188040d-6c67-4c5b-b112-36a304b66dad";
pub(in crate::connectors) struct ExchangeRequest<'a> {
    pub client_id: &'a str,
    pub code: &'a str,
    pub verifier: &'a str,
    pub redirect_uri: &'a str,
    pub nonce: &'a str,
    pub requested_scopes: &'a [String],
}

pub(in crate::connectors) struct IdentityResult {
    pub label: String,
    pub subject: String,
    pub metadata: ConnectorIdentityMetadata,
}

fn oauth_error(phase: &str, status: StatusCode, body: &Value) -> String {
    let category = if status == StatusCode::TOO_MANY_REQUESTS {
        "rate_limited"
    } else if status.is_server_error() {
        "unavailable"
    } else {
        match body.get("error").and_then(Value::as_str) {
            Some("invalid_grant") if phase == "refresh" => "revoked",
            Some("invalid_grant") => "invalid_grant",
            Some("invalid_request") => "invalid_request",
            Some("access_denied") => "access_denied",
            Some("invalid_client") | Some("unauthorized_client") => "invalid_client",
            Some("invalid_scope") => "invalid_scope",
            Some("interaction_required") | Some("consent_required") => "tenant_policy",
            Some("temporarily_unavailable") | Some("server_error") => "unavailable",
            _ => "rejected",
        }
    };
    format!("microsoft_{phase}_{category}")
}

fn parse_token_response(
    phase: &str,
    mut response: reqwest::blocking::Response,
) -> Result<Value, String> {
    let status = response.status();
    const MAX_TOKEN_RESPONSE_BYTES: u64 = 256 * 1024;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_TOKEN_RESPONSE_BYTES)
    {
        return Err(format!("microsoft_{phase}_response_too_large"));
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(MAX_TOKEN_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("microsoft_{phase}_response_invalid"))?;
    if bytes.len() as u64 > MAX_TOKEN_RESPONSE_BYTES {
        return Err(format!("microsoft_{phase}_response_too_large"));
    }
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if !status.is_success() || body.get("error").is_some() {
        return Err(oauth_error(phase, status, &body));
    }
    Ok(body)
}

fn scope_values(body: &Value, fallback: &[String]) -> Vec<String> {
    body.get("scope")
        .and_then(Value::as_str)
        .map(|raw| {
            raw.split_whitespace()
                .filter(|scope| !scope.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| fallback.to_vec())
}

fn require_returned_scopes(returned: &[String], requested: &[String]) -> Result<(), String> {
    let missing = requested.iter().any(|required| {
        !matches!(
            required.as_str(),
            "openid" | "profile" | "email" | "offline_access"
        ) && !returned
            .iter()
            .any(|scope| scope.eq_ignore_ascii_case(required))
    });
    if missing {
        Err("microsoft_token_scope_mismatch".to_string())
    } else {
        Ok(())
    }
}

fn expires_at_ms(body: &Value) -> Result<Option<i64>, String> {
    let Some(seconds) = body.get("expires_in").and_then(Value::as_i64) else {
        return Ok(None);
    };
    if seconds <= 0 {
        return Err("microsoft_token_expiry_invalid".to_string());
    }
    unix_time_ms_i64()
        .checked_add(
            seconds
                .checked_mul(1_000)
                .ok_or_else(|| "microsoft_token_expiry_invalid".to_string())?,
        )
        .map(Some)
        .ok_or_else(|| "microsoft_token_expiry_invalid".to_string())
}

pub(in crate::connectors) fn exchange(
    request: ExchangeRequest<'_>,
) -> Result<ConnectorCredential, String> {
    let response = auth_client()?
        .post(COMMON_TOKEN_ENDPOINT)
        .form(&[
            ("client_id", request.client_id.to_string()),
            ("code", request.code.to_string()),
            ("code_verifier", request.verifier.to_string()),
            ("redirect_uri", request.redirect_uri.to_string()),
            ("grant_type", "authorization_code".to_string()),
            ("scope", request.requested_scopes.join(" ")),
        ])
        .send()
        .map_err(|_| "microsoft_token_offline".to_string())?;
    let body = parse_token_response("token", response)?;
    let returned_scopes = scope_values(&body, request.requested_scopes);
    require_returned_scopes(&returned_scopes, request.requested_scopes)?;
    let id_token = body
        .get("id_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "microsoft_id_token_missing".to_string())?;
    let evidence = validated_id_token_evidence(
        id_token,
        request.client_id,
        request.nonce,
        unix_time_ms_i64() / 1_000,
    )?;
    Ok(ConnectorCredential {
        manifest_id: MANIFEST_ID.to_string(),
        access_token: body
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| "microsoft_access_token_missing".to_string())?
            .to_string(),
        bot_access_token: None,
        refresh_token: body
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_string),
        token_type: body
            .get("token_type")
            .and_then(Value::as_str)
            .unwrap_or("Bearer")
            .to_string(),
        scopes: returned_scopes,
        expires_at_ms: Some(
            expires_at_ms(&body)?.ok_or_else(|| "microsoft_token_expiry_missing".to_string())?,
        ),
        refresh_expires_at_ms: None,
        tenant_id: Some(evidence.tenant_id),
        tenant_label: evidence.tenant_hint,
        account_id: None,
        account_principal: evidence.account_hint,
        identity_binding_hash: None,
    })
}

fn refresh_form(
    client_id: &str,
    refresh_token: &str,
    scopes: &[String],
) -> Vec<(&'static str, String)> {
    vec![
        ("client_id", client_id.to_string()),
        ("refresh_token", refresh_token.to_string()),
        ("grant_type", "refresh_token".to_string()),
        ("scope", scopes.join(" ")),
    ]
}

pub(in crate::connectors) fn refresh(
    current: &ConnectorCredential,
    client_id: &str,
) -> Result<ConnectorCredential, String> {
    let refresh_token = current
        .refresh_token
        .as_deref()
        .ok_or_else(|| "microsoft_refresh_token_missing".to_string())?;
    let endpoint = tenant_token_endpoint(current.tenant_id.as_deref());
    let response = auth_client()?
        .post(&endpoint)
        .form(&refresh_form(client_id, refresh_token, &current.scopes))
        .send()
        .map_err(|_| "microsoft_refresh_offline".to_string())?;
    let body = parse_token_response("refresh", response)?;
    let returned_scopes = scope_values(&body, &current.scopes);
    require_returned_scopes(&returned_scopes, &current.scopes)?;
    let mut refreshed = current.clone();
    refreshed.access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "microsoft_access_token_missing".to_string())?
        .to_string();
    if let Some(rotated) = body.get("refresh_token").and_then(Value::as_str) {
        refreshed.refresh_token = Some(rotated.to_string());
    }
    refreshed.scopes = returned_scopes;
    refreshed.expires_at_ms =
        Some(expires_at_ms(&body)?.ok_or_else(|| "microsoft_token_expiry_missing".to_string())?);
    Ok(refreshed)
}

pub(in crate::connectors) fn probe_identity(
    credential: &ConnectorCredential,
) -> Result<IdentityResult, String> {
    let response = graph_client()?
        .get(format!(
            "{GRAPH_ROOT}/me?$select=id,displayName,mail,userPrincipalName"
        ))
        .bearer_auth(&credential.access_token)
        .send()
        .map_err(|_| "microsoft_identity_offline".to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(graph_status_code(status));
    }
    let bytes = response
        .bytes()
        .map_err(|_| "microsoft_identity_response_invalid".to_string())?;
    if bytes.len() > 256 * 1024 {
        return Err("microsoft_identity_response_too_large".to_string());
    }
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "microsoft_identity_response_invalid".to_string())?;
    let evidence = TokenIdentityEvidence {
        tenant_id: credential
            .tenant_id
            .clone()
            .ok_or_else(|| "microsoft_tenant_identity_missing".to_string())?,
        account_hint: credential.account_principal.clone(),
        tenant_hint: credential.tenant_label.clone(),
    };
    let (label, subject, metadata) = metadata_from_graph_me(&evidence, &body)?;
    Ok(IdentityResult {
        label,
        subject,
        metadata,
    })
}

pub(super) fn graph_status_code(status: StatusCode) -> String {
    if status == StatusCode::UNAUTHORIZED {
        "microsoft_authorization_revoked"
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        "microsoft_rate_limited"
    } else if status == StatusCode::FORBIDDEN {
        "microsoft_tenant_policy_blocked"
    } else if status.is_server_error() {
        "microsoft_service_unavailable"
    } else {
        "microsoft_request_rejected"
    }
    .to_string()
}

pub(super) fn tenant_token_endpoint(tenant_id: Option<&str>) -> String {
    tenant_id
        .filter(|tenant| valid_identifier(tenant, 128))
        .map(|tenant| format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"))
        .unwrap_or_else(|| COMMON_TOKEN_ENDPOINT.to_string())
}

fn claim_string<'a>(claims: &'a Value, key: &str) -> Option<&'a str> {
    claims
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn metadata_from_graph_me(
    evidence: &TokenIdentityEvidence,
    me: &Value,
) -> Result<(String, String, ConnectorIdentityMetadata), String> {
    let account_id = claim_string(me, "id")
        .filter(|id| valid_identifier(id, 256))
        .ok_or_else(|| "microsoft_account_identity_missing".to_string())?
        .to_string();
    let principal = claim_string(me, "mail")
        .or_else(|| claim_string(me, "userPrincipalName"))
        .or_else(|| claim_string(me, "displayName"))
        .or(evidence.account_hint.as_deref())
        .unwrap_or(account_id.as_str())
        .to_string();
    let account_kind = if evidence.tenant_id == PERSONAL_TENANT_ID {
        "personal"
    } else {
        "work"
    };
    let tenant_label = evidence.tenant_hint.clone().unwrap_or_default();
    let subject = format!("{MANIFEST_ID}\0{}\0{account_id}", evidence.tenant_id);
    let binding = sha256_hex(subject.as_bytes());
    let now = unix_time_ms_i64();
    Ok((
        principal.clone(),
        subject,
        ConnectorIdentityMetadata {
            tenant_id: evidence.tenant_id.clone(),
            tenant_label,
            account_id,
            account_principal: principal,
            account_kind: account_kind.to_string(),
            identity_binding_hash: binding,
            data_routing: data_routing(),
            consent_reviewed_at_ms: now,
            identity_verified_at_ms: now,
        },
    ))
}

pub(in crate::connectors) fn tenant_binding_hash(tenant_id: Option<&str>) -> Option<String> {
    tenant_id.map(|tenant| sha256_hex(tenant.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_and_graph_subject_form_a_tenant_bound_identity() {
        let evidence = TokenIdentityEvidence {
            tenant_id: "11111111-2222-3333-4444-555555555555".to_string(),
            account_hint: None,
            tenant_hint: None,
        };
        let me: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/microsoft_365/graph_me_work.json"
        ))
        .unwrap();
        let (_, subject, metadata) = metadata_from_graph_me(&evidence, &me).unwrap();
        assert!(subject.contains(&metadata.tenant_id));
        assert!(subject.contains(&metadata.account_id));
        assert_eq!(
            metadata.identity_binding_hash,
            sha256_hex(subject.as_bytes())
        );
        let mut second_me = me;
        second_me["id"] = Value::String("bbbbbbbb-cccc-dddd-eeee-ffffffffffff".to_string());
        let (_, _, second) = metadata_from_graph_me(&evidence, &second_me).unwrap();
        assert_ne!(metadata.identity_binding_hash, second.identity_binding_hash);
    }

    #[test]
    fn personal_accounts_are_identified_without_synthesizing_display_copy() {
        let evidence = TokenIdentityEvidence {
            tenant_id: PERSONAL_TENANT_ID.to_string(),
            account_hint: Some("user@example.com".to_string()),
            tenant_hint: None,
        };
        let me = serde_json::json!({
            "id":"personal-account-id",
            "userPrincipalName":"user@example.com"
        });
        let (label, _, metadata) = metadata_from_graph_me(&evidence, &me).unwrap();
        assert_eq!(metadata.account_kind, "personal");
        assert_eq!(metadata.tenant_label, "");
        assert_eq!(label, "user@example.com");
    }

    #[test]
    fn refresh_form_retains_the_full_incremental_scope_set() {
        let scopes = vec!["User.Read".into(), "Mail.Read".into(), "Files.Read".into()];
        let form = refresh_form("client", "refresh", &scopes);
        assert!(form.iter().any(|(key, value)| {
            *key == "scope" && value.contains("Mail.Read") && value.contains("Files.Read")
        }));
    }

    #[test]
    fn graph_statuses_map_before_any_response_body_is_parsed() {
        assert_eq!(
            graph_status_code(StatusCode::TOO_MANY_REQUESTS),
            "microsoft_rate_limited"
        );
        assert_eq!(
            graph_status_code(StatusCode::UNAUTHORIZED),
            "microsoft_authorization_revoked"
        );
        assert_eq!(
            graph_status_code(StatusCode::FORBIDDEN),
            "microsoft_tenant_policy_blocked"
        );
    }

    #[test]
    fn invalid_or_overflowing_expiry_is_rejected() {
        assert!(expires_at_ms(&serde_json::json!({"expires_in": 0})).is_err());
        assert!(expires_at_ms(&serde_json::json!({"expires_in": i64::MAX})).is_err());
    }
}
