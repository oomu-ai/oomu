use super::*;
use std::fs;

fn page(content: &str) -> OfficialPageReceipt {
    OfficialPageReceipt {
        requested_url: "https://example.com/source".to_string(),
        selected_url: "https://example.com/source".to_string(),
        attempted_urls: vec!["https://example.com/source".to_string()],
        fallback_used: false,
        final_url: "https://example.com/source".to_string(),
        accessed_at_utc: "2026-07-21T12:00:00.000Z".to_string(),
        status_code: 200,
        content_type: "text/html".to_string(),
        content: content.to_string(),
        content_sha256: "aabb".to_string(),
        content_bytes: content.len(),
        content_truncated: false,
    }
}

fn comparison_analysis() -> VerifiedComparisonAnalysis {
    cloud_analysis::verified_comparison_for_test(cloud_analysis::ComparisonAnalysis {
        executive_emphasis: ComparisonEmphasis::Auditability,
        ordered_implication_ids: vec![
            ComparisonImplication::PreserveApprovalReceipts,
            ComparisonImplication::SeparateScheduleAndLedger,
            ComparisonImplication::SurfaceLocalAndRemote,
        ],
    })
}

fn recovery_analysis(release: &str, unfinished: &[&str]) -> VerifiedRecoveryAnalysis {
    cloud_analysis::verified_recovery_for_test(cloud_analysis::RecoveryAnalysis {
        release_milestone_id: release.to_string(),
        unfinished_milestone_ids: unfinished.iter().map(|id| (*id).to_string()).collect(),
        execution_mode: RecoveryExecutionMode::ParallelAcrossOwnersSerialWithinOwner,
        ordered_risk_ids: vec![
            RecoveryRisk::SecurityValidationFailure,
            RecoveryRisk::PrerequisiteSlip,
            RecoveryRisk::OwnerCapacityBlock,
        ],
    })
}

#[test]
fn comparison_rejects_partial_keyword_pages_instead_of_emitting_fixed_claims() {
    let openclaw = page("scheduled tasks cron background task fresh (isolated) or shared");
    let cowork = page("scheduled task cowork recurring on demand connected tools skills plugins");
    assert!(comparison_markdown(&openclaw, &cowork, &comparison_analysis()).is_err());
}

#[test]
fn comparison_uses_verified_cloud_emphasis_and_implication_order() {
    let openclaw = page("Cron is the Gateway's built-in scheduler for precise timing. Fresh (isolated) or shared. The background task ledger tracks all detached work. Tasks are records, not schedulers.");
    let cowork = page("Run automatically on a recurring basis, or on demand. Same capabilities as regular Cowork tasks. Connected tools, skills, and installed plugins. Run web research. Each scheduled task runs as its own Cowork session. Can't be tied to a folder on your computer. Requires local files or apps, it will only run locally.");
    let content = comparison_markdown(&openclaw, &cowork, &comparison_analysis()).unwrap();
    assert!(content.contains("Specialist emphasis:** Auditability"));
    let receipts = content.find("Persist exact approvals").unwrap();
    let schedule = content.find("Keep scheduling authority separate").unwrap();
    assert!(receipts < schedule);
}

#[test]
fn recovery_content_is_derived_from_real_records_and_names_unfinished_work() {
    let milestones = serde_json::from_str::<Vec<Milestone>>(r#"[
      {"milestone_id":"M1","name":"Security","target_date":"2026-07-06","status":"COMPLETED","owner":"Alex"},
      {"milestone_id":"M2","name":"Localization","target_date":"2026-07-10","status":"IN_PROGRESS","owner":"Alex"},
      {"milestone_id":"M3","name":"Release Validation","target_date":"2026-07-15","status":"PENDING","owner":"OOMU"}
    ]"#).unwrap();
    let content = recovery_markdown(
        "/Users/test/project/milestone_source.json",
        "aabb",
        milestones,
        &recovery_analysis("M3", &["M2", "M3"]),
    )
    .unwrap();
    assert!(content.contains("M2 (Localization, IN_PROGRESS)"));
    assert!(content.contains("M3 (Release Validation, PENDING)"));
    assert!(content.contains("M2 + security validation -> M3"));
    assert!(content.contains("20%"));
    assert!(content.contains("Three failure contingencies"));
    let security = content.find("Security validation fails").unwrap();
    let prerequisite = content.find("unfinished prerequisite slips").unwrap();
    assert!(security < prerequisite);
}

#[test]
fn recovery_content_never_invents_fixture_specific_milestone_ids() {
    let milestones = serde_json::from_str::<Vec<Milestone>>(r#"[
      {"milestone_id":"A7","name":"Localization","target_date":"2026-08-10","status":"BLOCKED","owner":"Ari"},
      {"milestone_id":"R9","name":"Release Validation","target_date":"2026-08-12","status":"PENDING","owner":"Rin","dependencies":["A7"]}
    ]"#).unwrap();
    let content = recovery_markdown(
        "/Users/test/project/source.json",
        "ccdd",
        milestones,
        &recovery_analysis("R9", &["A7", "R9"]),
    )
    .unwrap();
    assert!(content.contains("A7 + security validation -> R9"));
    assert!(!content.contains("M2"));
    assert!(!content.contains("M3"));
}

#[test]
fn recovery_rejects_cloud_analysis_that_does_not_match_native_source() {
    let milestones = serde_json::from_str::<Vec<Milestone>>(r#"[
      {"milestone_id":"M2","name":"Localization","target_date":"2026-07-10","status":"IN_PROGRESS","owner":"Alex"},
      {"milestone_id":"M3","name":"Release Validation","target_date":"2026-07-15","status":"PENDING","owner":"OOMU"}
    ]"#).unwrap();
    assert!(recovery_markdown(
        "/Users/test/project/source.json",
        "ccdd",
        milestones,
        &recovery_analysis("M3", &["M3"]),
    )
    .is_err());
}

#[test]
fn output_writer_creates_only_one_missing_parent_and_verifies_nonempty_bytes() {
    let base = std::env::temp_dir().join(format!(
        "oomu-evidence-artifact-output-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    fs::create_dir_all(&base).unwrap();
    let output = base.join("output").join("recovery.md");
    let binding =
        crate::shield_gate::bind_approved_external_file_write(output.to_str().unwrap()).unwrap();
    let receipt =
        write_verified_markdown(output.to_str().unwrap(), &binding, "# Verified\n").unwrap();
    assert!(receipt.verified);
    assert!(receipt.byte_length > 0);
    assert_eq!(fs::read_to_string(output).unwrap(), "# Verified\n");
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn output_writer_never_replaces_a_file_that_appears_after_binding() {
    let base = std::env::temp_dir().join(format!(
        "oomu-evidence-artifact-race-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    fs::create_dir_all(&base).unwrap();
    let output = base.join("recovery_plan.md");
    let binding =
        crate::shield_gate::bind_approved_external_file_write(output.to_str().unwrap()).unwrap();
    fs::write(&output, "concurrent content").unwrap();
    assert!(write_verified_markdown(output.to_str().unwrap(), &binding, "new content").is_err());
    assert_eq!(fs::read_to_string(&output).unwrap(), "concurrent content");
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn pre_write_specialist_failure_is_verified_unchanged_and_retry_safe() {
    if register_task_tools().is_err() {
        assert!(crate::tools::task_tool_runtime::schema(RECOVERY_OPERATION).is_ok());
    }
    let raw = verified_unchanged_preparation_error(RECOVERY_OPERATION);
    let normalized =
        crate::tools::task_tool_runtime::normalize_agent_error(RECOVERY_OPERATION, &raw);
    let parsed = crate::tools::task_tool_runtime::parse_retry_safe_unchanged_error(
        RECOVERY_OPERATION,
        &normalized,
    )
    .expect("pre-write specialist failures must resume without an external-change warning");

    assert_eq!(parsed.code, PREPARATION_ERROR_CODE);
    assert_eq!(
        parsed.changed_state,
        crate::tools::task_tool_runtime::TaskToolChangedState::None
    );
    assert!(parsed.changed_state_verified);
}
