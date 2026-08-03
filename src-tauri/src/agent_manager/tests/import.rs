use super::*;

#[test]
fn execute_import_request_rejects_renderer_directory_paths() {
    let value = serde_json::json!({
        "directoryPath": "/",
        "grantId": "forged",
        "scanToken": "forged",
        "keysToImport": [],
        "agentName": "Imported",
        "agentDescription": "Imported agent",
        "modelId": "gemma-4-2b",
        "providerId": "local_model",
        "personalityTemplate": "everyday_agent"
    });

    assert!(serde_json::from_value::<ExecuteAgentImportRequest>(value).is_err());
}

#[test]
fn agent_import_grant_scans_and_consumes_exact_content_once() {
    let root = temp_agent_import_root("grant-once");
    fs::create_dir_all(root.join("memory")).unwrap();
    fs::write(root.join("SOUL.md"), "# Exact Soul").unwrap();
    fs::write(
        root.join("memory/2026-07-01.md"),
        "# Journal\n- Exact entry",
    )
    .unwrap();
    let issued = issue_agent_import_grant(&root).unwrap();
    let scanned = scan_agent_import_grant(&issued.grant_id, LogImportRange::AllHistory).unwrap();
    let keys = scanned
        .files
        .iter()
        .map(|file| file.key.clone())
        .collect::<Vec<_>>();

    let consumed =
        consume_agent_import_grant(&issued.grant_id, &scanned.scan_token, &keys).unwrap();
    assert_eq!(
        consumed.blueprint_content.get("soul").map(String::as_str),
        Some("# Exact Soul")
    );
    assert_eq!(consumed.journal_files.len(), 1);
    assert!(consumed.journal_files[0].content.contains("Exact entry"));
    assert!(consume_agent_import_grant(&issued.grant_id, &scanned.scan_token, &keys).is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn agent_import_grant_rejects_changed_file_identity() {
    let root = temp_agent_import_root("changed");
    fs::create_dir_all(&root).unwrap();
    let soul = root.join("SOUL.md");
    fs::write(&soul, "# Original").unwrap();
    let issued = issue_agent_import_grant(&root).unwrap();
    let scanned = scan_agent_import_grant(&issued.grant_id, LogImportRange::None).unwrap();
    fs::write(&soul, "# Changed content with a new size").unwrap();

    let error =
        consume_agent_import_grant(&issued.grant_id, &scanned.scan_token, &["soul".to_string()])
            .unwrap_err();
    assert_eq!(error.code, "agent_configuration_authorization_block");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn agent_import_grant_rejects_expiry_and_stale_scan_tokens() {
    let expired_root = temp_agent_import_root("expired");
    fs::create_dir_all(&expired_root).unwrap();
    fs::write(expired_root.join("SOUL.md"), "# Expired").unwrap();
    let expired = issue_agent_import_grant(&expired_root).unwrap();
    agent_import_grant_store()
        .lock()
        .unwrap()
        .grants
        .get_mut(&expired.grant_id)
        .unwrap()
        .expires_at_ms = unix_time_ms() - 1;
    assert!(scan_agent_import_grant(&expired.grant_id, LogImportRange::None).is_err());

    let stale_root = temp_agent_import_root("stale-scan");
    fs::create_dir_all(&stale_root).unwrap();
    fs::write(stale_root.join("SOUL.md"), "# Stale Scan").unwrap();
    let issued = issue_agent_import_grant(&stale_root).unwrap();
    let first = scan_agent_import_grant(&issued.grant_id, LogImportRange::None).unwrap();
    let _second = scan_agent_import_grant(&issued.grant_id, LogImportRange::None).unwrap();
    assert!(
        consume_agent_import_grant(&issued.grant_id, &first.scan_token, &["soul".to_string()],)
            .is_err()
    );
    let _ = fs::remove_dir_all(expired_root);
    let _ = fs::remove_dir_all(stale_root);
}

#[cfg(unix)]
#[test]
fn agent_import_picker_rejects_symlink_blueprint() {
    use std::os::unix::fs::symlink;

    let root = temp_agent_import_root("symlink");
    let outside = temp_agent_import_root("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("SOUL.md"), "# Outside Soul").unwrap();
    symlink(outside.join("SOUL.md"), root.join("SOUL.md")).unwrap();

    let error = issue_agent_import_grant(&root).unwrap_err();
    assert_eq!(error.code, "agent_configuration_authorization_block");
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn agent_import_picker_enforces_journal_depth_limit() {
    let root = temp_agent_import_root("depth");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("SOUL.md"), "# Soul").unwrap();
    let mut nested = root.join("memory");
    for index in 0..=MAX_AGENT_IMPORT_DISCOVERY_DEPTH {
        nested = nested.join(format!("level-{index}"));
    }
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("journal.md"), "# Too Deep").unwrap();

    assert!(issue_agent_import_grant(&root).is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn imported_agent_profile_does_not_use_legacy_name_checks() {
    let profile_json = imported_agent_personality_profile(
        &import_request("OOMU", "everyday_agent"),
        &AgentMetadata::default(),
    );
    let agent = AgentConfig {
        id: "imported-oomu".to_string(),
        name: "OOMU".to_string(),
        system_prompt: "You are a user-imported agent. Keep the work clear and grounded."
            .to_string(),
        model_id: "gemma-4-2b".to_string(),
        provider_id: "local_model".to_string(),
        description: "A McKinsey-grade strategic partner and systems architect.".to_string(),
        image: None,
        personality_profile: profile_json.to_string(),
        favorited: false,
        status: AgentConfigStatus::Active,
        created_at_ms: 1,
        updated_at_ms: 1,
    };

    let profile = agent.personality_profile().expect("OOMU import profile");
    assert_eq!(
        profile
            .template
            .as_ref()
            .map(|template| template.id.as_str()),
        Some("everyday_agent")
    );
    assert_eq!(profile.identity.role, "Imported Agent");
    assert_eq!(
        profile.personality.traits,
        vec![
            "helpful".to_string(),
            "clear".to_string(),
            "steady".to_string(),
        ]
    );

    let prompt = agent.dynamic_system_prompt().expect("OOMU prompt");
    assert!(prompt.contains("Template ID: everyday_agent"));
    assert!(prompt.contains("Configured role: Imported Agent"));
    assert!(!prompt.contains("Template ID: strategic_architect"));
}

#[test]
fn scan_agent_import_directory_discovers_memory_journals_in_chronological_group() {
    let root = std::env::temp_dir().join(format!(
        "oomu-agent-import-scan-{}-{}",
        std::process::id(),
        unix_time_ms()
    ));
    let memory_dir = root.join("memory");
    let nested_dir = memory_dir.join("daily");
    let memories_dir = root.join("memories");
    fs::create_dir_all(&nested_dir).expect("memory daily directory created");
    fs::create_dir_all(&memories_dir).expect("memories directory created");
    fs::write(root.join("SOUL.md"), "# Agent Soul").expect("soul file written");
    fs::write(memory_dir.join("2026-01-01.md"), "# 2026-01-01\n- First")
        .expect("first journal written");
    std::thread::sleep(std::time::Duration::from_millis(5));
    fs::write(
        memories_dir.join("2026-01-02.json"),
        r#"{"entry":"Second"}"#,
    )
    .expect("json journal written");
    std::thread::sleep(std::time::Duration::from_millis(5));
    fs::write(nested_dir.join("2026-01-03.md"), "# 2026-01-03\n- Third")
        .expect("nested journal written");
    fs::write(memory_dir.join("notes.txt"), "ignored").expect("ignored file written");

    let response = scan_agent_import_directory_sync(&root, LogImportRange::AllHistory)
        .expect("directory scans");
    let journals = response
        .files
        .iter()
        .filter(|file| file.group == JOURNAL_IMPORT_GROUP)
        .collect::<Vec<_>>();

    assert_eq!(journals.len(), 3);
    assert!(journals
        .iter()
        .all(|file| file.key.starts_with(JOURNAL_IMPORT_KEY_PREFIX)));
    assert!(journals
        .iter()
        .any(|file| file.relative_path == "memory/daily/2026-01-03.md"));
    assert!(journals.windows(2).all(|pair| {
        pair[0].modified_at_ms.unwrap_or_default() <= pair[1].modified_at_ms.unwrap_or_default()
    }));
    assert!(response
        .files
        .iter()
        .any(|file| { file.group == BLUEPRINT_IMPORT_GROUP && file.relative_path == "SOUL.md" }));

    let fresh_response =
        scan_agent_import_directory_sync(&root, LogImportRange::None).expect("fresh scan");
    assert!(fresh_response
        .files
        .iter()
        .all(|file| file.group != JOURNAL_IMPORT_GROUP));
    assert!(fresh_response
        .files
        .iter()
        .any(|file| { file.group == BLUEPRINT_IMPORT_GROUP && file.relative_path == "SOUL.md" }));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn log_import_range_keeps_ten_most_recent_journals_chronologically() {
    let mut files = (0..12)
        .map(|index| ScannedAgentFile {
            key: format!("{JOURNAL_IMPORT_KEY_PREFIX}memory/2026-01-{index:02}.md"),
            filename: format!("2026-01-{index:02}.md"),
            relative_path: format!("memory/2026-01-{index:02}.md"),
            size_bytes: 128,
            modified_at_ms: Some(1_000 + index),
            group: JOURNAL_IMPORT_GROUP.to_string(),
            label: "Chronological Journal".to_string(),
            description: "A dated memory note from this assistant's history.".to_string(),
            selected_by_default: true,
        })
        .collect::<Vec<_>>();

    apply_log_import_range(&mut files, LogImportRange::Last10Days);

    assert_eq!(files.len(), 10);
    assert_eq!(files[0].relative_path, "memory/2026-01-02.md");
    assert_eq!(files[9].relative_path, "memory/2026-01-11.md");
    assert!(files.windows(2).all(|pair| {
        pair[0].modified_at_ms.unwrap_or_default() <= pair[1].modified_at_ms.unwrap_or_default()
    }));
}
