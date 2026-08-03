use super::*;
use crate::{
    artifacts::{
        ArtifactBlock, ArtifactDocument, ArtifactMetadata, ArtifactSection, PageControls,
        ParagraphStyle, ThemeTokens, ARTIFACT_BUILDER_IDENTITY, ARTIFACT_DOCUMENT_SCHEMA_VERSION,
    },
    db::PersistenceEngine,
    projects::{CreateProjectRequest, ProjectDataPolicy},
    sovereign_identity::SovereignIdentity,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20poly1305::{aead::Aead, aead::Payload, ChaCha20Poly1305, Key, KeyInit, Nonce};
use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use rusqlite::{params, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};

struct Fixture {
    root: PathBuf,
    engine: PersistenceEngine,
    identity: SovereignIdentity,
    project_id: String,
    task_run_id: String,
    device: RemoteDeviceRecord,
    device_key: SigningKey,
}

fn setup() -> Fixture {
    let mut random = [0_u8; 8];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut random);
    let root = std::env::temp_dir().join(format!(
        "oomu-remote-test-{}-{}",
        crate::foundation::clock::unix_time_ms_i64(),
        hex::encode(random)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: "Remote".into(),
            description: String::new(),
            data_policy: ProjectDataPolicy::AskBeforeCloud,
        },
    )
    .unwrap();
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    let task_id = crate::p0_contracts::TaskId::new().to_string();
    let flow_id = format!("flow-remote-{}", hex::encode(random));
    let now = crate::foundation::clock::unix_time_ms_i64();
    let connection = engine.open_connection().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS taskflows (flow_id TEXT PRIMARY KEY,parent_session_id TEXT NOT NULL,directive TEXT NOT NULL,status TEXT NOT NULL,created_at_ms INTEGER NOT NULL,updated_at_ms INTEGER NOT NULL);\
             CREATE TABLE IF NOT EXISTS taskflow_steps (flow_id TEXT NOT NULL,status TEXT NOT NULL);",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO taskflows (flow_id,parent_session_id,directive,status,created_at_ms,updated_at_ms) VALUES (?1,'remote-test-session','Remote test task','active',?2,?2)",
            params![flow_id, now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,'taskflow',?4,'running','remote_test',?5,'Remote test task',?6,?6)",
            params![task_run_id, task_id, project.project_id, flow_id, format!("correlation-{task_run_id}"), now],
        )
        .unwrap();

    let device_key = SigningKey::generate(&mut OsRng);
    let challenge = repository::create_challenge(
        &engine,
        CreatePairingChallengeRequest {
            allowed_project_ids: vec![project.project_id.clone()],
            scopes: vec![
                "view_task".into(),
                "stop_task".into(),
                "request_artifact".into(),
            ],
        },
    )
    .unwrap();
    let qr_payload: String = connection
        .query_row(
            "SELECT qr_payload FROM remote_pairing_challenges WHERE challenge_id=?1",
            params![challenge.challenge_id],
            |row| row.get(0),
        )
        .unwrap();
    let secret = qr_payload.split("secret=").nth(1).unwrap();
    repository::submit_response(
        &engine,
        SubmitPairingResponseRequest {
            challenge_id: challenge.challenge_id.clone(),
            secret: secret.into(),
            device_label: "Test phone".into(),
            public_key: hex::encode(device_key.verifying_key().to_bytes()),
        },
    )
    .unwrap();
    let device = repository::confirm(
        &engine,
        ConfirmPairingRequest {
            challenge_id: challenge.challenge_id,
            allow: true,
        },
    )
    .unwrap()
    .unwrap();
    Fixture {
        root,
        engine,
        identity: SovereignIdentity::initialize_ephemeral(),
        project_id: project.project_id,
        task_run_id,
        device,
        device_key,
    }
}

fn signed_command(
    fixture: &Fixture,
    kind: &str,
    task_run_id: Option<String>,
    expected_task_sequence: Option<u64>,
    payload: serde_json::Value,
) -> SignedRemoteCommand {
    let payload_sha256 = crate::foundation::digest::sha256_hex(
        &serde_json::to_vec(&payload).expect("payload serializes"),
    );
    let mut command = SignedRemoteCommand {
        command_id: crypto::random_hex(16),
        remote_device_id: fixture.device.remote_device_id.clone(),
        project_id: fixture.project_id.clone(),
        task_run_id,
        command_kind: kind.to_string(),
        nonce: crypto::random_hex(32),
        expires_at_ms: crate::foundation::clock::unix_time_ms_i64() + 30_000,
        expected_task_sequence,
        payload_sha256,
        signer_public_key: hex::encode(fixture.device_key.verifying_key().to_bytes()),
        payload,
        signature: String::new(),
    };
    command.signature = hex::encode(
        fixture
            .device_key
            .sign(command_store::canonical(&command).as_bytes())
            .to_bytes(),
    );
    command
}

fn resign(fixture: &Fixture, command: &mut SignedRemoteCommand) {
    command.signature = hex::encode(
        fixture
            .device_key
            .sign(command_store::canonical(command).as_bytes())
            .to_bytes(),
    );
}

struct ArtifactFixture {
    artifact_id: String,
    pdf_path: PathBuf,
    docx_path: PathBuf,
    pdf_bytes: Vec<u8>,
    docx_bytes: Vec<u8>,
}

fn artifact_document() -> ArtifactDocument {
    ArtifactDocument {
        schema_version: ARTIFACT_DOCUMENT_SCHEMA_VERSION,
        metadata: ArtifactMetadata {
            title: "Remote test file".into(),
            subtitle: String::new(),
            author: "OOMU".into(),
            subject: "Remote transfer verification".into(),
            keywords: vec!["remote".into(), "verified".into()],
            language: "en-US".into(),
        },
        theme: ThemeTokens::default(),
        page: PageControls::default(),
        header: None,
        footer: None,
        sections: vec![ArtifactSection {
            heading: "Verified content".into(),
            page_break_before: false,
            blocks: vec![ArtifactBlock::Paragraph {
                text: "This file was built and verified locally.".into(),
                style: ParagraphStyle::Body,
                factual: false,
                sources: Vec::new(),
            }],
        }],
    }
}

fn register_verified_artifact(fixture: &Fixture) -> ArtifactFixture {
    let artifact_id = crate::p0_contracts::ArtifactId::new().to_string();
    let pdf_path = fixture.root.join(format!("{artifact_id}.pdf"));
    let docx_path = fixture.root.join(format!("{artifact_id}.docx"));
    let document = artifact_document();
    crate::artifacts::helper::write_pdf(&document, &pdf_path).unwrap();
    crate::artifacts::helper::write_docx(&document, &docx_path).unwrap();
    let pdf_bytes = std::fs::read(&pdf_path).unwrap();
    let docx_bytes = std::fs::read(&docx_path).unwrap();
    assert!(pdf_bytes.starts_with(b"%PDF"));
    assert!(docx_bytes.starts_with(b"PK"));
    let now = crate::foundation::clock::unix_time_ms_i64();
    let connection = fixture.engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO artifact_records (artifact_id,project_id,task_run_id,title,current_version,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,'Remote test file',1,?4,?4)",
            params![artifact_id, fixture.project_id, fixture.task_run_id, now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO artifact_versions (artifact_id,version,document_json,status,docx_private_path,pdf_private_path,verification_json,provenance_json,docx_sha256,pdf_sha256,docx_bytes,pdf_bytes,builder_identity,created_at_ms,completed_at_ms) VALUES (?1,1,?2,'verified',?3,?4,'{}','{}',?5,?6,?7,?8,?9,?10,?10)",
            params![
                artifact_id,
                serde_json::to_string(&document).unwrap(),
                docx_path.to_string_lossy(),
                pdf_path.to_string_lossy(),
                crate::foundation::digest::sha256_hex(&docx_bytes),
                crate::foundation::digest::sha256_hex(&pdf_bytes),
                docx_bytes.len() as i64,
                pdf_bytes.len() as i64,
                ARTIFACT_BUILDER_IDENTITY,
                now,
            ],
        )
        .unwrap();
    ArtifactFixture {
        artifact_id,
        pdf_path,
        docx_path,
        pdf_bytes,
        docx_bytes,
    }
}

fn insert_prepared_grant(fixture: &Fixture, grant: &artifact_transfer::PreparedArtifactGrant) {
    let mut connection = fixture.engine.open_connection().unwrap();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    artifact_transfer::insert_prepared(&transaction, grant).unwrap();
    transaction.commit().unwrap();
}

#[test]
fn pairing_code_is_live_expiring_and_one_use() {
    let fixture = setup();
    let pairing = repository::create_challenge(
        &fixture.engine,
        CreatePairingChallengeRequest {
            allowed_project_ids: vec![fixture.project_id],
            scopes: vec!["view_task".into()],
        },
    )
    .unwrap();
    assert!(pairing.qr_svg.contains("<svg"));
    assert!(pairing.expires_at_ms > crate::foundation::clock::unix_time_ms_i64());
    assert!(repository::confirm(
        &fixture.engine,
        ConfirmPairingRequest {
            challenge_id: pairing.challenge_id,
            allow: true,
        }
    )
    .is_err());
}

#[test]
fn remote_command_store_accepts_all_mvp_commands_and_decodes_every_field() {
    let fixture = setup();
    let artifact = register_verified_artifact(&fixture);
    let commands = [
        signed_command(
            &fixture,
            "view_task",
            Some(fixture.task_run_id.clone()),
            None,
            serde_json::json!({}),
        ),
        signed_command(
            &fixture,
            "stop_task",
            Some(fixture.task_run_id.clone()),
            Some(0),
            serde_json::json!({}),
        ),
        signed_command(
            &fixture,
            "request_artifact",
            None,
            None,
            serde_json::json!({"artifactId":artifact.artifact_id,"format":"pdf"}),
        ),
    ];
    for command in commands {
        let accepted = command_store::accept(&fixture.engine, &command).unwrap();
        let command_store::CommandAcceptance::Accepted(stored) = accepted else {
            panic!("valid command was not accepted")
        };
        assert_eq!(stored.signer_public_key, command.signer_public_key);
        assert_eq!(stored.payload_sha256, command.payload_sha256);
        assert_eq!(stored.status, "accepted");
        let reopened = fixture.engine.open_connection().unwrap();
        assert_eq!(
            command_store::load(&reopened, &command.command_id)
                .unwrap()
                .unwrap(),
            stored
        );
    }
}

#[test]
fn remote_command_store_returns_stable_rejection_codes() {
    let fixture = setup();
    let base = signed_command(
        &fixture,
        "view_task",
        Some(fixture.task_run_id.clone()),
        None,
        serde_json::json!({"view":"summary"}),
    );
    command_store::accept(&fixture.engine, &base).unwrap();
    assert_eq!(
        command_store::accept(&fixture.engine, &base)
            .unwrap_err()
            .code,
        "remote_command_duplicate_id"
    );

    let mut replay = base.clone();
    replay.command_id = crypto::random_hex(16);
    resign(&fixture, &mut replay);
    assert_eq!(
        command_store::accept(&fixture.engine, &replay)
            .unwrap_err()
            .code,
        "remote_command_replayed_nonce"
    );

    let mut changed = signed_command(
        &fixture,
        "view_task",
        Some(fixture.task_run_id.clone()),
        None,
        serde_json::json!({"view":"summary"}),
    );
    changed.payload = serde_json::json!({"view":"changed"});
    assert_eq!(
        command_store::accept(&fixture.engine, &changed)
            .unwrap_err()
            .code,
        "remote_command_payload_digest_mismatch"
    );

    let mut wrong_key = signed_command(
        &fixture,
        "view_task",
        Some(fixture.task_run_id.clone()),
        None,
        serde_json::json!({}),
    );
    let other_key = SigningKey::generate(&mut OsRng);
    wrong_key.signer_public_key = hex::encode(other_key.verifying_key().to_bytes());
    wrong_key.signature = hex::encode(
        other_key
            .sign(command_store::canonical(&wrong_key).as_bytes())
            .to_bytes(),
    );
    assert_eq!(
        command_store::accept(&fixture.engine, &wrong_key)
            .unwrap_err()
            .code,
        "remote_command_signer_key_mismatch"
    );

    let mut bad_signature = signed_command(
        &fixture,
        "view_task",
        Some(fixture.task_run_id.clone()),
        None,
        serde_json::json!({}),
    );
    bad_signature.signature = "00".repeat(64);
    assert_eq!(
        command_store::accept(&fixture.engine, &bad_signature)
            .unwrap_err()
            .code,
        "remote_command_signature_mismatch"
    );

    let mut expired = signed_command(
        &fixture,
        "view_task",
        Some(fixture.task_run_id.clone()),
        None,
        serde_json::json!({}),
    );
    expired.expires_at_ms = crate::foundation::clock::unix_time_ms_i64() - 1;
    resign(&fixture, &mut expired);
    assert_eq!(
        command_store::accept(&fixture.engine, &expired)
            .unwrap_err()
            .code,
        "remote_command_expired"
    );

    let other_project = crate::projects::repository::create(
        &fixture.engine,
        CreateProjectRequest {
            name: "Outside".into(),
            description: String::new(),
            data_policy: ProjectDataPolicy::AskBeforeCloud,
        },
    )
    .unwrap();
    let mut wrong_project = signed_command(
        &fixture,
        "view_task",
        Some(fixture.task_run_id.clone()),
        None,
        serde_json::json!({}),
    );
    wrong_project.project_id = other_project.project_id;
    resign(&fixture, &mut wrong_project);
    assert_eq!(
        command_store::accept(&fixture.engine, &wrong_project)
            .unwrap_err()
            .code,
        "remote_command_project_scope_mismatch"
    );
}

#[test]
fn remote_command_store_sequence_conflict_is_persisted_honestly() {
    let fixture = setup();
    let command = signed_command(
        &fixture,
        "stop_task",
        Some(fixture.task_run_id.clone()),
        Some(8),
        serde_json::json!({}),
    );
    let command_store::CommandAcceptance::SequenceConflict { command, .. } =
        command_store::accept(&fixture.engine, &command).unwrap()
    else {
        panic!("stale command was not rejected")
    };
    assert_eq!(command.status, "accepted");
    assert!(command.outcome_code.is_none());
}

#[test]
fn remote_command_store_schema_contract_matches_shipping_migrations() {
    let fixture = setup();
    let actual =
        command_store::schema_contract(&fixture.engine.open_connection().unwrap()).unwrap();
    let expected = command_store::REMOTE_COMMAND_SCHEMA
        .iter()
        .map(|(name, required)| (name.to_string(), *required))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn remote_execution_commit_view_task_persists_one_signed_receipt() {
    let fixture = setup();
    let command = signed_command(
        &fixture,
        "view_task",
        Some(fixture.task_run_id.clone()),
        None,
        serde_json::json!({}),
    );
    let result = repository::execute(&fixture.engine, &fixture.identity, command).unwrap();
    assert_eq!(result.status, "completed");
    assert_eq!(result.outcome_code, "applied");
    assert_eq!(result.task.unwrap().task_run_id, fixture.task_run_id);
    let connection = fixture.engine.open_connection().unwrap();
    let receipt: (i64, String) = connection
        .query_row(
            "SELECT COUNT(*),MIN(signer_public_key) FROM remote_audit_receipts WHERE command_id=?1 AND receipt_kind='remote_command'",
            params![result.command_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(receipt.0, 1);
    assert!(!receipt.1.is_empty());
    let mutation = connection.execute(
        "UPDATE remote_audit_receipts SET signature='tampered' WHERE command_id=?1 AND receipt_kind='remote_command'",
        params![result.command_id],
    );
    assert!(mutation
        .unwrap_err()
        .to_string()
        .contains("remote_command_receipt_immutable"));
}

#[test]
fn remote_execution_commit_sequence_conflict_finalizes_with_one_receipt() {
    let fixture = setup();
    let command = signed_command(
        &fixture,
        "stop_task",
        Some(fixture.task_run_id.clone()),
        Some(9),
        serde_json::json!({}),
    );
    let result = repository::execute(&fixture.engine, &fixture.identity, command).unwrap();
    assert_eq!(result.status, "rejected");
    assert_eq!(result.outcome_code, "remote_command_sequence_conflict");
    let persisted: (String, i64) = fixture
        .engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT c.status,(SELECT COUNT(*) FROM remote_audit_receipts r WHERE r.command_id=c.command_id AND r.receipt_kind='remote_command') FROM remote_commands c WHERE c.command_id=?1",
            params![result.command_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(persisted, ("rejected".to_string(), 1));
}

fn accepted_stop(fixture: &Fixture) -> (SignedRemoteCommand, command_store::StoredRemoteCommand) {
    let command = signed_command(
        fixture,
        "stop_task",
        Some(fixture.task_run_id.clone()),
        Some(0),
        serde_json::json!({}),
    );
    let command_store::CommandAcceptance::Accepted(stored) =
        command_store::accept(&fixture.engine, &command).unwrap()
    else {
        panic!("stop command was not accepted")
    };
    (command, stored)
}

fn assert_stop_rolled_back(fixture: &Fixture, command_id: &str) {
    let connection = fixture.engine.open_connection().unwrap();
    let states: (String, String, String) = connection
        .query_row(
            "SELECT r.status,t.state,c.status FROM remote_commands r JOIN task_runs t ON t.task_run_id=r.task_run_id JOIN taskflows c ON c.flow_id=t.runtime_record_id WHERE r.command_id=?1",
            params![command_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        states,
        ("accepted".into(), "running".into(), "active".into())
    );
    let receipts: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM remote_audit_receipts WHERE command_id=?1",
            params![command_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 0);
}

#[test]
fn remote_execution_commit_receipt_failure_rolls_back_and_retry_applies_once() {
    let fixture = setup();
    let (signed, stored) = accepted_stop(&fixture);
    fixture.engine.open_connection().unwrap().execute_batch(
        "CREATE TRIGGER remote_test_fail_receipt BEFORE INSERT ON remote_audit_receipts WHEN NEW.receipt_kind='remote_command' BEGIN SELECT RAISE(ABORT,'forced_receipt_failure'); END;",
    ).unwrap();
    assert!(execution_commit::execute_accepted(
        &fixture.engine,
        &fixture.identity,
        &signed,
        &stored
    )
    .is_err());
    assert_stop_rolled_back(&fixture, &stored.command_id);
    fixture
        .engine
        .open_connection()
        .unwrap()
        .execute_batch("DROP TRIGGER remote_test_fail_receipt;")
        .unwrap();
    let result =
        execution_commit::execute_accepted(&fixture.engine, &fixture.identity, &signed, &stored)
            .unwrap();
    assert_eq!(result.status, "completed");
    let connection = fixture.engine.open_connection().unwrap();
    let counts: (i64, i64) = connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM task_events WHERE task_run_id=?1),(SELECT COUNT(*) FROM remote_audit_receipts WHERE command_id=?2 AND receipt_kind='remote_command')",
            params![fixture.task_run_id, stored.command_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(counts, (1, 1));
}

#[test]
fn remote_execution_commit_finalization_and_device_failures_roll_back() {
    for trigger in [
        "CREATE TRIGGER remote_test_fail_finalize BEFORE UPDATE ON remote_commands WHEN OLD.status='accepted' BEGIN SELECT RAISE(ABORT,'forced_finalize_failure'); END;",
        "CREATE TRIGGER remote_test_fail_device BEFORE UPDATE ON remote_devices BEGIN SELECT RAISE(ABORT,'forced_device_failure'); END;",
    ] {
        let fixture = setup();
        let (signed, stored) = accepted_stop(&fixture);
        fixture
            .engine
            .open_connection()
            .unwrap()
            .execute_batch(trigger)
            .unwrap();
        assert!(execution_commit::execute_accepted(
            &fixture.engine,
            &fixture.identity,
            &signed,
            &stored
        )
        .is_err());
        assert_stop_rolled_back(&fixture, &stored.command_id);
    }
}

#[test]
fn remote_execution_commit_concurrent_stop_commits_exactly_once() {
    let fixture = setup();
    let (signed, stored) = accepted_stop(&fixture);
    let engine = fixture.engine.clone();
    let identity = fixture.identity.clone();
    let signed = Arc::new(signed);
    let stored = Arc::new(stored);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let engine = engine.clone();
        let identity = identity.clone();
        let signed = signed.clone();
        let stored = stored.clone();
        let barrier = barrier.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            execution_commit::execute_accepted(&engine, &identity, &signed, &stored)
        }));
    }
    barrier.wait();
    for handle in handles {
        assert_eq!(handle.join().unwrap().unwrap().status, "completed");
    }
    let connection = fixture.engine.open_connection().unwrap();
    let counts: (i64, i64) = connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM task_events WHERE task_run_id=?1),(SELECT COUNT(*) FROM remote_audit_receipts WHERE command_id=?2 AND receipt_kind='remote_command')",
            params![fixture.task_run_id, stored.command_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(counts, (1, 1));
}

#[path = "artifact_transfer_tests.rs"]
mod artifact_transfer_tests;
