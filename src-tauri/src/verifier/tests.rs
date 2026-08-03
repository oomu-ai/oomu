use super::*;

fn semantic_claim(operation: &str, score: f64, reasoning: &str) -> String {
    let reasoning_b64 = BASE64_STANDARD.encode(reasoning.as_bytes());
    let reasoning_hash = sha256_hex(reasoning.as_bytes());
    format!(
            "operation={operation} status=completed semantic_pass=true relevance_score={score:.4} reasoning_b64={reasoning_b64} reasoning_hash={reasoning_hash}"
        )
}

fn signed_local_certificate_claim(identity: &SovereignIdentity, output_sha256: &str) -> String {
    let mut certificate = LogicalCertificate::unsigned(
        vec![format!("output_sha256={output_sha256}")],
        vec!["Bind certificate to the provided output JSON.".to_string()],
        "The operation is certified with the prescribed output hash.".to_string(),
    );
    certificate.signature = Some(
        identity
            .sign_certificate_parts(
                &certificate.premises,
                &certificate.execution_path,
                &certificate.formal_conclusion,
            )
            .expect("certificate signs"),
    );
    let certificate_json = serde_json::to_string(&certificate).expect("certificate serializes");
    let certificate_hash = sha256_hex(certificate_json.as_bytes());

    format!(
        "local_certificate_hash={} output_sha256={} local_certificate_b64={}",
        certificate_hash,
        output_sha256,
        BASE64_STANDARD.encode(certificate_json.as_bytes())
    )
}

fn signed_external_write_plan() -> (ActionPlan, SovereignIdentity) {
    let identity = SovereignIdentity::initialize().expect("identity initializes");
    let id = "plan-approved-external-write".to_string();
    let objective = "Write the approved briefing file.".to_string();
    let step = crate::agentic_loop::Step {
        step: "Write approved external briefing.".to_string(),
        tool: crate::agentic_loop::Tool::FileWrite {
            path: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("Downloads")
                .join("oomu-approved-plan-verifier.md")
                .display()
                .to_string(),
            content: "approved content".to_string(),
        },
        risk_level: crate::agentic_loop::RiskLevel::High,
    };
    let exit_condition = "Exit after the approved write completes.".to_string();
    let mut certificate = LogicalCertificate::unsigned(
        vec![format!("objective={objective}"), format!("plan_id={id}")],
        vec![format!(
            "1. step={} tool={} risk={:?}",
            step.step,
            step.tool.authorization_kind(),
            step.risk_level
        )],
        exit_condition.clone(),
    );
    certificate.signature = Some(
        identity
            .sign_certificate_parts(
                &certificate.premises,
                &certificate.execution_path,
                &certificate.formal_conclusion,
            )
            .expect("plan certificate signs"),
    );

    (
        ActionPlan {
            id,
            objective: objective.clone(),
            intent: crate::gemma::StructuredIntent {
                objective,
                category: crate::gemma::IntentCategory::ProjectAnalysis,
                source: crate::gemma::IntentSource::Degraded,
                degraded_reason: Some("test fixture".to_string()),
            },
            steps: vec![step],
            exit_condition,
            logical_certificate: certificate,
            trusted_automatic_execution: false,
            model_route: crate::agentic_loop::ModelRouteDecision {
                selected_model: crate::shield_gate::ModelMetadata::local_gemma(),
                provider_config_id: None,
                provider_id: Some("local_model".to_string()),
                recommended_model: None,
                requires_principal_authorization: false,
                reason: "test route".to_string(),
                context_excerpt_count: 0,
                context_sources: Vec::new(),
            },
            parent_artifact_hashes: Vec::new(),
        },
        identity,
    )
}

fn unique_temp_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{}-{}-{}",
        label,
        std::process::id(),
        unix_time_ms()
    ))
}

#[test]
fn plan_preview_and_approved_execution_allow_external_file_write() {
    let (plan, identity) = signed_external_write_plan();
    let verifier = MlcVerifier::new();

    let unapproved = verifier
        .verify_plan(&plan, &identity)
        .expect_err("unapproved external write still rejects");
    assert!(unapproved
        .message
        .contains("file_write rejected path outside project quarantine"));

    let preview = verifier
        .verify_plan_preview(&plan, &identity)
        .expect("preview structurally accepts approval-required external write");
    assert!(matches!(
        preview.authorized_actions.first(),
        Some(AuthorizedActions::ApprovedExternalFileWrite(_))
    ));

    let approved = verifier
        .verify_approved_plan(&plan, &identity)
        .expect("approved plan authorizes external write");
    assert!(matches!(
        approved.authorized_actions.first(),
        Some(AuthorizedActions::ApprovedExternalFileWrite(_))
    ));

    let resumed = verifier
        .verify_approved_plan_from_step(&plan, &identity, 1)
        .expect("a fully checkpointed plan needs no new action authorization");
    assert!(resumed.authorized_actions.is_empty());
}

#[test]
fn legacy_logical_certificate_cannot_authorize_new_plan_execution() {
    let (mut plan, identity) = signed_external_write_plan();
    let certificate = &plan.logical_certificate;
    let legacy_payload = serde_json::json!({
        "premises": certificate.premises.clone(),
        "execution_path": certificate.execution_path.clone(),
        "formal_conclusion": certificate.formal_conclusion.clone(),
    })
    .to_string();
    plan.logical_certificate.signature = Some(
        identity
            .sign_exact_payload(&legacy_payload)
            .expect("legacy historical certificate fixture signs"),
    );

    let error = MlcVerifier::new()
        .verify_approved_plan(&plan, &identity)
        .expect_err("legacy certificate must not authorize new plan work");
    assert!(error
        .message
        .contains("payload hash does not match signature block"));
}

#[test]
fn claim_path_value_preserves_paths_with_spaces() {
    let claim =
        "file_exists path=/Users/test/Mobile Documents/project/mock_data/q3.txt min_bytes=806";

    assert_eq!(
        claim_path_value(claim).as_deref(),
        Some("/Users/test/Mobile Documents/project/mock_data/q3.txt")
    );
}

#[test]
fn artifact_claim_reopens_and_rehashes_the_exact_nonempty_file() {
    let root = unique_temp_dir("oomu-artifact-verifier");
    let directory = root.join("Mobile Documents").join("ship_test_04");
    let path = directory.join("recovery_plan.md");
    fs::create_dir_all(&directory).expect("create artifact directory");
    let bytes = b"# Recovery plan\n\nVerified content.\n";
    fs::write(&path, bytes).expect("write artifact");
    let digest = sha256_hex(bytes);
    let claim = format!(
        "artifact_verified=true path={} sha256={} byte_length={}",
        path.display(),
        digest,
        bytes.len()
    );
    let verifier = MlcVerifier { root: root.clone() };

    verifier
        .verify_claim(&claim)
        .expect("the exact artifact read-back verifies");
    assert!(verifier
        .verify_claim(&claim.replace(&digest, &"0".repeat(64)))
        .is_err());
    assert!(verifier
        .verify_claim(&format!("{claim} replayed=true"))
        .is_err());

    fs::remove_dir_all(root).expect("remove artifact fixture");
}

#[test]
fn verifier_accepts_path_claims_with_spaces_under_root() {
    let root = unique_temp_dir("oomu-verifier-root-with-space");
    let dir = root.join("Mobile Documents").join("mock_data");
    let file = dir.join("q3 strategic vendor proposals.txt");
    fs::create_dir_all(&dir).expect("create test dir");
    fs::write(&file, "vendor data").expect("write test file");
    let verifier = MlcVerifier { root: root.clone() };

    verifier
        .verify_claim(&format!("dir_exists path={}", dir.display()))
        .expect("directory claim with spaces verifies");
    verifier
        .verify_claim(&format!("file_exists path={} min_bytes=4", file.display()))
        .expect("file claim with spaces verifies");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn verifier_accepts_directory_entry_and_approved_external_write_claims() {
    let root = unique_temp_dir("oomu-verifier-root");
    let listed_dir = root.join("listed directory");
    let external_dir = unique_temp_dir("oomu-verifier-external");
    let external_file = external_dir.join("vendor margin audit.md");
    fs::create_dir_all(&listed_dir).expect("create listed dir");
    fs::create_dir_all(&external_dir).expect("create external dir");
    fs::write(&external_file, "approved content").expect("write external file");
    let verifier = MlcVerifier { root: root.clone() };

    verifier
        .verify_claim("directory_entries count=4")
        .expect("legacy count-only directory claim verifies");
    verifier
        .verify_claim(&format!(
            "directory_entries path={} count=4",
            listed_dir.display()
        ))
        .expect("path-bearing directory claim verifies");
    verifier
        .verify_claim(&format!(
            "shield_gate_approved_external_write path={} min_bytes=4",
            external_file.display()
        ))
        .expect("approved external write claim verifies");

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(external_dir);
}

#[test]
fn verifier_rehashes_created_files_and_rejects_tampered_digests() {
    let root = unique_temp_dir("oomu-created-file-verifier-root");
    let external_dir = unique_temp_dir("oomu-created-file-verifier-external");
    let external_file = external_dir.join("verification-canary.pdf");
    fs::create_dir_all(&root).expect("create verifier root");
    fs::create_dir_all(&external_dir).expect("create external dir");
    fs::write(&external_file, b"%PDF-1.7\nHello World\n%%EOF").expect("write created-file fixture");
    let digest = sha256_file_hex(&external_file).expect("hash created-file fixture");
    let verifier = MlcVerifier { root: root.clone() };
    let content_digest = sha256_hex(b"Hello World");
    let byte_length = fs::metadata(&external_file).unwrap().len();
    let claim = format!(
            "local_file_created format=pdf sha256={} content_sha256={} byte_length={} verification_method=production_structural_content_verifier path={}",
            digest,
            content_digest,
            byte_length,
            external_file.display()
        );

    verifier
        .verify_claim(&claim)
        .expect("exact created-file digest verifies");

    let tampered = claim.replace(&digest, &"0".repeat(64));
    let error = verifier
        .verify_claim(&tampered)
        .expect_err("tampered created-file digest is rejected");
    assert!(error.contains("digest mismatch"));

    let wrong_format = claim.replace("format=pdf", "format=txt");
    let error = verifier
        .verify_claim(&wrong_format)
        .expect_err("created-file format must match its extension");
    assert!(error.contains("format mismatch"));

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(external_dir);
}

#[test]
fn semantic_operation_requires_detailed_reasoning() {
    let claim = "operation=document_index status=completed";
    assert!(verify_semantic_reasoning_claim("document_index", claim).is_err());
}

#[test]
fn semantic_reasoning_score_and_hash_must_match() {
    let reasoning = "semantic_pass=true score=0.8200; factors=coverage:0.72,path:0.10; decision=retain grounded document";
    let claim = semantic_claim("document_index", 0.82, reasoning);
    verify_semantic_reasoning_claim("document_index", &claim).expect("valid semantic claim");

    let mismatched = claim.replace("relevance_score=0.8200", "relevance_score=0.9100");
    assert!(verify_semantic_reasoning_claim("document_index", &mismatched).is_err());
}

#[test]
fn local_certificate_claim_verifies_hash_output_and_signature() {
    let output_sha256 = "a".repeat(64);
    let identity = SovereignIdentity::initialize().expect("identity initializes");
    let claim = signed_local_certificate_claim(&identity, &output_sha256);
    let verifier = MlcVerifier::new();

    verifier
        .verify_local_certificate_claim(&claim, &identity)
        .expect("valid local certificate claim verifies");

    let mismatched_output = "b".repeat(64);
    let tampered = claim.replace(
        &format!("output_sha256={output_sha256}"),
        &format!("output_sha256={mismatched_output}"),
    );
    assert!(verifier
        .verify_local_certificate_claim(&tampered, &identity)
        .is_err());
}

#[test]
fn completed_decision_pack_mlc_accepts_every_verified_runtime_claim() {
    let root = unique_temp_dir("oomu-decision-pack-mlc");
    fs::create_dir_all(&root).expect("create verifier root");
    let identity = SovereignIdentity::initialize_with_session_passphrase(
        "OOMU decision pack evidence verifier 208",
    )
    .expect("memory-only identity initializes");
    let digest = "0123456789abcdef".repeat(4);
    let claims = [
            format!("decision_pack_file_verified=true kind=workbook path_sha256={digest} sha256={digest} byte_count=143750"),
            format!("decision_pack_file_verified=true kind=presentation path_sha256={digest} sha256={digest} byte_count=99889"),
            format!("decision_pack_file_verified=true kind=pdf path_sha256={digest} sha256={digest} byte_count=62935"),
            format!("decision_pack_file_verified=true kind=sources path_sha256={digest} sha256={digest} byte_count=3770"),
            format!("decision_pack_analysis_verified=true analysis_sha256={digest} official_web_sources=1"),
            "calendar_event_created=true reused_existing=false".to_string(),
            format!("calendar_event_verified=true exists=true event_id_sha256={digest}"),
            format!("mail_draft_saved=true sent=false reused_existing=false draft_id_sha256={digest} subject_sha256={digest} body_sha256={digest}"),
            format!("decision_pack_postcondition_verified=true file_count=4 calendar_exact_match_count=1 mail_exact_match_count=1 evidence_sha256={digest}"),
        ];
    let mlc_path = root.join("decision-pack.mlc.md");
    fs::write(
        &mlc_path,
        claims
            .iter()
            .map(|claim| format!("- CLAIM {claim}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .expect("write completed decision-pack MLC");

    let report = MlcVerifier { root: root.clone() }
        .verify_with_identity(mlc_path.to_str().unwrap(), &identity)
        .expect("all completed decision-pack evidence claims verify");
    assert!(report.verified);
    assert_eq!(report.claims_checked, 9);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn completed_release_recovery_mlc_accepts_every_verified_runtime_claim() {
    let root = unique_temp_dir("oomu-release-recovery-mlc");
    fs::create_dir_all(&root).expect("create verifier root");
    let identity = SovereignIdentity::initialize_with_session_passphrase(
        "OOMU release recovery evidence verifier",
    )
    .expect("memory-only identity initializes");
    let digest = "0123456789abcdef".repeat(4);
    let claims = [
            format!("release_recovery_agenda_verified=true output_sha256={digest} input_sha256={digest} path_sha256={digest} agenda_item_count=5 start_date=2026-07-21T13:30:00-04:00 end_date=2026-07-21T14:00:00-04:00"),
            "calendar_event_created=true reused_existing=false".to_string(),
            format!("calendar_event_verified=true exists=true event_id_sha256={digest}"),
            format!("mail_draft_saved=true sent=false reused_existing=false draft_id_sha256={digest} subject_sha256={digest} body_sha256={digest}"),
            format!("release_recovery_postcondition_verified=true file_count=1 calendar_exact_match_count=1 mail_exact_match_count=1 sent_match_count=0 evidence_sha256={digest}"),
        ];
    let mlc_path = root.join("release-recovery.mlc.md");
    fs::write(
        &mlc_path,
        claims
            .iter()
            .map(|claim| format!("- CLAIM {claim}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .expect("write completed release-recovery MLC");

    let report = MlcVerifier { root: root.clone() }
        .verify_with_identity(mlc_path.to_str().unwrap(), &identity)
        .expect("all completed release-recovery evidence claims verify");
    assert!(report.verified);
    assert_eq!(report.claims_checked, 5);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn memory_only_identity_verifies_operation_and_local_certificate_claims_end_to_end() {
    let root = unique_temp_dir("oomu-memory-identity-mlc");
    fs::create_dir_all(&root).expect("create verifier root");
    let identity =
        SovereignIdentity::initialize_with_session_passphrase("OOMU memory identity verifier 148")
            .expect("memory-only identity initializes");
    let node = identity
        .generate_node_identity()
        .expect("memory-only node identity resolves");
    let output_hash = sha256_hex(b"verified decision-pack task receipt");
    let signature = identity
        .sign_node_payload(&output_hash)
        .expect("memory-only node signs the operation receipt");
    let operation_claim = format!(
        "operation=create_decision_pack status=completed node_id={} hash={} signature_json={}",
        node.node_id,
        output_hash,
        serde_json::to_string(&signature).expect("signature serializes")
    );
    let certificate_claim = signed_local_certificate_claim(&identity, &output_hash);
    let mlc_path = root.join("execution.mlc.md");
    fs::write(
        &mlc_path,
        format!("- CLAIM {operation_claim}\n- CLAIM {certificate_claim}\n"),
    )
    .expect("write MLC fixture");

    let report = MlcVerifier { root: root.clone() }
        .verify_with_identity(mlc_path.to_str().unwrap(), &identity)
        .expect("the live memory-only identity verifies every signed claim");

    assert!(report.verified);
    assert_eq!(report.claims_checked, 2);
    let _ = fs::remove_dir_all(root);
}
