//! Debug-only, opt-in UI actuation for the manual Scenario 1 acceptance run.
//!
//! The release-build callback is inert; all driver code and its injected script
//! compile only under debug assertions. The script interacts with rendered
//! controls and has no Tauri command bridge to the execution backend.

#[cfg(debug_assertions)]
use std::ffi::OsStr;

#[cfg(debug_assertions)]
use tauri::Manager;

#[cfg(debug_assertions)]
const ENABLE_ENV: &str = "OOMU_SCENARIO_ONE_E2E";
#[cfg(debug_assertions)]
const DISABLE_DRIVER_ENV: &str = "OOMU_SCENARIO_ONE_E2E_NO_UI_DRIVER";
#[cfg(debug_assertions)]
const DRIVER_SCRIPT: &str = include_str!("scenario_one_ui_driver.js");

#[cfg(debug_assertions)]
fn activation_value_is_enabled(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

#[cfg(debug_assertions)]
fn driver_is_explicitly_disabled(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

pub(crate) fn on_page_load(
    webview: &tauri::Webview,
    payload: &tauri::webview::PageLoadPayload<'_>,
) {
    #[cfg(not(debug_assertions))]
    let _ = (webview, payload);

    #[cfg(debug_assertions)]
    install_after_main_page_load(webview, payload);
}

#[cfg(debug_assertions)]
fn install_after_main_page_load(
    webview: &tauri::Webview,
    payload: &tauri::webview::PageLoadPayload<'_>,
) {
    if payload.event() != tauri::webview::PageLoadEvent::Finished {
        return;
    }
    if !activation_value_is_enabled(std::env::var_os(ENABLE_ENV).as_deref()) {
        return;
    }
    // The isolated encrypted profile is also useful for manually qualifying
    // other real workflows without touching Keychain. Keep Scenario 1's DOM
    // driver on by default, but allow an explicit debug-only opt-out so it
    // cannot race or mutate unrelated acceptance runs.
    if driver_is_explicitly_disabled(std::env::var_os(DISABLE_DRIVER_ENV).as_deref()) {
        eprintln!("OOMU_SCENARIO_ONE_E2E_TRACE stage=install status=driver_disabled");
        return;
    }
    if webview.label() != "main" {
        return;
    }

    let host_window = webview.window();
    let Some(window) = host_window.get_webview_window(webview.label()) else {
        eprintln!("OOMU_SCENARIO_ONE_E2E_TRACE stage=install status=missing_main_window");
        return;
    };

    eprintln!("OOMU_SCENARIO_ONE_E2E_TRACE stage=install status=enabled");
    if let Err(error) = window.eval(DRIVER_SCRIPT) {
        eprintln!(
            "OOMU_SCENARIO_ONE_E2E_TRACE stage=install status=eval_failed error={}",
            crate::redaction::redacted_log_text(&error.to_string())
        );
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    fn activation_requires_the_exact_explicit_value() {
        assert!(activation_value_is_enabled(Some(OsStr::new("1"))));
        for value in [None, Some(OsStr::new("0")), Some(OsStr::new("true"))] {
            assert!(!activation_value_is_enabled(value));
        }
    }

    #[test]
    fn driver_opt_out_requires_the_exact_explicit_value() {
        assert!(driver_is_explicitly_disabled(Some(OsStr::new("1"))));
        for value in [None, Some(OsStr::new("0")), Some(OsStr::new("true"))] {
            assert!(!driver_is_explicitly_disabled(value));
        }
    }

    #[test]
    fn driver_is_dom_only_and_scope_bound() {
        for required in [
            "ship_test_01",
            "OOMU Test",
            "recipient@example.com",
            "Auto-route",
            "gemma-4-E4B-it-qat-q4_0-gguf",
            "data-route-mode",
            "data-auto-route-status",
            "data-auto-route-choice",
            "data-cloud-model-id",
            "data-agent-execution-status",
            "data-oomu-plan-approval",
            "data-oomu-approval-detail",
            "data-oomu-native-approval-status",
            "data-oomu-calendar-recovery-code",
            "data-oomu-calendar-recovery-action",
            "data-oomu-calendar-recovery",
            "data-calendar-recovery-requested",
            "data-calendar-recovery-action",
            "data-oomu-calendar-name",
            "data-oomu-mail-recovery-code",
            "data-oomu-mail-recovery-action",
            "data-calendar-permission-action",
            "data-mail-automation-action",
            "data-setup-journey",
            "data-setup-action",
            "querySelector",
            ".click()",
            "dispatchEvent",
        ] {
            assert!(DRIVER_SCRIPT.contains(required), "missing {required}");
        }
        for forbidden in [
            "__TAURI__",
            "invoke(",
            "execute_action_plan",
            "resolve_shield",
        ] {
            assert!(
                !DRIVER_SCRIPT.contains(forbidden),
                "driver must not contain backend bridge {forbidden}"
            );
        }
    }

    #[test]
    fn driver_recovers_only_the_exact_missing_oomu_test_calendar_once() {
        for required in [
            "calendar_not_found",
            "resolve_calendar_target",
            "OOMU Test",
            "create-requested",
            "calendarRecoveryHandled",
            "calendar_recovery_resume",
            "same_execution_resumed",
        ] {
            assert!(DRIVER_SCRIPT.contains(required), "missing {required}");
        }
        assert!(DRIVER_SCRIPT.contains("if (status === \"halted\")"));
        assert!(DRIVER_SCRIPT.contains("const createCalendar = !calendarRecoveryHandled"));
        assert!(DRIVER_SCRIPT.contains("if (status === \"failed\" || status === \"halted\")"));
    }

    #[test]
    fn driver_waits_boundedly_and_rechecks_only_exact_permission_cards() {
        for required in [
            "PERMISSION_RECOVERY_TIMEOUT_MS = 600_000",
            "PERMISSION_RECOVERY_RETRY_INTERVAL_MS = 5_000",
            "RECOVERY_CARD_RENDER_TIMEOUT_MS = 120_000",
            "restore_calendar_full_access",
            "calendar_authorization_timeout",
            "calendar_permission_denied",
            "calendar_permission_restricted",
            "calendar_permission_unavailable",
            "calendar_permission_write_only",
            "restore_mail_automation_access",
            "mail_automation_permission_required",
            "data-calendar-permission-action=\"check_and_continue\"",
            "data-mail-automation-action=\"check_and_continue\"",
            "candidate.dataset.oomuRecoveryExecutionId === executionId",
            "waitForPermissionGrantAndResume",
            "waiting_for_user_grant",
            "read_only_check_requested",
            "grant_still_pending",
            "same_execution_resumed",
            "user_grant_timeout",
        ] {
            assert!(DRIVER_SCRIPT.contains(required), "missing {required}");
        }
        assert!(DRIVER_SCRIPT.contains(
            "if (recovery?.kind === \"calendar_full_access\" || recovery?.kind === \"mail_automation\")"
        ));
        assert!(DRIVER_SCRIPT.contains("candidate.dataset.agentExecutionId === executionId"));
    }

    #[test]
    fn permission_wait_never_opens_settings_cancels_or_retries_a_different_error() {
        for forbidden in [
            "data-calendar-permission-action=\"open_settings\"",
            "data-calendar-permission-action=\"cancel_remaining\"",
            "data-mail-automation-action=\"open_settings\"",
            "data-mail-automation-action=\"cancel_remaining\"",
            "data-mail-automation-action=\"retry_and_continue\"",
            "open_calendar_privacy_settings",
            "open_mail_automation_settings",
            "cancel_agent_execution_remaining_work",
        ] {
            assert!(
                !DRIVER_SCRIPT.contains(forbidden),
                "permission driver must not contain unsafe recovery action {forbidden}"
            );
        }
    }
}
