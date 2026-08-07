mod build_identity_policy;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const TRUSTED_RELEASE_PUBLIC_KEY_HEX: &str =
    "10543bcbfa20b4c58d587aa969053124cc3340b11470b84ba9df763fee9100bb";
const PACKAGE_IDENTITY_DIRECTORIES: &[&str] = &[
    "out",
    "public",
    "src-tauri/binaries",
    "src-tauri/capabilities",
    "src-tauri/icons",
    "src-tauri/macos-localizations",
    "src-tauri/migrations",
    "src-tauri/oauth",
    "src-tauri/permissions",
    "src-tauri/resources/mcp",
    "src-tauri/resources/python",
];
const PACKAGE_IDENTITY_FILES: &[&str] = &[
    "THIRD_PARTY_NOTICES.md",
    "eslint.config.mjs",
    "next.config.ts",
    "postcss.config.mjs",
    "rust-toolchain.toml",
    "tsconfig.json",
    "vitest.config.ts",
    "src-tauri/.cargo/config.toml",
    "src-tauri/.taurignore",
    "src-tauri/Info.plist",
    "src-tauri/entitlements.plist",
    "src-tauri/tauri.conf.json",
    "src-tauri/tauri.development.conf.json",
    "src-tauri/tauri.release.conf.json",
    "src-tauri/build_identity_policy.rs",
];

fn main() {
    publish_release_version();
    println!("cargo:rustc-env=OOMU_MLC_STRICT_MODE=true");
    println!("cargo:rerun-if-env-changed=PATH");
    println!("cargo:rerun-if-env-changed=OOMU_RELEASE_PIPELINE");
    println!("cargo:rerun-if-env-changed=OOMU_LOCAL_UNSIGNED_BUILD");
    println!("cargo:rerun-if-env-changed=OOMU_BUILD_ID");
    println!("cargo:rerun-if-env-changed=OOMU_SOURCE_REVISION");
    println!("cargo:rerun-if-env-changed=OOMU_RELEASE_AUTHORIZATION_BASE64");
    println!("cargo:rerun-if-env-changed=OOMU_GOOGLE_OAUTH_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=OOMU_GOOGLE_OAUTH_CLIENT_SECRET");
    println!("cargo:rerun-if-env-changed=OOMU_SLACK_OAUTH_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=OOMU_SLACK_OAUTH_BROKER_URL");
    println!("cargo:rerun-if-env-changed=OOMU_SLACK_OAUTH_BROKER_CERT_SHA256");
    println!("cargo:rerun-if-env-changed=OOMU_SLACK_OAUTH_REDIRECT_PORT");
    println!("cargo:rerun-if-env-changed=OOMU_MICROSOFT_OAUTH_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=OOMU_UPDATER_PUBLIC_KEY");
    publish_updater_public_key();
    publish_frontend_export_fingerprint();
    publish_source_identity();
    publish_oauth_client_identities();
    publish_artifact_helper_digest();
    publish_artifact_pdf_helper_digest();
    verify_release_entrypoint();
    verify_build_toolchain();
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "abandon_accepted_chat_turn",
            "accept_chat_turn",
            "record_accepted_chat_turn_checkpoint",
            "accept_license",
            "activate_sovereign_identity_session_passphrase",
            "activate_sovereign_trust_session",
            "analyze_visual_artifact",
            "choose_artifact_export_destination",
            "choose_browser_upload",
            "choose_directory_path",
            "authorize_native_browser_navigation",
            "apply_surgical_patch_directive",
            "audit_ark_artifacts",
            "bind_mod_to_agent",
            "begin_connector_oauth",
            "capture_agent_chat_memories",
            "cancel_agent_execution_remaining_work",
            "cancel_task_run",
            "cancel_chat_stream",
            "cancel_delegation_child",
            "cancel_delegation_plan",
            "cancel_native_inference",
            "cancel_recommended_model_install",
            "cancel_permission_recovery_turn",
            "cancel_saved_chat_turn",
            "chat_turn",
            "choose_agent_import_directory",
            "choose_knowledge_ingest_directory",
            "choose_project_root",
            "choose_local_context",
            "claim_dropped_local_context",
            "claim_latest_dropped_local_context",
            "choose_local_model_directory",
            "choose_recommended_model_install_location",
            "choose_mod_package_path",
            "choose_presentation_export_destination",
            "choose_volatile_persistence_export",
            "choose_workflow_source_folder",
            "check_calendar_full_access",
            "check_for_application_update",
            "check_mail_automation_access",
            "classify_chat_intent_route",
            "cleanup_reconciled_volatile_persistence",
            "compact_session_history",
            "close_native_browser",
            "commit_memory_proposal",
            "compose_workflow",
            "control_browser_automation",
            "control_app_control_session",
            "create_artifact",
            "create_project_chat_document",
            "create_presentation",
            "create_workbook",
            "create_workbook_from_template",
            "inspect_workbook_template",
            "create_chat_session",
            "commit_chat_session_deletion",
            "create_decision_brief_from_delegation",
            "create_delegation_plan",
            "create_routine",
            "create_project",
            "create_taskflow",
            "delegate_signing_authority",
            "decline_license",
            "delete_agent_config",
            "delete_chat_session",
            "delete_provider_config",
            "delete_project",
            "delete_routine",
            "delete_workflow",
            "discard_recommended_model_partial",
            "edit_workflow",
            "execute_action_plan",
            "execute_browser_action",
            "execute_agent_action_plan",
            "resume_agent_execution_after_permission",
            "resume_interrupted_chat_turn",
            "request_agent_plan_authority",
            "execute_agent_import",
            "execute_queued_messages",
            "execute_command",
            "execute_native_file_access",
            "prepare_approved_chat_file",
            "read_imported_agent_source",
            "execute_connector_operation",
            "execute_delegation_plan",
            "compact_chat_session",
            "execute_semantic_compaction",
            "execute_system_apple_app_tool",
            "execute_taskflow",
            "execute_workflow",
            "export_logical_certificate",
            "export_artifact",
            "export_presentation_revision",
            "export_workbook_revision",
            "export_browser_download",
            "export_volatile_persistence",
            "finalize_accepted_chat_turn",
            "generate_node_identity",
            "get_default_prewarmed_model",
            "get_agent_config",
            "get_agent_mods",
            "get_agentic_state",
            "get_agent_execution_recovery_states",
            "get_artifact",
            "get_artifact_pipeline_health",
            "get_artifact_preview_page",
            "get_presentation_checker_readiness",
            "get_presentation_preview",
            "get_presentation_review",
            "get_workbook_preview",
            "get_workbook_review",
            "get_actuation_lease_status",
            "get_channel_statuses",
            "get_capability_health",
            "get_background_service_status",
            "get_connector_connection_status",
            "get_browser_automation_session",
            "get_app_control_status",
            "get_commander_state",
            "get_compiled_instructions",
            "get_degraded_mode_status",
            "get_knowledge_state",
            "get_launch_readiness",
            "get_launch_options",
            "get_privacy_settings",
            "get_project_memory_summary",
            "get_reviewed_approval_scopes",
            "get_routine",
            "get_routine_history",
            "get_project",
            "get_locale_state",
            "get_auto_route_classifier_health",
            "get_auto_route_session_readiness",
            "repair_auto_route_session_baseline",
            "get_local_generation_health",
            "get_local_model_directory",
            "get_local_model_status",
            "get_memory_ledger_state",
            "get_persistence_recovery_status",
            "get_queued_messages",
            "get_recoverable_actions",
            "get_routing_preference",
            "get_sandbox_status",
            "get_session_config",
            "get_session_context_status",
            "get_sovereign_identity",
            "get_sovereign_ledger_stats",
            "get_sovereign_trust_dashboard",
            "get_system_diagnostic_context",
            "get_system_hardware_profile",
            "get_taskflow_state",
            "get_task_run",
            "get_user_personality_profile",
            "get_weekly_decision_brief_status",
            "get_workflow_capability_catalog",
            "get_workflow_irs",
            "get_workflows",
            "grant_actuation_lease",
            "grant_routine_authority",
            "hydrate_agent_prompt_context",
            "infer",
            "ingest_media_asset",
            "list_media_assets",
            "get_media_asset_data",
            "save_media_transcript",
            "delete_media_asset",
            "sanitize_media_image",
            "analyze_media_image",
            "save_media_alt_text",
            "list_remote_devices",
            "rename_remote_device",
            "revoke_remote_device",
            "execute_remote_command",
            "retrieve_remote_artifact",
            "inspect_capability_bundle",
            "list_capability_bundles",
            "activate_capability_bundle",
            "disable_capability_bundle",
            "authorize_bundle_capability",
            "refresh_capability_registry",
            "list_capability_registry",
            "ingest_knowledge",
            "install_mod_from_path",
            "inject_taskflow_override",
            "inspect_presentation_template",
            "list_agent_configs",
            "list_artifacts",
            "list_presentation_reviews",
            "list_workbook_reviews",
            "list_channel_configs",
            "list_connector_accounts",
            "list_connector_manifests",
            "list_slack_conversations",
            "list_chat_messages",
            "list_chat_sessions",
            "mark_chat_session_completion_unread",
            "mark_chat_session_read",
            "list_delegation_plans",
            "list_installed_mods",
            "list_local_directory",
            "list_local_models",
            "list_macos_permission_states",
            "list_pending_shield_approvals",
            "list_pending_workflow_approvals",
            "list_provider_configs",
            "list_project_sources",
            "list_projects",
            "list_routines",
            "list_session_scope_trust_grants",
            "list_task_runs",
            "mcp_connect_server",
            "mcp_connect_builtin_server",
            "mcp_cancel_remote_operations",
            "mcp_execute_tool",
            "mcp_get_tool_details",
            "mcp_list_tools",
            "mcp_prepare_tool_approval",
            "mcp_reject_tool_approval",
            "mcp_search_tools",
            "mcp_builtin_server_configs",
            "parse_intent",
            "open_authorized_native_browser",
            "open_calendar_privacy_settings",
            "open_external_http_url",
            "open_macos_permission_settings",
            "open_mail_automation_settings",
            "open_oomu_marketplace",
            "open_oomu_privacy_policy",
            "open_presentation_checker_download",
            "observe_app_control_session",
            "prepare_agent_execution_replan",
            "prepare_system_apple_app_tool_approval",
            "preview_project_deletion",
            "propose_routine",
            "project_policy_preflight",
            "process_agent_objective",
            "process_objective",
            "request_macos_permission",
            "queue_message",
            "reconcile_volatile_persistence",
            "retry_sovereign_identity_health",
            "retry_routine_delivery",
            "recheck_presentation_revision",
            "revise_artifact",
            "revise_presentation_scope",
            "revise_workbook_range",
            "review_and_execute_app_control_action",
            "reconcile_task_runs",
            "reconnect_task_events",
            "read_system_calendar",
            "read_system_emails",
            "read_system_photos",
            "read_system_music",
            "read_local_context",
            "recall_global_memory",
            "recycle_autonomic_helper",
            "record_browser_chat_turn",
            "recover_local_inference",
            "reload_native_browser",
            "rename_chat_session",
            "remove_knowledge_document",
            "reset_sovereign_ledger_stats",
            "reveal_workflow_output_file",
            "resume_agent_execution",
            "restore_agent_sessions",
            "restore_persistence_migration_backup",
            "resize_native_browser",
            "resolve_agent_calendar_recovery",
            "resolve_task_effect_verification",
            "resolve_workflow_permission",
            "revoke_session_scope_trust_grant",
            "revoke_sovereign_trust_policy",
            "revoke_sovereign_trust_session",
            "revoke_actuation_lease",
            "request_native_authority",
            "revoke_local_context_grants",
            "revoke_project_source",
            "revoke_reviewed_approval_scope",
            "run_memory_comparative_audit",
            "run_network_diagnostic",
            "run_pre_alpha_audit",
            "run_system_diagnostics",
            "run_workflow",
            "reserve_task_effect",
            "resume_task_run",
            "resume_routine",
            "retry_delegation_child",
            "pause_delegation_plan",
            "resume_delegation_plan",
            "list_work_suggestions",
            "review_work_suggestion",
            "run_project_data_analysis",
            "list_task_analyses",
            "prepare_learning_offer",
            "list_learning_offers",
            "review_learning_offer",
            "list_saved_methods",
            "set_saved_method_enabled",
            "forget_saved_method",
            "undo_forget_saved_method",
            "go_back_saved_method",
            "edit_saved_method",
            "export_saved_method",
            "retry_task_run",
            "save_agent_config",
            "save_agentic_state",
            "save_channel_config",
            "save_provider_config",
            "save_routing_preference",
            "save_setup_progress",
            "save_session_context_policy",
            "save_session_config",
            "stage_chat_session_deletion",
            "save_user_personality_profile",
            "save_workflow",
            "scan_agent_import_directory",
            "scrape_active_page_content",
            "set_active_locale",
            "set_application_update_ui_ready",
            "set_automated_web_grounding_enabled",
            "set_default_prewarmed_model",
            "set_mod_active_state",
            "set_routing_preference",
            "set_project_instructions",
            "set_project_policy",
            "set_connector_project_scope",
            "set_background_service_enabled",
            "record_application_update_decision",
            "install_pending_application_update",
            "restart_after_application_update",
            "open_application_update_release_notes",
            "open_background_login_items_settings",
            "cancel_sovereign_search",
            "continue_browser_research_headlessly",
            "sovereign_duckduckgo_search",
            "spawn_agent_execution",
            "spawn_agent_session",
            "start_voice_capture",
            "stream_dom_to_context",
            "start_taskflow_monitor",
            "start_browser_automation",
            "start_app_control_session",
            "start_recommended_model_install",
            "stream_execution_steps",
            "stream_native_inference",
            "stop_voice_capture",
            "test_connector",
            "triage_local_app_intent",
            "subagent_yield",
            "sync_provider_models",
            "update_agent_configuration",
            "upsert_sovereign_trust_policy",
            "update_agent_soul_manifest",
            "update_project",
            "update_routine",
            "update_chat_session_dynamic_routing_override",
            "update_chat_session_web_grounding_override",
            "update_compiled_instruction",
            "update_workflow_last_run",
            "undo_chat_session_deletion",
            "unbind_mod_to_agent",
            "uninstall_mod",
            "validate_mod_compatibility_for_turn",
            "verify_artifact_signature",
            "verify_task_effect",
            "disconnect_connector",
            "duplicate_routine",
            "pause_routine",
            "run_routine_now",
            "run_setup_sample_task",
            "get_setup_state",
            "get_recommended_model_install_state",
            "get_private_egress_confirmation",
            "archive_project",
            "attach_project_source",
            "bind_project_record",
            "acknowledge_task_failure",
            "refresh_project_source",
            "resolve_private_egress_confirmation",
        ]),
    ))
    .expect("failed to build Tauri app manifest")
}

fn publish_updater_public_key() {
    const DEVELOPMENT_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEVDNTdGQzNFMUNGRkUxQzQKUldURTRmOGNQdnhYN055YjlpWEVscnNSdWhKN3B5cS9WdVRsN1RLOVNSNm16QUhGQzRmN0RsVXIK";
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let configured = env::var("OOMU_UPDATER_PUBLIC_KEY").ok();
    let public_key = if profile == "release" {
        configured
            .as_deref()
            .filter(|value| {
                let value = value.trim();
                !value.is_empty() && value != DEVELOPMENT_PUBLIC_KEY && value.len() <= 4096
            })
            .unwrap_or_else(|| {
                panic!("Release builds require a dedicated OOMU_UPDATER_PUBLIC_KEY that is not the development key")
            })
    } else {
        configured
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(DEVELOPMENT_PUBLIC_KEY)
    };
    println!(
        "cargo:rustc-env=OOMU_UPDATER_PUBLIC_KEY={}",
        public_key.trim()
    );
}

fn publish_release_version() {
    let path = Path::new("../release/version.json");
    println!("cargo:rerun-if-changed={}", path.display());
    let bytes = fs::read(path).unwrap_or_else(|error| {
        panic!(
            "failed to read authoritative release version {}: {error}",
            path.display()
        )
    });
    let record: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "failed to parse authoritative release version {}: {error}",
            path.display()
        )
    });
    let public_label = record
        .get("publicLabel")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            panic!(
                "authoritative release version {} has no publicLabel",
                path.display()
            )
        });
    let build_number = record
        .get("buildNumber")
        .and_then(serde_json::Value::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            panic!(
                "authoritative release version {} has no valid buildNumber",
                path.display()
            )
        });
    println!("cargo:rustc-env=OOMU_RELEASE_VERSION={public_label}");
    println!("cargo:rustc-env=OOMU_RELEASE_BUILD_NUMBER={build_number}");
}

fn publish_frontend_export_fingerprint() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let frontend = manifest.join("..").join("out");
    let profile = validated_build_profile();
    println!("cargo:rerun-if-changed={}", frontend.display());

    match fs::symlink_metadata(&frontend) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            println!(
                "cargo:warning=OOMU frontend export is not a real directory at {}",
                frontend.display()
            );
            std::process::exit(1);
        }
        Err(error) => {
            match build_identity_policy::missing_frontend_export_identity(&profile, error.kind()) {
                Ok(identity) => {
                    println!(
                        "cargo:rustc-env=OOMU_FRONTEND_EXPORT_SHA256={}",
                        identity.digest
                    );
                    println!(
                        "cargo:rustc-env=OOMU_FRONTEND_EXPORT_FILE_COUNT={}",
                        identity.file_count
                    );
                    return;
                }
                Err(policy_error) => {
                    println!(
                        "cargo:warning=OOMU frontend export is unavailable at {}: {error}; {policy_error}",
                        frontend.display()
                    );
                    std::process::exit(1);
                }
            }
        }
    }

    let mut files = Vec::new();
    collect_frontend_files(&frontend, &mut files);
    files.sort();

    let mut hasher = Sha256::new();
    for path in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        let relative = path.strip_prefix(&frontend).unwrap_or(path);
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        match fs::read(path) {
            Ok(bytes) => hasher.update(bytes),
            Err(error) => {
                println!(
                    "cargo:warning=OOMU frontend export is unreadable at {}: {error}",
                    path.display()
                );
                std::process::exit(1);
            }
        }
        hasher.update([0]);
    }

    let digest = hex::encode(hasher.finalize());
    println!("cargo:rustc-env=OOMU_FRONTEND_EXPORT_SHA256={digest}");
    println!(
        "cargo:rustc-env=OOMU_FRONTEND_EXPORT_FILE_COUNT={}",
        files.len()
    );
}

fn collect_frontend_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            println!(
                "cargo:warning=OOMU frontend export is missing at {}: {error}",
                directory.display()
            );
            std::process::exit(1);
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                println!("cargo:warning=OOMU frontend export could not be enumerated: {error}");
                std::process::exit(1);
            }
        };
        let path = entry.path();
        if path.is_dir() {
            collect_frontend_files(&path, files);
        } else if path.is_file() {
            files.push(path);
        }
    }
}

fn publish_source_identity() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let root = manifest.parent().unwrap_or(&manifest);
    let profile = validated_build_profile();
    let mut files = Vec::new();
    let mut missing_directory_markers = Vec::new();
    collect_source_identity_files(&root.join("src"), &mut files);
    collect_source_identity_files(&manifest.join("src"), &mut files);
    for directory in PACKAGE_IDENTITY_DIRECTORIES {
        let path = root.join(directory);
        println!("cargo:rerun-if-changed={}", path.display());
        collect_package_identity_files(
            &path,
            root,
            &profile,
            &mut files,
            &mut missing_directory_markers,
        );
    }
    files.extend([
        root.join("package.json"),
        root.join("package-lock.json"),
        manifest.join("Cargo.toml"),
        manifest.join("Cargo.lock"),
        manifest.join("build.rs"),
    ]);
    files.extend(PACKAGE_IDENTITY_FILES.iter().map(|path| root.join(path)));
    files.sort_by_key(|path| {
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    });
    files.dedup();
    missing_directory_markers.sort();
    missing_directory_markers.dedup();

    let mut hasher = Sha256::new();
    for path in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        let relative = path.strip_prefix(root).unwrap_or(path);
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(path).unwrap_or_else(|error| {
            panic!(
                "OOMU source identity could not read {}: {error}",
                path.display()
            )
        }));
        hasher.update([0]);
    }
    for marker in missing_directory_markers {
        hasher.update(marker.as_bytes());
        hasher.update([0]);
    }

    println!(
        "cargo:rustc-env=OOMU_BUILD_SOURCE_SHA256={}",
        hex::encode(hasher.finalize())
    );
    println!(
        "cargo:rustc-env=OOMU_BUILD_SOURCE_FILE_COUNT={}",
        files.len()
    );
}

fn collect_package_identity_files(
    directory: &Path,
    root: &Path,
    profile: &str,
    files: &mut Vec<PathBuf>,
    missing_directory_markers: &mut Vec<String>,
) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            let relative = directory.strip_prefix(root).unwrap_or(directory);
            match build_identity_policy::missing_generated_directory_marker(
                profile,
                &relative.to_string_lossy(),
                error.kind(),
            ) {
                Ok(marker) => {
                    missing_directory_markers.push(marker);
                    return;
                }
                Err(policy_error) => panic!(
                    "OOMU package identity could not enumerate {}: {error}; {policy_error}",
                    directory.display()
                ),
            }
        }
    };
    for entry in entries {
        let path = entry.expect("OOMU package identity entry").path();
        if path.is_dir() {
            collect_package_identity_files(&path, root, profile, files, missing_directory_markers);
        } else if path.is_file()
            && path.file_name().and_then(|name| name.to_str()) != Some(".DS_Store")
            && path.file_name().and_then(|name| name.to_str()) != Some("google-desktop-client.json")
        {
            files.push(path);
        }
    }
}

fn validated_build_profile() -> String {
    let profile = env::var("PROFILE").unwrap_or_default();
    build_identity_policy::validate_profile(&profile)
        .unwrap_or_else(|error| panic!("OOMU package identity rejected build profile: {error}"));
    profile
}

fn collect_source_identity_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(directory).unwrap_or_else(|error| {
        panic!(
            "OOMU source identity could not enumerate {}: {error}",
            directory.display()
        )
    });
    for entry in entries {
        let path = entry.expect("OOMU source identity entry").path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if !matches!(name, "target" | ".next" | "node_modules") {
                collect_source_identity_files(&path, files);
            }
        } else if path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(
                        extension,
                        "rs" | "ts"
                            | "tsx"
                            | "js"
                            | "mjs"
                            | "json"
                            | "toml"
                            | "swift"
                            | "css"
                            | "scss"
                            | "sass"
                            | "svg"
                            | "png"
                            | "jpg"
                            | "jpeg"
                            | "webp"
                            | "gif"
                            | "ico"
                            | "icns"
                            | "avif"
                            | "woff"
                            | "woff2"
                            | "ttf"
                            | "otf"
                    )
                })
        {
            files.push(path);
        }
    }
}

fn publish_artifact_helper_digest() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let target = env::var("TARGET").unwrap_or_default();
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let path = manifest
        .join("binaries")
        .join(format!("artifact_build_helper-{target}{suffix}"));
    println!("cargo:rerun-if-changed={}", path.display());
    let digest = fs::read(&path)
        .ok()
        .filter(|bytes| bytes.len() > 4)
        .map(|bytes| hex::encode(Sha256::digest(bytes)))
        .unwrap_or_else(|| "unprepared".to_string());
    println!("cargo:rustc-env=OOMU_ARTIFACT_HELPER_SHA256={digest}");
}

fn publish_artifact_pdf_helper_digest() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let target = env::var("TARGET").unwrap_or_default();
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let path = manifest
        .join("binaries")
        .join(format!("oomu-artifact-pdf-helper-{target}{suffix}"));
    println!("cargo:rerun-if-changed={}", path.display());
    let digest = fs::read(&path)
        .ok()
        .filter(|bytes| bytes.len() > 4)
        .map(|bytes| hex::encode(Sha256::digest(bytes)))
        .unwrap_or_else(|| "unprepared".to_string());
    println!("cargo:rustc-env=OOMU_ARTIFACT_PDF_HELPER_SHA256={digest}");
}

fn configured_oauth_client_id(environment_name: &str, file_name: &str) -> Option<String> {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let reviewed = fs::read_to_string(manifest.join("oauth").join(file_name))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if env::var("PROFILE").as_deref() == Ok("release") {
        return reviewed;
    }
    env::var(environment_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or(reviewed)
}

fn valid_public_oauth_client_id(value: &str) -> bool {
    (8..=256).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn valid_google_oauth_client_secret(value: &str) -> bool {
    (8..=512).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn configured_google_oauth_client_secret() -> Result<Option<String>, String> {
    let environment_secret = env::var("OOMU_GOOGLE_OAUTH_CLIENT_SECRET")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if environment_secret.is_some() {
        return Ok(environment_secret);
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let path = manifest.join("oauth/google-desktop-client.json");
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Google desktop OAuth credential file is unreadable".to_string()),
    };
    let document: Value = serde_json::from_str(&raw)
        .map_err(|_| "Google desktop OAuth credential file is invalid JSON".to_string())?;
    let installed = document
        .get("installed")
        .and_then(Value::as_object)
        .ok_or_else(|| "Google OAuth credentials are not a Desktop app client".to_string())?;
    let file_client_id = installed
        .get("client_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let reviewed_client_id = configured_oauth_client_id(
        "OOMU_GOOGLE_OAUTH_CLIENT_ID",
        "google-desktop-client-id.txt",
    )
    .unwrap_or_default();
    if file_client_id != reviewed_client_id {
        return Err(
            "Google desktop OAuth credential does not match the reviewed client identity"
                .to_string(),
        );
    }
    Ok(installed
        .get("client_secret")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn publish_google_oauth_client_credential() {
    let credential = configured_google_oauth_client_secret().unwrap_or_else(|error| {
        println!("cargo:warning=OOMU Google OAuth configuration error: {error}");
        std::process::exit(1);
    });
    if credential
        .as_deref()
        .is_some_and(|value| !valid_google_oauth_client_secret(value))
    {
        println!("cargo:warning=OOMU Google OAuth client credential is malformed");
        std::process::exit(1);
    }
    let encoded = credential
        .as_deref()
        .map(serde_json::to_string)
        .transpose()
        .expect("Google OAuth client credential could not be encoded");
    let source = match encoded {
        Some(value) => format!("const GOOGLE_CLIENT_SECRET: Option<&str> = Some({value});\n"),
        None => "const GOOGLE_CLIENT_SECRET: Option<&str> = None;\n".to_string(),
    };
    let path = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is required"))
        .join("google_oauth_client_credential.rs");
    fs::write(&path, source).expect("Google OAuth client credential source could not be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("Google OAuth client credential source permissions could not be restricted");
    }
}

fn configured_public_oauth_value(environment_name: &str, file_name: &str) -> Option<String> {
    env::var(environment_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
            fs::read_to_string(manifest.join("oauth").join(file_name))
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
}

fn valid_slack_broker_url(value: &str) -> bool {
    value.starts_with("https://")
        && value.len() <= 512
        && !value.contains('@')
        && !value.contains('#')
}

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn configured_slack_redirect_port() -> Option<String> {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let reviewed = fs::read_to_string(manifest.join("oauth/slack-redirect-port.txt"))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if env::var("PROFILE").as_deref() == Ok("release") {
        return reviewed;
    }
    env::var("OOMU_SLACK_OAUTH_REDIRECT_PORT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or(reviewed)
}

fn publish_oauth_client_identities() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    for (environment_name, file_name) in [
        (
            "OOMU_GOOGLE_OAUTH_CLIENT_ID",
            "google-desktop-client-id.txt",
        ),
        ("OOMU_SLACK_OAUTH_CLIENT_ID", "slack-pkce-client-id.txt"),
        (
            "OOMU_MICROSOFT_OAUTH_CLIENT_ID",
            "microsoft-public-client-id.txt",
        ),
    ] {
        let path = manifest.join("oauth").join(file_name);
        println!("cargo:rerun-if-changed={}", path.display());
        if let Some(value) = configured_oauth_client_id(environment_name, file_name) {
            if !valid_public_oauth_client_id(&value) {
                println!(
                    "cargo:warning=OOMU OAuth client identity is malformed: {environment_name}"
                );
                std::process::exit(1);
            }
            println!("cargo:rustc-env={environment_name}={value}");
        }
    }
    let google_credentials_path = manifest.join("oauth/google-desktop-client.json");
    println!(
        "cargo:rerun-if-changed={}",
        google_credentials_path.display()
    );
    publish_google_oauth_client_credential();
    let redirect_port_path = manifest.join("oauth/slack-redirect-port.txt");
    println!("cargo:rerun-if-changed={}", redirect_port_path.display());
    if let Some(port) = configured_slack_redirect_port() {
        if port.parse::<u16>().ok().is_none_or(|value| value < 1024) {
            println!("cargo:warning=OOMU Slack OAuth redirect port is malformed");
            std::process::exit(1);
        }
        println!("cargo:rustc-env=OOMU_SLACK_OAUTH_REDIRECT_PORT={port}");
    }
    for (environment_name, file_name, validator) in [
        (
            "OOMU_SLACK_OAUTH_BROKER_URL",
            "slack-broker-url.txt",
            valid_slack_broker_url as fn(&str) -> bool,
        ),
        (
            "OOMU_SLACK_OAUTH_BROKER_CERT_SHA256",
            "slack-broker-certificate-sha256.txt",
            valid_sha256_hex as fn(&str) -> bool,
        ),
    ] {
        let path = manifest.join("oauth").join(file_name);
        println!("cargo:rerun-if-changed={}", path.display());
        if let Some(value) = configured_public_oauth_value(environment_name, file_name) {
            if !validator(&value) {
                println!("cargo:warning={environment_name} is malformed");
                std::process::exit(1);
            }
            println!("cargo:rustc-env={environment_name}={value}");
        }
    }
}

fn verify_release_entrypoint() {
    if env::var("PROFILE").as_deref() != Ok("release") {
        return;
    }
    if env::var("OOMU_LOCAL_UNSIGNED_BUILD").as_deref() == Ok("1") {
        println!(
            "cargo:warning=OOMU local unsigned build: canonical release authorization skipped"
        );
        return;
    }
    let unsigned_pipeline = env::var("OOMU_RELEASE_PIPELINE").as_deref() == Ok("unsigned-v2");
    if unsigned_pipeline && !unsigned_release_pipeline_authorized() {
        fail_release_guard("unsigned release pipeline authorization is absent or invalid");
    }
    if !unsigned_pipeline && env::var("OOMU_RELEASE_PIPELINE").as_deref() != Ok("canonical-v1") {
        fail_release_guard("release profile was requested outside the canonical release pipeline");
    }
    for (environment_name, file_name, label) in [
        (
            "OOMU_GOOGLE_OAUTH_CLIENT_ID",
            "google-desktop-client-id.txt",
            "Google OAuth client identity",
        ),
        (
            "OOMU_SLACK_OAUTH_CLIENT_ID",
            "slack-pkce-client-id.txt",
            "Slack OAuth client identity",
        ),
    ] {
        let value = configured_oauth_client_id(environment_name, file_name).unwrap_or_default();
        if !valid_public_oauth_client_id(&value) {
            fail_release_guard(&format!("{label} is absent or malformed"));
        }
    }
    if configured_google_oauth_client_secret()
        .ok()
        .flatten()
        .as_deref()
        .is_none_or(|value| !valid_google_oauth_client_secret(value))
    {
        fail_release_guard("Google OAuth desktop client credential is absent or malformed");
    }
    if configured_slack_redirect_port()
        .and_then(|value| value.parse::<u16>().ok())
        .is_none_or(|port| port < 1024)
    {
        fail_release_guard("Slack OAuth redirect port is absent or malformed");
    }
    let slack_broker_url =
        configured_public_oauth_value("OOMU_SLACK_OAUTH_BROKER_URL", "slack-broker-url.txt");
    let slack_broker_pin = configured_public_oauth_value(
        "OOMU_SLACK_OAUTH_BROKER_CERT_SHA256",
        "slack-broker-certificate-sha256.txt",
    );
    match (slack_broker_url.as_deref(), slack_broker_pin.as_deref()) {
        (None, None) => {}
        (Some(url), Some(pin)) if valid_slack_broker_url(url) && valid_sha256_hex(pin) => {}
        _ => {
            fail_release_guard(
                "Slack messaging service configuration must be absent or complete and valid",
            );
        }
    }
    let build_id = env::var("OOMU_BUILD_ID").unwrap_or_default();
    if !(8..=128).contains(&build_id.len())
        || !build_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        fail_release_guard("OOMU_BUILD_ID is absent or malformed");
    }
    let source_revision = env::var("OOMU_SOURCE_REVISION").unwrap_or_default();
    if source_revision.len() != 40 || !source_revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        fail_release_guard("OOMU_SOURCE_REVISION is not a full Git revision");
    }
    let actual_revision = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string());
    if actual_revision.as_deref() != Some(source_revision.as_str()) {
        fail_release_guard("OOMU_SOURCE_REVISION does not match the checked-out source tree");
    }
    if !unsigned_pipeline {
        let public_key_bytes = hex::decode(TRUSTED_RELEASE_PUBLIC_KEY_HEX)
            .ok()
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok());
        let signature = env::var("OOMU_RELEASE_AUTHORIZATION_BASE64")
            .ok()
            .and_then(|value| BASE64_STANDARD.decode(value.as_bytes()).ok())
            .and_then(|bytes| Signature::from_slice(&bytes).ok());
        let payload = format!("oomu-release-v1\n{build_id}\n{source_revision}");
        let authorized = public_key_bytes
            .and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
            .zip(signature)
            .is_some_and(|(key, signature)| key.verify(payload.as_bytes(), &signature).is_ok());
        if !authorized {
            fail_release_guard("canonical release authorization signature is absent or invalid");
        }
    }
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let shadow_dir = manifest_dir.join("..").join("scripts").join("bin");
    let path_contains_shadow = env::var_os("PATH").is_some_and(|path| {
        env::split_paths(&path).any(|entry| {
            entry == shadow_dir
                || (entry.exists()
                    && shadow_dir.exists()
                    && entry.canonicalize().ok() == shadow_dir.canonicalize().ok())
        })
    });
    if path_contains_shadow {
        fail_release_guard("repository-local scripts/bin is present on the production PATH");
    }
}

fn unsigned_release_pipeline_authorized() -> bool {
    if env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
        return false;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let policy_path = manifest
        .join("..")
        .join("release")
        .join("release-policy.json");
    let actual_digest = fs::read(policy_path)
        .ok()
        .map(|bytes| hex::encode(Sha256::digest(bytes)));
    actual_digest.as_deref() == env::var("OOMU_RELEASE_POLICY_SHA256").ok().as_deref()
}

fn fail_release_guard(message: &str) -> ! {
    println!("cargo:warning=OOMU RELEASE GUARD: {message}.");
    println!("cargo:warning=Run `npm run build:prod` to create a distributable candidate.");
    std::process::exit(1)
}

fn verify_build_toolchain() {
    match Command::new("cmake").arg("--version").output() {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let detail = String::from_utf8_lossy(&output.stderr);
            warn_missing_cmake(Some(detail.trim()));
            std::process::exit(1);
        }
        Err(_) => {
            warn_missing_cmake(None);
            std::process::exit(1);
        }
    }
}

fn warn_missing_cmake(detail: Option<&str>) {
    println!("cargo:warning=OOMU BUILD FAILURE: 'cmake' was not found in the system PATH.");
    println!(
        "cargo:warning=The local inference engine (llama-cpp-sys-2) requires CMake to compile."
    );
    println!("cargo:warning=Install CMake with Homebrew using 'brew install cmake', then run the build again.");
    if let Some(detail) = detail.filter(|value| !value.is_empty()) {
        println!("cargo:warning=cmake check detail: {detail}");
    }
}
