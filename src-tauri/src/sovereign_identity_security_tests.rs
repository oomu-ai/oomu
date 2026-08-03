use super::APP_DATA_ENV_LOCK;
use super::*;

#[test]
fn manual_session_passphrase_derives_key_without_plaintext_file() {
    let _env_guard = APP_DATA_ENV_LOCK.lock().unwrap();
    let root = std::env::temp_dir().join(format!(
        "oomu_identity_session_{}_{}",
        std::process::id(),
        unix_time_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let previous_app_root = std::env::var_os(crate::settings::APP_DATA_ROOT_ENV);
    std::env::set_var(crate::settings::APP_DATA_ROOT_ENV, &root);

    let identity =
        SovereignIdentity::initialize_with_session_passphrase("correct horse battery 148")
            .expect("session passphrase initializes");
    let profile = identity.profile().expect("profile resolves from memory");
    let (service, account) = keychain_location();
    let root_reads_before = crate::keychain_session::backend_read_count_for_test(service, account);
    let node_profile = identity
        .node_identity()
        .expect("session node identity resolves from memory");
    let (receipt_node_profile, node_signature) = identity
        .sign_node_payload_with_profile("session-scoped tool receipt")
        .expect("session node and receipt resolve atomically without Keychain access");

    assert_eq!(
        profile.storage_backend,
        "manual session passphrase (memory only)"
    );
    assert_ne!(profile.public_key, node_profile.public_key);
    assert_eq!(receipt_node_profile.node_id, node_profile.node_id);
    assert_eq!(node_signature.public_key, node_profile.public_key);
    assert_eq!(
        node_profile.private_key_path,
        "memory:manual-session-passphrase-node-key"
    );
    assert_eq!(
        crate::keychain_session::backend_read_count_for_test(service, account),
        root_reads_before
    );
    assert!(!root.join("genesis.key").exists());
    assert!(!root
        .join(OOMU_IDENTITY_DIR)
        .join(NODE_IDENTITY_FILE)
        .exists());

    let shared_identity = identity.clone();
    let rekeyed = shared_identity
        .activate_manual_session_passphrase("correct horse battery 249")
        .expect("a shared clone rekeys one atomic session generation");
    let (rekeyed_node, rekeyed_signature) = identity
        .sign_node_payload_with_profile("receipt after rekey")
        .expect("every clone observes the complete rekeyed generation");
    assert_ne!(profile.public_key, rekeyed.public_key);
    assert_ne!(node_profile.public_key, rekeyed_node.public_key);
    assert_eq!(rekeyed_signature.public_key, rekeyed_node.public_key);
    assert_eq!(
        rekeyed_node.architect_signature.public_key,
        rekeyed.public_key
    );
    identity
        .verify_architect_signature(
            &node_identity_payload(
                &rekeyed_node.node_id,
                &rekeyed_node.public_key,
                rekeyed_node.created_at_ms,
            ),
            &rekeyed_node.architect_signature,
        )
        .expect("the rekeyed node proof is bound to the same root generation");
    if let Some(previous_app_root) = previous_app_root {
        std::env::set_var(crate::settings::APP_DATA_ROOT_ENV, previous_app_root);
    } else {
        std::env::remove_var(crate::settings::APP_DATA_ROOT_ENV);
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn weak_manual_session_passphrase_is_rejected() {
    let error = match SovereignIdentity::initialize_with_session_passphrase("short") {
        Ok(_) => panic!("weak passphrase must fail"),
        Err(error) => error,
    };
    assert_eq!(error.code, "identity_invalid_crypto_material");
}

#[test]
fn secure_storage_profile_reconciliation_quarantines_the_previous_key() {
    let root = std::env::temp_dir().join(format!(
        "oomu_identity_reconciliation_{}_{}",
        std::process::id(),
        unix_time_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let profile_path = root.join(IDENTITY_PROFILE);
    let previous_key = SigningKey::generate(&mut OsRng);
    let active_key = SigningKey::generate(&mut OsRng);
    let previous_public_key = hex::encode(previous_key.verifying_key().to_bytes());
    let active_public_key = hex::encode(active_key.verifying_key().to_bytes());
    std::fs::write(
        &profile_path,
        serde_json::json!({
            "public_key": previous_public_key,
            "fingerprint": fingerprint(&previous_public_key),
            "hardware_binding": hardware_binding(),
            "storage_backend": "local file (fallback)",
            "genesis_created_at_ms": 42
        })
        .to_string(),
    )
    .unwrap();
    let identity = SovereignIdentity::new_without_test_override(profile_path.clone());
    let previous_signature = signature_block_from_signing_key(&previous_key, "before rotation");
    let active_signature = signature_block_from_signing_key(&active_key, "after rotation");

    identity
        .reconcile_profile_with_signing_key(&active_key)
        .expect("secure storage key reconciles the public profile");

    let previous_error = identity
        .verify_payload("before rotation", &previous_signature)
        .expect_err("a quarantined predecessor cannot authorize current operations");
    assert_eq!(previous_error.code, "ledger_integrity_violation");
    identity
        .verify_payload("after rotation", &active_signature)
        .expect("the active secure-storage key verifies");
    let profile: PersistedIdentityProfile =
        serde_json::from_str(&std::fs::read_to_string(&profile_path).unwrap()).unwrap();
    assert_eq!(profile.profile_version, IDENTITY_PROFILE_VERSION);
    assert_eq!(profile.public_key, active_public_key);
    assert_eq!(profile.genesis_created_at_ms, 42);
    assert_eq!(
        profile.quarantined_predecessor_public_keys,
        vec![previous_public_key]
    );
    assert_eq!(profile.key_rotations.len(), 1);
    let recovery_profiles = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("sovereign_identity.recovery.")
        })
        .count();
    assert_eq!(recovery_profiles, 1);

    let unknown_key = SigningKey::generate(&mut OsRng);
    let unknown_signature = signature_block_from_signing_key(&unknown_key, "unknown signer");
    let error = identity
        .verify_payload("unknown signer", &unknown_signature)
        .expect_err("an unknown signer must remain rejected");
    assert_eq!(error.code, "ledger_integrity_violation");

    let _ = std::fs::remove_dir_all(root);
}
