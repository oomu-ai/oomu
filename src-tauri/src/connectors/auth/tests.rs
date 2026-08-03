use super::*;

fn microsoft_credential(refresh_token: Option<&str>) -> ConnectorCredential {
    ConnectorCredential {
        manifest_id: microsoft365::MANIFEST_ID.to_string(),
        access_token: "access".to_string(),
        bot_access_token: None,
        refresh_token: refresh_token.map(str::to_string),
        token_type: "Bearer".to_string(),
        scopes: vec!["User.Read".to_string(), "Mail.Read".to_string()],
        expires_at_ms: Some(10_000),
        refresh_expires_at_ms: None,
        tenant_id: Some("tenant".to_string()),
        tenant_label: None,
        account_id: Some("account".to_string()),
        account_principal: Some("person@example.com".to_string()),
        identity_binding_hash: Some("binding".to_string()),
    }
}

#[test]
fn pkce_challenge_uses_s256_without_padding() {
    let verifier = random_url_secret(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    assert!(!challenge.contains('='));
    assert_ne!(challenge, verifier);
}

#[test]
fn microsoft_uses_the_exact_registered_loopback_redirect() {
    assert_eq!(
        loopback_redirect_uri(
            microsoft365::MANIFEST_ID,
            microsoft365::LOOPBACK_REDIRECT_PORT,
        ),
        "http://127.0.0.1:53683/oauth/callback"
    );
}

#[test]
fn slack_pkce_uses_the_exact_registered_localhost_redirect() {
    assert_eq!(
        loopback_redirect_uri("slack", 53_682),
        "http://localhost:53682/oauth/callback"
    );
}

#[test]
fn slack_identity_probe_uses_the_documented_post_method() {
    let credential = ConnectorCredential {
        manifest_id: "slack".to_string(),
        access_token: "xoxp-test-token".to_string(),
        bot_access_token: None,
        refresh_token: None,
        token_type: "user".to_string(),
        scopes: vec![],
        expires_at_ms: None,
        refresh_expires_at_ms: None,
        tenant_id: None,
        tenant_label: None,
        account_id: None,
        account_principal: None,
        identity_binding_hash: None,
    };
    let request = identity_probe_request(&reqwest::blocking::Client::new(), &credential)
        .unwrap()
        .build()
        .unwrap();

    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(request.url().as_str(), "https://slack.com/api/auth.test");
}

#[test]
fn slack_identity_maps_to_the_persisted_work_account_kind() {
    let identity = slack_identity_from_response(&serde_json::json!({
        "ok": true,
        "user": "Alex",
        "user_id": "U123",
        "team": "Eldris",
        "team_id": "T123"
    }))
    .unwrap();
    let metadata = identity.metadata.unwrap();

    assert_eq!(identity.label, "Alex");
    assert_eq!(identity.subject, "U123");
    assert_eq!(metadata.tenant_id, "T123");
    assert_eq!(metadata.tenant_label, "Eldris");
    assert_eq!(metadata.account_kind, "work");
}

#[test]
fn google_oauth_identity_uses_the_v2_userinfo_id() {
    let credential = ConnectorCredential {
        manifest_id: "google_workspace".to_string(),
        access_token: "access".to_string(),
        bot_access_token: None,
        refresh_token: None,
        token_type: "Bearer".to_string(),
        scopes: vec![
            "https://www.googleapis.com/auth/userinfo.email".to_string(),
            "https://www.googleapis.com/auth/userinfo.profile".to_string(),
        ],
        expires_at_ms: None,
        refresh_expires_at_ms: None,
        tenant_id: None,
        tenant_label: None,
        account_id: None,
        account_principal: None,
        identity_binding_hash: None,
    };
    let request = identity_probe_request(&reqwest::blocking::Client::new(), &credential)
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(request.method(), reqwest::Method::GET);
    assert_eq!(
        request.url().as_str(),
        "https://www.googleapis.com/oauth2/v2/userinfo"
    );

    let identity = google_identity_from_response(&serde_json::json!({
        "id": "stable-google-subject",
        "email": "person@example.com"
    }))
    .unwrap();
    assert_eq!(identity.subject, "stable-google-subject");
    assert_eq!(identity.label, "person@example.com");
    assert_eq!(
        google_identity_from_response(&serde_json::json!({
            "sub": "oidc-sub-is-not-the-oauth2-id-field",
            "email": "person@example.com"
        }))
        .err()
        .unwrap(),
        "connector_identity_probe_invalid"
    );
}

#[test]
fn google_native_consent_uses_complete_selected_capability_set() {
    let operations = vec!["gmail.read".to_string(), "calendar.read".to_string()];
    let scopes = manifest::google_requested_scopes(&operations).unwrap();
    let authorization = authorization::authorization_url(
        "google_workspace",
        "desktop-client",
        "http://127.0.0.1:49152/oauth/callback",
        "state",
        "challenge",
        "nonce",
        &scopes,
        &operations,
        true,
    )
    .unwrap();
    let parameters = authorization.query_pairs().collect::<Vec<_>>();
    assert!(!parameters
        .iter()
        .any(|(key, _)| key == "include_granted_scopes"));
    let requested = parameters
        .iter()
        .find(|(key, _)| key == "scope")
        .map(|(_, value)| value.split_whitespace().collect::<Vec<_>>())
        .unwrap();
    assert!(!requested
        .iter()
        .any(|scope| matches!(*scope, "openid" | "email" | "profile")));
    assert!(requested.contains(&"https://www.googleapis.com/auth/userinfo.email"));
    assert!(requested.contains(&"https://www.googleapis.com/auth/userinfo.profile"));
    assert!(scopes
        .iter()
        .all(|scope| requested.contains(&scope.as_str())));
}

#[test]
fn oauth_attempt_storage_accepts_only_approved_loopback_hosts() {
    let root = std::env::temp_dir().join(format!("oomu-oauth-loopback-{}", unix_time_ms_i64()));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let connector_id = repository::create_account(&engine, "slack", 1).unwrap();
    assert!(repository::record_oauth_attempt(
        &engine,
        "oauth_localhost",
        &connector_id,
        "state-hash",
        "http://localhost:53682/oauth/callback",
        unix_time_ms_i64() + 60_000,
    )
    .is_ok());
    assert!(repository::record_oauth_attempt(
        &engine,
        "oauth_external",
        &connector_id,
        "state-hash",
        "http://localhost.example:53682/oauth/callback",
        unix_time_ms_i64() + 60_000,
    )
    .is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn incremental_exchange_preserves_an_existing_refresh_token() {
    let previous = microsoft_credential(Some("long-lived-refresh"));
    let mut replacement = microsoft_credential(None);
    preserve_incremental_refresh_token(Some(&previous), &mut replacement);
    assert_eq!(
        replacement.refresh_token.as_deref(),
        Some("long-lived-refresh")
    );
}

#[test]
fn invalid_incremental_consent_creates_no_account_or_attempt() {
    let root = std::env::temp_dir().join(format!(
        "oomu-invalid-microsoft-consent-{}",
        unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let identity = SovereignIdentity::initialize_ephemeral();
    let error = begin(
        &engine,
        &identity,
        microsoft365::MANIFEST_ID,
        None,
        &["outlook.mail.send".to_string()],
    )
    .unwrap_err();
    assert_eq!(error, "microsoft_operation_unsupported");
    let connection = engine.open_connection().unwrap();
    let accounts: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM connector_accounts WHERE manifest_id='microsoft_365'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let attempts: i64 = connection
        .query_row("SELECT COUNT(*) FROM connector_oauth_attempts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(accounts, 0);
    assert_eq!(attempts, 0);
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn google_incremental_consent_preserves_existing_scopes() {
    let root = std::env::temp_dir().join(format!(
        "oomu-google-incremental-consent-{}",
        unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let version = manifest::manifest("google_workspace").unwrap().version;
    let connector_id = repository::create_account(&engine, "google_workspace", version).unwrap();
    engine.open_connection().unwrap().execute(
        "UPDATE connector_accounts
         SET connection_state='authorized',
             granted_scopes_json='[\"openid\",\"email\",\"profile\",\"https://www.googleapis.com/auth/gmail.readonly\"]'
         WHERE connector_id=?1",
        rusqlite::params![connector_id],
    ).unwrap();

    let (_, _, scopes, existing) = prepare_authorization(
        &engine,
        "google_workspace",
        Some(&connector_id),
        &["calendar.read".to_string()],
    )
    .unwrap();
    assert_eq!(existing.as_deref(), Some(connector_id.as_str()));
    assert!(scopes
        .iter()
        .any(|scope| scope.ends_with("/gmail.readonly")));
    assert!(scopes
        .iter()
        .any(|scope| scope.ends_with("/calendar.readonly")));
    assert!(scopes
        .iter()
        .any(|scope| scope == "https://www.googleapis.com/auth/userinfo.email"));
    assert!(scopes
        .iter()
        .any(|scope| scope == "https://www.googleapis.com/auth/userinfo.profile"));
    assert!(!scopes
        .iter()
        .any(|scope| matches!(scope.as_str(), "openid" | "email" | "profile")));
    assert!(!scopes
        .iter()
        .any(|scope| scope.ends_with("/drive.readonly")));

    let _ = std::fs::remove_dir_all(root);
}
