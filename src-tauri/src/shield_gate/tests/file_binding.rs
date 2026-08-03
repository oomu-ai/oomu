use super::*;

#[test]
fn approved_file_receipt_is_signed_and_bound_to_one_root_task() {
    let identity = SovereignIdentity::initialize_ephemeral();
    let content = "Observed text from the approved file.".to_string();
    let now_ms = unix_time_ms_i64();
    let payload = ApprovedFileReceiptPayload {
        version: APPROVED_CHAT_FILE_RECEIPT_VERSION,
        receipt_id: "a".repeat(48),
        session_id: "session-1".to_string(),
        issued_turn_id: "turn-1".to_string(),
        root_turn_id: "turn-1".to_string(),
        agent_id: "agent-1".to_string(),
        target_identity_hash: "b".repeat(64),
        display_name: "Quarterly Review.txt".to_string(),
        mime_type: "text/plain".to_string(),
        byte_count: content.len(),
        content_sha256: sha256_hex(content.as_bytes()),
        content,
        media_sha256: None,
        display_message: "Please review my quarterly file.".to_string(),
        issued_at_ms: now_ms,
        expires_at_ms: now_ms + 60_000,
    };
    let payload_json = serde_json::to_string(&payload).unwrap();
    let token = ApprovedFileReceiptToken {
        payload: URL_SAFE_NO_PAD.encode(payload_json.as_bytes()),
        signature: identity.sign_payload(&payload_json).unwrap(),
    };

    let verified =
        verify_approved_file_receipt(&token, &identity, "session-1", "turn-1", "agent-1").unwrap();
    assert_eq!(verified.display_name, "Quarterly Review.txt");
    assert_eq!(verified.content, "Observed text from the approved file.");
    assert!(verified.data_base64.is_none());
    assert!(verify_approved_file_receipt(
        &token,
        &identity,
        "session-1",
        "different-root",
        "agent-1",
    )
    .is_err());

    let mut tampered = token;
    tampered.payload.push('A');
    assert!(
        verify_approved_file_receipt(&tampered, &identity, "session-1", "turn-1", "agent-1",)
            .is_err()
    );
}

#[test]
fn approved_image_receipt_hydrates_authenticated_pixels_without_a_path() {
    let identity = SovereignIdentity::initialize_ephemeral();
    let bytes = Zeroizing::new(vec![137, 80, 78, 71, 1, 2, 3, 4]);
    let sha256 = sha256_hex(bytes.as_slice());
    let content = "Observed image context.".to_string();
    let now_ms = unix_time_ms_i64();
    let receipt_id = "c".repeat(48);
    let payload = ApprovedFileReceiptPayload {
        version: APPROVED_CHAT_FILE_RECEIPT_VERSION,
        receipt_id: receipt_id.clone(),
        session_id: "session-image".to_string(),
        issued_turn_id: "turn-image".to_string(),
        root_turn_id: "root-image".to_string(),
        agent_id: "agent-image".to_string(),
        target_identity_hash: "d".repeat(64),
        display_name: "photo.png".to_string(),
        mime_type: "image/png".to_string(),
        byte_count: bytes.len(),
        content_sha256: sha256_hex(content.as_bytes()),
        content,
        media_sha256: Some(sha256.clone()),
        display_message: "What is in this image?".to_string(),
        issued_at_ms: now_ms,
        expires_at_ms: now_ms + 60_000,
    };
    cache_approved_chat_file_media(
        receipt_id,
        ApprovedChatFileMedia {
            session_id: payload.session_id.clone(),
            root_turn_id: payload.root_turn_id.clone(),
            agent_id: payload.agent_id.clone(),
            mime_type: payload.mime_type.clone(),
            sha256,
            issued_at_ms: now_ms,
            expires_at_ms: payload.expires_at_ms,
            bytes,
        },
    )
    .unwrap();
    let payload_json = serde_json::to_string(&payload).unwrap();
    let token = ApprovedFileReceiptToken {
        payload: URL_SAFE_NO_PAD.encode(payload_json.as_bytes()),
        signature: identity.sign_payload(&payload_json).unwrap(),
    };

    let verified = verify_approved_file_receipt(
        &token,
        &identity,
        "session-image",
        "root-image",
        "agent-image",
    )
    .unwrap();
    assert_eq!(verified.mime_type, "image/png");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(verified.data_base64.unwrap())
            .unwrap(),
        vec![137, 80, 78, 71, 1, 2, 3, 4]
    );
}

#[test]
fn shield_observability_summaries_exclude_paths_content_and_error_messages() {
    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: Some("principal".to_string()),
        path: Some("/Volumes/client-canary/private.txt".to_string()),
        content: Some("message-body-canary token=raw-secret-canary".to_string()),
    };
    let input = action_observability_summary(&action);
    for canary in ["client-canary", "message-body-canary", "raw-secret-canary"] {
        assert!(!input.contains(canary), "input leaked {canary}: {input}");
    }

    let error = ShieldGateError {
        code: "security_boundary_violation",
        boundary: "ShieldGate",
        message: "/Volumes/client-canary message-body-canary".to_string(),
    };
    let error_summary = shield_error_observability(&error);
    assert!(!error_summary.contains("client-canary"));
    assert!(!error_summary.contains("message-body-canary"));
}

#[test]
fn strict_generic_certificate_still_requires_reasoning_path() {
    let certificate = LogicalCertificate::unsigned(
        vec!["File read path and objective are declared.".to_string()],
        vec!["Read the file through the bounded filesystem tool.".to_string()],
        "The file read remains inside ShieldGate constraints.".to_string(),
    );

    assert!(certificate.validate().is_err());
}

#[test]
fn unified_diff_parser_extracts_file_hunks() {
    let diff = "\
diff --git a/src/app/page.tsx b/src/app/page.tsx
--- a/src/app/page.tsx
+++ b/src/app/page.tsx
@@ -1,3 +1,3 @@
 export default function Page() {
-  return <main>Old</main>;
+  return <main>New</main>;
 }
";

    let patches = parse_unified_diff(diff).expect("diff parses");

    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].path, "src/app/page.tsx");
    assert_eq!(patches[0].hunks.len(), 1);
    assert!(patches[0].hunks[0].old_block.contains("Old"));
    assert!(patches[0].hunks[0].new_block.contains("New"));
}

#[test]
fn diagnostic_allowlist_rejects_implicit_home_read_paths() {
    assert!(resolve_diagnostic_read_path("file_list", "Downloads").is_err());
    assert!(!is_diagnostic_query_permitted(
        "file_read",
        "~/Downloads/sprint219-private-canary.txt"
    ));
    assert!(!is_diagnostic_query_permitted("system_audit", "Documents"));
}

#[test]
fn diagnostic_allowlist_permits_profile_checks_without_writes() {
    assert!(is_diagnostic_query_permitted("file_read", "soul.md"));
    assert!(is_diagnostic_query_permitted("file_list", "soul_manifest"));
    assert!(!is_diagnostic_query_permitted(
        "file_write",
        "~/Downloads/note.txt"
    ));
}

#[test]
fn diagnostic_allowlist_rejects_protected_system_paths() {
    assert!(!is_diagnostic_query_permitted("file_list", "/etc"));
    assert!(!is_diagnostic_query_permitted(
        "file_read",
        "/var/log/system.log"
    ));
    assert!(!is_diagnostic_query_permitted(
        "file_list",
        "/private/var/log"
    ));
}

#[test]
fn file_list_downloads_requires_picker_authority() {
    let action = RequestedAction {
        kind: "file_list".to_string(),
        principal: None,
        path: Some("~/Downloads/sprint219-private-canary".to_string()),
        content: None,
    };

    let error = authorize_action(action).expect_err("implicit HOME access must be blocked");
    assert_eq!(error.code, "security_boundary_violation");
    assert!(!error.message.contains("sprint219-private-canary"));
}

#[test]
fn visual_consent_file_write_includes_semantic_diff_and_scope_trust() {
    let temp_dir = std::env::temp_dir().join(format!("oomu_visual_consent_{}", unix_time_ms_i64()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let target = temp_dir.join("preview.md");
    std::fs::write(&target, "old\n").unwrap();
    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(target.display().to_string()),
        content: Some("new\n".to_string()),
    };

    let approval =
        build_shield_approval_request(&action).expect("file write should request approval");

    assert_eq!(approval.approval_tier, "visual_consent");
    assert_eq!(approval.approval_mode, "visual");
    assert_eq!(approval.risk_tier, "Medium Risk");
    assert!(approval.semantic_summary.contains("Save proposed changes"));
    assert!(approval
        .diff_preview
        .as_deref()
        .is_some_and(|diff| { !diff.contains("-old") && diff.contains("+new") }));
    assert!(approval.scope_trust_available);
    let canonical_temp_dir = fs::canonicalize(&temp_dir).unwrap();
    assert!(approval
        .scope_trust_prefix
        .as_deref()
        .is_some_and(|prefix| prefix == canonical_temp_dir.display().to_string()));
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn create_file_binds_the_destination_and_offers_clear_folder_access_choices() {
    let _ = super::super::super::artifacts::register_file_task_tool();
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let downloads = home.join("Downloads");
    if !downloads.is_dir() {
        return;
    }
    let destination = downloads.join("OOMU Permission Test.pdf");
    let arguments = serde_json::json!({"file":{
        "title":"OOMU Permission Test",
        "content":"Hello World",
        "locale":"en-US",
        "format":"pdf",
        "destinationPath":destination.display().to_string(),
    }});
    let action = RequestedAction {
        kind: "create_file".to_string(),
        principal: None,
        path: Some(destination.display().to_string()),
        content: Some(arguments.to_string()),
    };

    let approval = build_shield_approval_request(&action)
        .expect("creating a Downloads file should request approval");
    assert_eq!(approval.action_class, "filesystem_write");
    assert_eq!(approval.action_label, "Create a local file");
    assert_eq!(
        approval.semantic_summary,
        "Create OOMU Permission Test.pdf."
    );
    assert!(approval.diff_preview.is_none());
    assert_eq!(approval.approval_scope_kinds[0], "once");
    assert!(approval
        .approval_scope_kinds
        .iter()
        .any(|kind| kind == "app_session"));
    assert!(approval
        .approval_scope_kinds
        .iter()
        .any(|kind| kind == "persistent"));

    let rejected = authorize_action(action.clone()).expect_err("approval is required");
    assert_eq!(rejected.code, "shield_gate_rejected");
    assert!(matches!(
        authorize_action_for_execution(action, true),
        Ok(AuthorizedActions::RegisteredTaskTool(request))
            if request.operation == "create_file"
    ));
}

#[test]
fn external_file_reads_and_folder_lists_require_and_honor_shield_approval() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_external_read_{}_{}",
        std::process::id(),
        unix_time_ms_i64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let target = temp_dir.join("private-note.txt");
    std::fs::write(&target, "approved local content").unwrap();

    let read_action = RequestedAction {
        kind: "file_read".to_string(),
        principal: None,
        path: Some(target.display().to_string()),
        content: None,
    };
    let read_approval = build_shield_approval_request(&read_action)
        .expect("an external file read must open a Shield approval");
    assert_eq!(read_approval.approval_tier, "visual_consent");
    assert_eq!(read_approval.action_class, "filesystem_read");
    assert!(authorize_action(read_action.clone()).is_err());
    match authorize_action_for_execution(read_action, true)
        .expect("an approved external file read should be classified")
    {
        AuthorizedActions::ApprovedExternalFileRead(request) => {
            let response =
                handle_authorized_action(AuthorizedActions::ApprovedExternalFileRead(request));
            assert_eq!(response.operation, "file_read");
            assert!(response.verified);
        }
        other => panic!("expected approved external file read, got {other:?}"),
    }

    let image_target = temp_dir.join("preview.png");
    let image_bytes = hex::decode(
            "89504e470d0a1a0a0000000d4948445200000001000000010804000000b51c0c020000000b4944415478da6364f80f00010501012718e3660000000049454e44ae426082",
        )
        .unwrap();
    std::fs::write(&image_target, image_bytes).unwrap();
    let image_action = RequestedAction {
        kind: "file_read".to_string(),
        principal: None,
        path: Some(image_target.display().to_string()),
        content: None,
    };
    match authorize_action_for_execution(image_action, true)
        .expect("an approved external image should be classified")
    {
        AuthorizedActions::ApprovedExternalFileRead(request) => {
            let response =
                handle_authorized_action(AuthorizedActions::ApprovedExternalFileRead(request));
            assert!(response.verified, "{}", response.message);
            assert!(response.message.contains("Visual analysis for preview.png"));
        }
        other => panic!("expected approved external image read, got {other:?}"),
    }

    let unsupported_target = temp_dir.join("workbook.xlsx");
    std::fs::write(&unsupported_target, b"PK\x03\x04\0unsupported").unwrap();
    let unsupported_action = RequestedAction {
        kind: "file_read".to_string(),
        principal: None,
        path: Some(unsupported_target.display().to_string()),
        content: None,
    };
    match authorize_action_for_execution(unsupported_action, true)
        .expect("an approved external binary should reach bounded parsing")
    {
        AuthorizedActions::ApprovedExternalFileRead(request) => {
            let response =
                handle_authorized_action(AuthorizedActions::ApprovedExternalFileRead(request));
            assert!(!response.verified);
            assert_eq!(response.status.as_str(), "failed");
        }
        other => panic!("expected approved external binary read, got {other:?}"),
    }

    let list_action = RequestedAction {
        kind: "file_list".to_string(),
        principal: None,
        path: Some(temp_dir.display().to_string()),
        content: None,
    };
    let list_approval = build_shield_approval_request(&list_action)
        .expect("an external folder list must open a Shield approval");
    assert_eq!(list_approval.approval_tier, "visual_consent");
    assert_eq!(list_approval.action_class, "filesystem_read");
    assert!(authorize_action(list_action.clone()).is_err());
    match authorize_action_for_execution(list_action, true)
        .expect("an approved external folder list should be classified")
    {
        AuthorizedActions::ApprovedExternalFileList(request) => {
            let response =
                handle_authorized_action(AuthorizedActions::ApprovedExternalFileList(request));
            assert_eq!(response.operation, "file_list");
            assert!(response.verified);
        }
        other => panic!("expected approved external folder list, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn external_directory_sent_as_file_read_recovers_to_an_honest_folder_listing() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu corrected mock data {} {}.json",
        std::process::id(),
        unix_time_ms_i64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    std::fs::write(temp_dir.join("supplier_proposals.json"), "{}").unwrap();
    let shell_escaped_path = temp_dir.display().to_string().replace(' ', "\\ ");
    let action = RequestedAction {
        kind: "file_read".to_string(),
        principal: None,
        path: Some(shell_escaped_path),
        content: None,
    };

    let approval = build_shield_approval_request(&action)
        .expect("a corrected external directory should open a folder approval");
    assert_eq!(approval.action_type, "file_list");
    assert_eq!(approval.action_label, "View a local folder");
    assert_eq!(approval.action_class, "filesystem_read");
    assert_eq!(approval.approval_scope_kinds[0], "once");
    assert!(approval
        .approval_scope_kinds
        .iter()
        .any(|kind| kind == "app_session"));
    assert!(authorize_action(action.clone()).is_err());

    let managed_folder_action = RequestedAction {
        kind: "file_read".to_string(),
        principal: None,
        path: Some(development_repo_root().display().to_string()),
        content: None,
    };
    let managed_folder_approval = build_shield_approval_request(&managed_folder_action)
        .expect("a user-managed folder should offer reviewed persistent access");
    assert!(managed_folder_approval
        .approval_scope_kinds
        .iter()
        .any(|kind| kind == "persistent"));

    let (prepared, bound_action) = prepare_external_filesystem_binding(&action)
        .expect("a directory read should prepare safely")
        .expect("an external directory should use a bound external action");
    assert_eq!(bound_action.kind, "file_list");
    assert_eq!(
        bound_action.path.as_deref(),
        Some(
            std::fs::canonicalize(&temp_dir)
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
    match prepared {
        AuthorizedActions::ApprovedExternalFileList(request) => {
            let response =
                handle_authorized_action(AuthorizedActions::ApprovedExternalFileList(request));
            assert_eq!(response.operation, "file_list");
            assert!(response.verified);
            assert!(response.message.contains("supplier_proposals.json"));
        }
        other => panic!("expected approved external folder listing, got {other:?}"),
    }

    match authorize_action_for_execution(action, true)
        .expect("approved directory read should normalize to a folder listing")
    {
        AuthorizedActions::ApprovedExternalFileList(_) => {}
        other => panic!("expected approved external folder listing, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn app_session_folder_access_is_memory_only_and_covers_reads_not_siblings() {
    let temp_dir = std::env::temp_dir().join(format!(
        "oomu_app_session_scope_{}_{}",
        std::process::id(),
        unix_time_ms_i64()
    ));
    let trusted_dir = temp_dir.join("trusted");
    let sibling_dir = temp_dir.join("sibling");
    std::fs::create_dir_all(&trusted_dir).unwrap();
    std::fs::create_dir_all(&sibling_dir).unwrap();
    let trusted_file = trusted_dir.join("one.txt");
    let second_file = trusted_dir.join("two.txt");
    let sibling_file = sibling_dir.join("outside.txt");
    for path in [&trusted_file, &second_file, &sibling_file] {
        std::fs::write(path, "content").unwrap();
    }

    let approved_read = RequestedAction {
        kind: "file_read".to_string(),
        principal: None,
        path: Some(trusted_file.display().to_string()),
        content: None,
    };
    let approval =
        build_shield_approval_request(&approved_read).expect("external read should be reviewable");
    assert!(approval
        .approval_scope_kinds
        .iter()
        .any(|kind| kind == "app_session"));

    let scope_trust = ScopeTrustManager::default();
    assert!(scope_trust
        .grant_from_approval(
            &approval,
            Some(&ScopeTrustApprovalRequest {
                enabled: true,
                duration_ms: Some(1_000),
                kind: Some("app_session".to_string()),
                max_uses: None,
            }),
        )
        .expect("application-session access should be granted"));

    let second_read = RequestedAction {
        kind: "file_read".to_string(),
        principal: None,
        path: Some(second_file.display().to_string()),
        content: None,
    };
    let trusted_list = RequestedAction {
        kind: "file_list".to_string(),
        principal: None,
        path: Some(trusted_dir.display().to_string()),
        content: None,
    };
    let sibling_read = RequestedAction {
        kind: "file_read".to_string(),
        principal: None,
        path: Some(sibling_file.display().to_string()),
        content: None,
    };
    assert!(scope_trust.allows_action(&second_read).unwrap());
    assert!(scope_trust.allows_action(&trusted_list).unwrap());
    assert!(!scope_trust.allows_action(&sibling_read).unwrap());

    let fresh_process_cache = ScopeTrustManager::default();
    assert!(!fresh_process_cache.allows_action(&second_read).unwrap());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn external_symlink_paths_are_rejected_or_shown_as_their_canonical_target() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!(
        "oomu_external_symlink_{}_{}",
        std::process::id(),
        unix_time_ms_i64()
    ));
    let shown = root.join("shown");
    let actual = root.join("actual");
    let existing = actual.join("existing");
    std::fs::create_dir_all(&shown).unwrap();
    std::fs::create_dir_all(&existing).unwrap();
    let secret = actual.join("secret.txt");
    std::fs::write(&secret, "private").unwrap();
    let alias = shown.join("alias");
    symlink(&actual, &alias).unwrap();

    let read = RequestedAction {
        kind: "file_read".to_string(),
        principal: None,
        path: Some(alias.join("secret.txt").display().to_string()),
        content: None,
    };
    let read_approval = build_shield_approval_request(&read)
        .expect("an intermediate alias may proceed only with an honest canonical prompt");
    let canonical_secret = fs::canonicalize(&secret).unwrap();
    assert_eq!(
        read_approval.target_path.as_deref(),
        Some(canonical_secret.to_string_lossy().as_ref())
    );
    assert_eq!(
        read_approval.canonical_resource.as_deref(),
        Some(canonical_secret.to_string_lossy().as_ref())
    );

    let list_alias = RequestedAction {
        kind: "file_list".to_string(),
        principal: None,
        path: Some(alias.display().to_string()),
        content: None,
    };
    assert!(build_shield_approval_request(&list_alias).is_none());

    let write_through_alias = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(alias.join("new.txt").display().to_string()),
        content: Some("blocked".to_string()),
    };
    assert!(build_shield_approval_request(&write_through_alias).is_none());
    assert!(authorize_action_for_execution(write_through_alias, true).is_err());

    let canonical_write = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(alias.join("existing").join("new.txt").display().to_string()),
        content: Some("allowed after honest review".to_string()),
    };
    let write_approval = build_shield_approval_request(&canonical_write)
        .expect("a resolvable alias must expose its canonical destination");
    let canonical_existing = fs::canonicalize(&existing).unwrap();
    assert_eq!(
        write_approval.target_path.as_deref(),
        Some(
            canonical_existing
                .join("new.txt")
                .to_string_lossy()
                .as_ref()
        )
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn approved_external_read_rejects_a_symlink_swap_before_opening() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!(
        "oomu_external_read_swap_{}_{}",
        std::process::id(),
        new_approval_token()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let approved_path = root.join("approved.txt");
    let original_path = root.join("approved-original.txt");
    let secret_path = root.join("not-approved.txt");
    std::fs::write(&approved_path, "approved content").unwrap();
    std::fs::write(&secret_path, "must never be read").unwrap();
    let action = RequestedAction {
        kind: "file_read".to_string(),
        principal: None,
        path: Some(approved_path.display().to_string()),
        content: None,
    };
    let approved_request =
        match authorize_action_for_execution(action, true).expect("approval should bind") {
            AuthorizedActions::ApprovedExternalFileRead(request) => request,
            other => panic!("expected bound external read, got {other:?}"),
        };

    std::fs::rename(&approved_path, &original_path).unwrap();
    symlink(&secret_path, &approved_path).unwrap();
    let response = handle_authorized_action(AuthorizedActions::ApprovedExternalFileRead(
        approved_request,
    ));

    assert!(!response.verified);
    assert!(matches!(&response.status, CommandStatus::Failed));
    assert!(!response.message.contains("must never be read"));
    assert_eq!(
        std::fs::read_to_string(&secret_path).unwrap(),
        "must never be read"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn approved_external_write_rejects_a_parent_symlink_swap() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!(
        "oomu_external_write_swap_{}_{}",
        std::process::id(),
        new_approval_token()
    ));
    let approved_parent = root.join("approved");
    let original_parent = root.join("approved-original");
    let attacker_parent = root.join("not-approved");
    std::fs::create_dir_all(&approved_parent).unwrap();
    std::fs::create_dir_all(&attacker_parent).unwrap();
    let approved_target = approved_parent.join("note.txt");
    let attacker_target = attacker_parent.join("note.txt");
    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(approved_target.display().to_string()),
        content: Some("must stay in the approved folder".to_string()),
    };
    let approved_request =
        match authorize_action_for_execution(action, true).expect("approval should bind") {
            AuthorizedActions::ApprovedExternalFileWrite(request) => request,
            other => panic!("expected bound external write, got {other:?}"),
        };

    std::fs::rename(&approved_parent, &original_parent).unwrap();
    symlink(&attacker_parent, &approved_parent).unwrap();
    let response = handle_authorized_action(AuthorizedActions::ApprovedExternalFileWrite(
        approved_request,
    ));

    assert!(!response.verified);
    assert!(matches!(&response.status, CommandStatus::Failed));
    assert!(!attacker_target.exists());
    assert!(!original_parent.join("note.txt").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn sensitive_folder_scopes_never_offer_persistent_access() {
    assert!(!folder_scope_allows_persistent_access("/"));

    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let home = std::fs::canonicalize(home).expect("HOME should resolve during tests");
    assert!(!folder_scope_allows_persistent_access(
        &home.display().to_string()
    ));
    for protected in [
        "/System",
        "/Library",
        "/Applications",
        "/etc",
        "/private",
        "/usr",
        "/opt",
    ] {
        assert!(
            !folder_scope_allows_persistent_access(protected),
            "{protected} must never receive permanent folder authority"
        );
    }
    assert!(!folder_scope_allows_persistent_access(
        &home.join("Library").display().to_string()
    ));

    let action = RequestedAction {
        kind: "file_list".to_string(),
        principal: None,
        path: Some(home.display().to_string()),
        content: None,
    };
    let approval = build_shield_approval_request(&action)
        .expect("reading HOME should still require a one-time decision");
    assert!(approval
        .approval_scope_kinds
        .iter()
        .any(|kind| kind == "app_session"));
    assert!(!approval
        .approval_scope_kinds
        .iter()
        .any(|kind| kind == "persistent"));
}

#[test]
fn shell_escaped_icloud_path_is_normalized_before_native_validation() {
    let escaped =
        "/Users/example/Library/Mobile\\ Documents/com\\~apple\\~CloudDocs/OOMU/profile.jpeg";
    let expected = "/Users/example/Library/Mobile Documents/com~apple~CloudDocs/OOMU/profile.jpeg";

    assert_eq!(normalize_shell_escaped_path(escaped), expected);
}

#[test]
fn external_file_write_requires_shield_approval() {
    let target_path = std::env::temp_dir().join("oomu-shield-approval-test.txt");
    let _ = std::fs::remove_file(&target_path);
    let target = target_path.display().to_string();
    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(target.clone()),
        content: Some("approved external write".to_string()),
    };

    let rejected = authorize_action(action.clone()).expect_err("external write must gate");
    assert_eq!(rejected.code, "security_boundary_violation");

    match authorize_action_for_execution(action, true).expect("approved write is classified") {
        AuthorizedActions::ApprovedExternalFileWrite(request) => {
            let resolved_target = request.path.clone();
            assert!(Path::new(&resolved_target).is_absolute());
            assert_eq!(
                Path::new(&resolved_target).file_name(),
                target_path.file_name()
            );
            let response =
                handle_authorized_action(AuthorizedActions::ApprovedExternalFileWrite(request));
            assert_eq!(response.operation, "file_write");
            assert!(response.verified);
            assert!(response.model_used.is_none());
        }
        other => panic!("expected approved external write, got {other:?}"),
    }

    let _ = std::fs::remove_file(target_path);
}

#[test]
fn approved_external_write_creates_a_missing_parent_and_exact_file() {
    let root = std::env::temp_dir().join(format!(
        "oomu-missing-parent-write-{}-{}",
        std::process::id(),
        new_approval_token()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("ship_test_08").join("reliability_review.md");
    let content = "# Verified review\n";
    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(target.display().to_string()),
        content: Some(content.to_string()),
    };

    let approved_target = validate_approved_external_write_target(target.to_str().unwrap())
        .expect("the missing suffix resolves against its existing ancestor");
    let approval = build_shield_approval_request(&action)
        .expect("the exact external target must reach Shield approval");
    assert_eq!(
        approval.target_path.as_deref(),
        Some(approved_target.to_str().unwrap())
    );
    assert!(!target.parent().unwrap().exists());

    let request = match authorize_action_for_execution(action, true).expect("write binds") {
        AuthorizedActions::ApprovedExternalFileWrite(request) => request,
        other => panic!("expected approved external write, got {other:?}"),
    };
    let response = handle_authorized_action(AuthorizedActions::ApprovedExternalFileWrite(request));

    assert!(response.verified);
    assert_eq!(response.operation, "file_write");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), content);
    assert_eq!(
        std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
        1,
        "one approval creates exactly one output file"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn approved_external_write_atomically_replaces_an_existing_file() {
    use std::os::unix::fs::MetadataExt;

    let root = std::env::temp_dir().join(format!(
        "oomu-atomic-external-write-{}-{}",
        std::process::id(),
        new_approval_token()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("report.md");
    std::fs::write(&target, "original content").unwrap();
    let original_inode = std::fs::metadata(&target).unwrap().ino();
    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(target.display().to_string()),
        content: Some("verified replacement".to_string()),
    };
    let request = match authorize_action_for_execution(action, true).expect("write binds") {
        AuthorizedActions::ApprovedExternalFileWrite(request) => request,
        other => panic!("expected approved external write, got {other:?}"),
    };

    let response = handle_authorized_action(AuthorizedActions::ApprovedExternalFileWrite(request));

    assert!(response.verified);
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "verified replacement"
    );
    assert_ne!(std::fs::metadata(&target).unwrap().ino(), original_inode);
    assert!(std::fs::read_dir(&root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".oomu-write-")
    }));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn approved_external_write_preserves_files_when_the_target_changes() {
    let root = std::env::temp_dir().join(format!(
        "oomu-atomic-external-write-swap-{}-{}",
        std::process::id(),
        new_approval_token()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("report.md");
    let approved_original = root.join("approved-original.md");
    std::fs::write(&target, "approved original").unwrap();
    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(target.display().to_string()),
        content: Some("replacement".to_string()),
    };
    let request = match authorize_action_for_execution(action, true).expect("write binds") {
        AuthorizedActions::ApprovedExternalFileWrite(request) => request,
        other => panic!("expected approved external write, got {other:?}"),
    };
    std::fs::rename(&target, &approved_original).unwrap();
    std::fs::write(&target, "unapproved replacement").unwrap();

    let response = handle_authorized_action(AuthorizedActions::ApprovedExternalFileWrite(request));

    assert!(!response.verified);
    assert_eq!(
        std::fs::read_to_string(&approved_original).unwrap(),
        "approved original"
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "unapproved replacement"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(target_os = "macos")]
#[test]
fn approved_external_write_rolls_back_a_swap_in_the_final_commit_window() {
    use std::ffi::CString;
    use std::os::unix::fs::MetadataExt;

    let root = std::env::temp_dir().join(format!(
        "oomu-atomic-external-write-final-race-{}-{}",
        std::process::id(),
        new_approval_token()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("report.md");
    let displaced = root.join("approved-original.md");
    let temporary = root.join(".oomu-write-prepared.tmp");
    std::fs::write(&target, "approved original").unwrap();
    std::fs::write(&temporary, "prepared replacement").unwrap();
    let expected_metadata = std::fs::metadata(&target).unwrap();
    let prepared_metadata = std::fs::metadata(&temporary).unwrap();
    let expected_identity = ApprovedFileIdentity {
        device: expected_metadata.dev(),
        inode: expected_metadata.ino(),
    };
    let prepared_identity = ApprovedFileIdentity {
        device: prepared_metadata.dev(),
        inode: prepared_metadata.ino(),
    };
    let parent = std::fs::File::open(&root).unwrap();
    let temporary_name = CString::new(".oomu-write-prepared.tmp").unwrap();
    let target_name = CString::new("report.md").unwrap();

    // This is the exact race the old implementation missed: the approved
    // inode changes after the final verification but before publication.
    std::fs::rename(&target, &displaced).unwrap();
    std::fs::write(&target, "unapproved replacement").unwrap();

    let result = external_file_binding::commit_bound_external_write_temp(
        &parent,
        &temporary_name,
        &target_name,
        Some(expected_identity),
        prepared_identity,
    );

    assert!(result.is_err());
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "unapproved replacement"
    );
    assert_eq!(
        std::fs::read_to_string(&displaced).unwrap(),
        "approved original"
    );
    assert_eq!(
        std::fs::read_to_string(&temporary).unwrap(),
        "prepared replacement"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn approved_external_file_write_rejects_root_level_mockdata_target() {
    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some("/mockdata/calendar_draft.md".to_string()),
        content: Some("draft".to_string()),
    };

    let rejected =
        authorize_action_for_execution(action, true).expect_err("root-level write rejects");
    assert_eq!(rejected.code, "security_boundary_violation");
    assert!(rejected.message.contains("root-level directory target"));
}

#[test]
fn external_file_write_expands_user_home_paths() {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return;
    };
    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some("~/Downloads/oomu-shield-home-write.md".to_string()),
        content: Some("home write".to_string()),
    };

    match authorize_action_for_execution(action, true).expect("approved home write is classified") {
        AuthorizedActions::ApprovedExternalFileWrite(request) => {
            assert!(Path::new(&request.path).is_absolute());
            assert_eq!(
                Path::new(&request.path).file_name(),
                Some(std::ffi::OsStr::new("oomu-shield-home-write.md"))
            );
            let canonical_home = fs::canonicalize(home).unwrap();
            assert!(Path::new(&request.path).starts_with(canonical_home));
        }
        other => panic!("expected approved external write, got {other:?}"),
    }
}

#[test]
fn approved_plan_authorizes_external_file_write_without_project_quarantine() {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return;
    };
    let path = home
        .join("Downloads")
        .join("oomu-approved-plan-write.md")
        .display()
        .to_string();
    let action = RequestedAction {
        kind: "file_write".to_string(),
        principal: None,
        path: Some(path.clone()),
        content: Some("approved plan write".to_string()),
    };

    let rejected = authorize_action(action.clone()).expect_err("external write gates");
    assert_eq!(rejected.code, "security_boundary_violation");

    match authorize_action_for_approved_plan(action).expect("approved plan write authorizes") {
        AuthorizedActions::ApprovedExternalFileWrite(request) => {
            assert!(Path::new(&request.path).is_absolute());
            assert_eq!(
                Path::new(&request.path).file_name(),
                Some(std::ffi::OsStr::new("oomu-approved-plan-write.md"))
            );
            assert_eq!(request.content, "approved plan write");
        }
        other => panic!("expected approved external file write, got {other:?}"),
    }
}

#[test]
fn delete_file_requires_shield_approval_and_verifies_removal() {
    let target = std::env::temp_dir().join(format!(
        "oomu-shield-delete-{}-{}.txt",
        std::process::id(),
        unix_time_ms_i64()
    ));
    std::fs::write(&target, "temporary").expect("test file writes");
    let action = RequestedAction {
        kind: "delete_file".to_string(),
        principal: None,
        path: Some(target.display().to_string()),
        content: None,
    };

    let rejected = authorize_action(action.clone()).expect_err("delete must gate");
    assert_eq!(rejected.code, "shield_gate_rejected");

    match authorize_action_for_execution(action, true).expect("approved delete is classified") {
        AuthorizedActions::ApprovedFileDelete(request) => {
            let canonical_target = std::fs::canonicalize(&target).unwrap();
            assert_eq!(request.path, canonical_target.display().to_string());
            let response = handle_authorized_action(AuthorizedActions::ApprovedFileDelete(request));
            assert_eq!(response.operation, "delete_file");
            assert!(response.verified);
            assert!(!canonical_target.exists());
        }
        other => panic!("expected approved file delete, got {other:?}"),
    }
}

#[test]
fn missing_delete_target_has_a_stable_plain_error() {
    let target = std::env::temp_dir().join(format!(
        "oomu-shield-missing-delete-{}-{}.txt",
        std::process::id(),
        new_approval_token()
    ));
    let _ = std::fs::remove_file(&target);
    let action = RequestedAction {
        kind: "delete_file".to_string(),
        principal: None,
        path: Some(target.display().to_string()),
        content: None,
    };

    let error = authorize_action_for_approved_plan(action)
        .expect_err("a missing delete target is an actionable no-op");

    assert_eq!(error.code, "delete_target_not_found");
    assert_eq!(error.boundary, "DeleteFileAuthority");
    assert_eq!(error.message, "The requested file is not there.");
    assert!(!error.message.contains("Shield"));
    assert!(!error.message.contains("preflight"));
}

#[test]
fn codebase_patch_is_repo_only_and_rejects_host_paths() {
    let external = RequestedAction {
        kind: "codebase_patch".to_string(),
        principal: Some("127.0.0.1".to_string()),
        path: Some("/etc/hosts".to_string()),
        content: Some("127.0.0.1 localhost".to_string()),
    };
    let rejected = authorize_action(external).expect_err("external path is rejected");
    assert_eq!(rejected.code, "security_boundary_violation");

    let traversal = RequestedAction {
        kind: "codebase_patch".to_string(),
        principal: Some("old".to_string()),
        path: Some("../outside.rs".to_string()),
        content: Some("new".to_string()),
    };
    let rejected = authorize_action(traversal).expect_err("traversal is rejected");
    assert_eq!(rejected.code, "security_boundary_violation");
    assert!(rejected.message.contains("path traversal"));
}

#[test]
fn codebase_patch_authorizes_existing_repo_file() {
    let action = RequestedAction {
        kind: "codebase_patch".to_string(),
        principal: Some("export default".to_string()),
        path: Some("src/app/page.tsx".to_string()),
        content: Some("export default".to_string()),
    };

    match authorize_action(action).expect("repo patch is classified") {
        AuthorizedActions::CodebasePatch(request) => {
            assert_eq!(request.target_file_path, "src/app/page.tsx");
            assert_eq!(request.search_pattern, "export default");
        }
        other => panic!("expected codebase patch, got {other:?}"),
    }
}
