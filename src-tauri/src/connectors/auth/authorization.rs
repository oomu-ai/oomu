use super::super::ConnectorManifest;
use super::{manifest, microsoft365, repository};
use crate::db::PersistenceEngine;
use url::Url;

type PreparedAuthorization = (u32, String, Vec<String>, Option<String>);

pub(super) fn prepare(
    engine: &PersistenceEngine,
    manifest_id: &str,
    existing_connector_id: Option<&str>,
    requested_operations: &[String],
) -> Result<PreparedAuthorization, String> {
    let requested_scopes = scopes_for_operations(manifest_id, requested_operations)?;
    let descriptor = manifest::manifest(manifest_id)?;
    ensure_operations_available(&descriptor, requested_operations)?;
    let client_id = manifest::oauth_client_id(manifest_id)
        .ok_or_else(|| "OAuth client identity is unavailable.".to_string())?
        .to_string();
    let prepared_existing = existing_connector_id
        .map(|connector_id| repository::validate_oauth_account(engine, connector_id, manifest_id))
        .transpose()?;
    let requested_scopes = merge_existing_scopes(
        engine,
        manifest_id,
        prepared_existing.as_deref(),
        requested_scopes,
    )?;
    Ok((
        descriptor.version,
        client_id,
        requested_scopes,
        prepared_existing,
    ))
}

fn scopes_for_operations(
    manifest_id: &str,
    requested_operations: &[String],
) -> Result<Vec<String>, String> {
    match manifest_id {
        microsoft365::MANIFEST_ID => microsoft365::requested_scopes(requested_operations),
        "google_workspace" => manifest::google_requested_scopes(requested_operations),
        "slack" => manifest::slack_requested_scopes(requested_operations),
        _ if requested_operations.is_empty() => Ok(manifest::oauth_base_scopes(manifest_id)),
        _ => Err("connector_incremental_consent_unsupported".to_string()),
    }
}

fn ensure_operations_available(
    descriptor: &ConnectorManifest,
    requested_operations: &[String],
) -> Result<(), String> {
    if !descriptor.supported {
        return Err(descriptor
            .availability_reason_code
            .clone()
            .unwrap_or_else(|| "connector_unsupported_build".to_string()));
    }
    for operation in requested_operations {
        let grant = descriptor
            .operation_grants
            .iter()
            .find(|grant| grant.operation == operation.as_str())
            .ok_or_else(|| "connector_incremental_consent_unsupported".to_string())?;
        if !grant.available {
            return Err(grant
                .unavailable_reason_code
                .clone()
                .unwrap_or_else(|| "connector_operation_unavailable".to_string()));
        }
    }
    Ok(())
}

fn merge_existing_scopes(
    engine: &PersistenceEngine,
    manifest_id: &str,
    connector_id: Option<&str>,
    mut requested_scopes: Vec<String>,
) -> Result<Vec<String>, String> {
    let Some(connector_id) = connector_id else {
        return Ok(requested_scopes);
    };
    let granted_scopes = repository::account_granted_scopes(engine, connector_id)?;
    if manifest_id == microsoft365::MANIFEST_ID {
        return Ok(microsoft365::merge_scopes(
            &requested_scopes,
            &granted_scopes,
        ));
    }
    if manifest_id == "google_workspace" {
        requested_scopes = manifest::normalize_google_scopes(&requested_scopes);
        for scope in manifest::normalize_google_scopes(&granted_scopes) {
            if !requested_scopes.contains(&scope) {
                requested_scopes.push(scope);
            }
        }
    }
    Ok(requested_scopes)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn authorization_url(
    manifest_id: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
    nonce: &str,
    requested_scopes: &[String],
    requested_operations: &[String],
    created_new_account: bool,
) -> Result<Url, String> {
    let mut authorization = authorization_endpoint(manifest_id)?;
    authorization
        .query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    match manifest_id {
        "google_workspace" => {
            append_google_parameters(&mut authorization, requested_scopes, created_new_account)
        }
        "slack" => append_slack_parameters(&mut authorization, requested_operations)?,
        _ => append_microsoft_parameters(
            &mut authorization,
            requested_scopes,
            nonce,
            created_new_account,
        ),
    };
    Ok(authorization)
}

fn authorization_endpoint(manifest_id: &str) -> Result<Url, String> {
    let endpoint = match manifest_id {
        "google_workspace" => "https://accounts.google.com/o/oauth2/v2/auth",
        "slack" => "https://slack.com/oauth/v2/authorize",
        microsoft365::MANIFEST_ID => microsoft365::AUTHORIZATION_ENDPOINT,
        _ => return Err("Connector does not use OAuth.".to_string()),
    };
    Url::parse(endpoint).map_err(|error| error.to_string())
}

fn append_google_parameters(
    authorization: &mut Url,
    requested_scopes: &[String],
    created_new_account: bool,
) {
    authorization
        .query_pairs_mut()
        .append_pair("scope", &requested_scopes.join(" "))
        .append_pair("access_type", "offline")
        .append_pair(
            "prompt",
            if created_new_account {
                "select_account consent"
            } else {
                "consent"
            },
        );
}

fn append_slack_parameters(
    authorization: &mut Url,
    requested_operations: &[String],
) -> Result<(), String> {
    authorization
        .query_pairs_mut()
        .append_pair("user_scope", &manifest::slack_read_scopes().join(","));
    let bot_scopes = manifest::slack_bot_scopes(requested_operations)?;
    if !bot_scopes.is_empty() {
        authorization
            .query_pairs_mut()
            .append_pair("scope", &bot_scopes.join(","));
    }
    Ok(())
}

fn append_microsoft_parameters(
    authorization: &mut Url,
    requested_scopes: &[String],
    nonce: &str,
    created_new_account: bool,
) {
    authorization
        .query_pairs_mut()
        .append_pair("scope", &requested_scopes.join(" "))
        .append_pair("response_mode", "query")
        .append_pair("nonce", nonce)
        .append_pair(
            "prompt",
            if created_new_account {
                "select_account"
            } else {
                "consent"
            },
        );
}
