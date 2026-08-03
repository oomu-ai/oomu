use super::launch_options::parse_launch_options_from;
use crate::*;

#[test]
fn launch_options_parser_handles_debug_safe_dump_and_reset_flags() {
    let options = parse_launch_options_from([
        "--debug",
        "--safe-mode",
        "--first-run-setup",
        "--dump-db",
        "--reset-state",
        "--help",
    ]);

    assert!(options.debug_mode);
    assert!(options.safe_mode);
    assert!(options.first_run_setup);
    assert!(options.dump_db);
    assert!(options.reset_state);
    assert!(options.show_help);
    assert_eq!(options.log_level, "trace");
}

#[test]
fn launch_options_parser_preserves_explicit_log_level() {
    let options = parse_launch_options_from(["-d", "--log-level", "warn"]);

    assert!(options.debug_mode);
    assert_eq!(options.log_level, "warn");

    let options = parse_launch_options_from(["--log-level=debug", "--unknown", "-s"]);
    assert_eq!(options.log_level, "debug");
    assert!(options.safe_mode);
}

#[test]
fn launch_options_parser_ignores_missing_or_invalid_log_level() {
    let missing = parse_launch_options_from(["--log-level", "--safe-mode"]);
    assert_eq!(missing.log_level, "info");
    assert!(missing.safe_mode);

    let invalid = parse_launch_options_from(["-l", "verbose"]);
    assert_eq!(invalid.log_level, "info");
}

#[test]
fn launch_options_parser_accepts_one_bounded_native_receipt_scope() {
    let digest = "a".repeat(64);
    let options = parse_launch_options_from([
        "--native-acceptance-scope",
        &format!("release-run_302:{digest}"),
    ]);

    let scope = options.native_acceptance_scope.unwrap();
    assert_eq!(scope.run_id, "release-run_302");
    assert_eq!(scope.incident_prompt_sha256, digest);
    assert!(options.native_acceptance_scope_error.is_none());
}

#[test]
fn launch_options_parser_rejects_partial_unsafe_or_duplicate_receipt_scopes() {
    let digest = "b".repeat(64);
    for arguments in [
        vec!["--native-acceptance-scope".to_string()],
        vec![
            "--native-acceptance-scope".to_string(),
            "short:abc".to_string(),
        ],
        vec![
            "--native-acceptance-scope".to_string(),
            format!("unsafe/path:{digest}"),
        ],
        vec![
            "--native-acceptance-scope".to_string(),
            format!("release-run:{digest}"),
            "--native-acceptance-scope".to_string(),
            format!("second-run:{digest}"),
        ],
    ] {
        let options = parse_launch_options_from(arguments);
        assert!(options.native_acceptance_scope.is_none());
        assert!(options.native_acceptance_scope_error.is_some());
    }
}

#[cfg(target_os = "macos")]
#[test]
fn background_tray_copy_uses_only_the_complete_active_locale() {
    use crate::background_runtime_tray::{tray_copy, TrayCopy};

    let translated = serde_json::json!({
        "menu_bar": {
            "background_running": "OOMU arbeitet im Hintergrund",
            "open_oomu": "OOMU öffnen",
            "quit_oomu": "OOMU beenden"
        }
    });
    assert_eq!(
        tray_copy(Some(&translated)).unwrap(),
        TrayCopy::new(
            "OOMU arbeitet im Hintergrund",
            "OOMU öffnen",
            "OOMU beenden",
        )
    );
    assert_eq!(
        tray_copy(None).unwrap_err(),
        "background_menu_language_unavailable"
    );
    assert_eq!(
        tray_copy(Some(
            &serde_json::json!({"menu_bar": {"open_oomu": "Open"}})
        ))
        .unwrap_err(),
        "background_menu_language_unavailable"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn background_tray_copy_is_complete_in_every_shipped_locale() {
    use crate::background_runtime_tray::tray_copy;

    for source in [
        include_str!("../../../src/locales/de-DE.json"),
        include_str!("../../../src/locales/en-US.json"),
        include_str!("../../../src/locales/es-ES.json"),
        include_str!("../../../src/locales/fr-FR.json"),
        include_str!("../../../src/locales/id-ID.json"),
        include_str!("../../../src/locales/ja-JP.json"),
        include_str!("../../../src/locales/pt-BR.json"),
        include_str!("../../../src/locales/ru-RU.json"),
        include_str!("../../../src/locales/uk-UA.json"),
        include_str!("../../../src/locales/vi-VN.json"),
        include_str!("../../../src/locales/zh-CN.json"),
        include_str!("../../../src/locales/zh-TW.json"),
    ] {
        let catalog: serde_json::Value = serde_json::from_str(source).unwrap();
        assert!(tray_copy(Some(&catalog)).is_ok());
    }
}

#[test]
fn database_startup_fallback_activates_degraded_mode_without_panicking() {
    let degraded_mode = DegradedModeState::default();
    let isolated_base = std::env::temp_dir().join(format!(
        "oomu-startup-fallback-test-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let volatile_sessions =
        VolatileStoreSessionManager::initialize_in(isolated_base.clone()).unwrap();

    let value = initialize_database_with_degraded_fallback(
        &degraded_mode,
        &volatile_sessions,
        "chatSessionPersistence",
        "PersistentStateEngine",
        "Chat writes are volatile until recovery succeeds.",
        || Err("database is locked".to_string()),
        |_session| Ok("temporary database"),
    )
    .expect("fallback database initializes");

    let status = degraded_mode.snapshot();
    assert_eq!(value, "temporary database");
    assert!(status.active);
    assert!(status
        .reason
        .as_deref()
        .unwrap_or_default()
        .contains("database is locked"));
    assert!(status.has_volatile_storage);
    let session = volatile_sessions.current().unwrap();
    session
        .record_reconciliation(true, "test cleanup")
        .expect("reconciliation manifest is persisted");
    session.cleanup_after_reconciliation().unwrap();
    let _ = std::fs::remove_dir_all(isolated_base);
}
