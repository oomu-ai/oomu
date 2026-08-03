use super::*;

#[test]
fn approved_pdf_decrypts_exact_verified_bytes_once() {
    let fixture = setup();
    let artifact = register_verified_artifact(&fixture);
    let command = signed_command(
        &fixture,
        "request_artifact",
        None,
        None,
        serde_json::json!({"artifactId":artifact.artifact_id,"format":"pdf"}),
    );
    let grant = artifact_transfer::prepare_grant_for_test(
        &fixture.engine,
        &fixture.identity,
        &command,
        "Test phone",
        true,
    )
    .unwrap();
    insert_prepared_grant(&fixture, &grant);
    let encrypted = artifact_transfer::retrieve(
        &fixture.engine,
        RetrieveRemoteArtifactRequest {
            remote_device_id: fixture.device.remote_device_id.clone(),
            token: grant.token.clone(),
        },
    )
    .unwrap();
    assert_eq!(encrypted.content_state, "full_content");
    assert_eq!(encrypted.source_sha256, grant.source_sha256);
    assert_eq!(encrypted.transfer_sha256, grant.transfer_sha256);
    let key_material = Sha256::digest(grant.token.as_bytes());
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_material));
    let nonce_bytes = STANDARD.decode(encrypted.nonce_base64).unwrap();
    let aad = STANDARD.decode(encrypted.associated_data_base64).unwrap();
    let ciphertext = STANDARD.decode(encrypted.ciphertext_base64).unwrap();
    let decrypted = cipher
        .decrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: ciphertext.as_slice(),
                aad: aad.as_slice(),
            },
        )
        .unwrap();
    assert_eq!(decrypted, artifact.pdf_bytes);
    assert!(artifact_transfer::retrieve(
        &fixture.engine,
        RetrieveRemoteArtifactRequest {
            remote_device_id: fixture.device.remote_device_id,
            token: grant.token,
        }
    )
    .is_err());
}

#[test]
fn docx_is_real_and_declared_full_content() {
    let fixture = setup();
    let artifact = register_verified_artifact(&fixture);
    let command = signed_command(
        &fixture,
        "request_artifact",
        None,
        None,
        serde_json::json!({"artifactId":artifact.artifact_id,"format":"docx"}),
    );
    let grant = artifact_transfer::prepare_grant_for_test(
        &fixture.engine,
        &fixture.identity,
        &command,
        "Test phone",
        true,
    )
    .unwrap();
    assert!(artifact.docx_bytes.starts_with(b"PK"));
    assert_eq!(
        grant.source_path,
        artifact.docx_path.to_string_lossy().to_string()
    );
    assert_eq!(grant.content_state, "full_content");
    assert_eq!(grant.source_sha256, grant.transfer_sha256);
}

#[test]
fn denial_protected_changed_and_wrong_device_fail_closed() {
    let fixture = setup();
    let artifact = register_verified_artifact(&fixture);
    let command = signed_command(
        &fixture,
        "request_artifact",
        None,
        None,
        serde_json::json!({"artifactId":artifact.artifact_id,"format":"pdf"}),
    );
    assert!(artifact_transfer::prepare_grant_for_test(
        &fixture.engine,
        &fixture.identity,
        &command,
        "Test phone",
        false,
    )
    .unwrap_err()
    .starts_with("remote_artifact_user_denied"));
    let count: i64 = fixture
        .engine
        .open_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM remote_artifact_grants", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);

    fixture
        .engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE artifact_versions SET provenance_json=?2 WHERE artifact_id=?1",
            params![
                artifact.artifact_id,
                serde_json::json!({"protected": true}).to_string()
            ],
        )
        .unwrap();
    assert!(artifact_transfer::prepare_grant_for_test(
        &fixture.engine,
        &fixture.identity,
        &command,
        "Test phone",
        true,
    )
    .unwrap_err()
    .starts_with("remote_artifact_protected"));
    fixture
        .engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE artifact_versions SET provenance_json='{}' WHERE artifact_id=?1",
            params![artifact.artifact_id],
        )
        .unwrap();

    let grant = artifact_transfer::prepare_grant_for_test(
        &fixture.engine,
        &fixture.identity,
        &command,
        "Test phone",
        true,
    )
    .unwrap();
    insert_prepared_grant(&fixture, &grant);
    assert!(artifact_transfer::retrieve(
        &fixture.engine,
        RetrieveRemoteArtifactRequest {
            remote_device_id: "device_00000000-0000-4000-8000-000000000000".into(),
            token: grant.token.clone(),
        }
    )
    .is_err());
    let relabel = fixture.engine.open_connection().unwrap().execute(
        "UPDATE remote_artifact_grants SET content_state='legacy_unverified' WHERE grant_id=?1",
        params![grant.grant_id],
    );
    assert!(relabel
        .unwrap_err()
        .to_string()
        .contains("remote_artifact_grant_immutable"));

    let changed = artifact_transfer::prepare_grant_for_test(
        &fixture.engine,
        &fixture.identity,
        &command,
        "Test phone",
        true,
    )
    .unwrap();
    insert_prepared_grant(&fixture, &changed);
    std::fs::write(&artifact.pdf_path, b"changed after approval").unwrap();
    assert!(artifact_transfer::retrieve(
        &fixture.engine,
        RetrieveRemoteArtifactRequest {
            remote_device_id: fixture.device.remote_device_id,
            token: changed.token,
        }
    )
    .unwrap_err()
    .starts_with("remote_artifact_source_changed"));
}

#[test]
fn shipping_migration_quarantines_legacy_claims() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch(include_str!(
            "../../migrations/0021_secure_remote_dispatch.sql"
        ))
        .unwrap();
    connection.execute(
        "INSERT INTO remote_devices (remote_device_id,label,public_key,allowed_project_ids_json,scopes_json,paired_at_ms,expires_at_ms) VALUES ('device_legacy','Old phone','old-key','[]','[]',1,9999999999999)",
        [],
    ).unwrap();
    connection.execute(
        "INSERT INTO remote_artifact_grants (grant_id,token_hash,remote_device_id,project_id,artifact_id,artifact_format,private_path,artifact_sha256,redaction_state,protected,expires_at_ms,created_at_ms) VALUES ('grant_legacy','token-hash','device_legacy','project_legacy','artifact_legacy','pdf','/private/legacy.pdf','legacy-digest','redacted_by_default',0,9999999999999,1)",
        [],
    ).unwrap();
    connection
        .execute_batch(include_str!(
            "../../migrations/0027_remote_receipt_atomicity.sql"
        ))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../../migrations/0028_remote_artifact_truth.sql"
        ))
        .unwrap();
    let state: String = connection
        .query_row(
            "SELECT content_state FROM remote_artifact_grants WHERE grant_id='grant_legacy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "legacy_unverified");
    let retrievable: i64 = connection.query_row(
        "SELECT COUNT(*) FROM remote_artifact_grants WHERE grant_id='grant_legacy' AND content_state IN ('full_content','verified_redacted_derivative')",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(retrievable, 0);
}
