use crate::tools::eventkit_calendar::{CalendarAuthorizationDisposition, CalendarFullAccessStatus};

#[cfg(target_os = "macos")]
use tauri_plugin_opener::OpenerExt;

#[cfg(target_os = "macos")]
const CALENDAR_PRIVACY_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars";

#[tauri::command]
pub async fn check_calendar_full_access() -> CalendarFullAccessStatus {
    let initial = crate::tools::eventkit_calendar::calendar_full_access_status().await;
    if should_request_calendar_full_access(initial.status) {
        // This command is invoked only from the user's explicit recovery action.
        // NotDetermined and WriteOnly are the two macOS states where EventKit can
        // still present the native Full Access request instead of dead-ending.
        let _ = crate::tools::eventkit_calendar::ensure_full_calendar_access().await;
        return crate::tools::eventkit_calendar::calendar_full_access_status().await;
    }
    initial
}

fn should_request_calendar_full_access(status: CalendarAuthorizationDisposition) -> bool {
    matches!(
        status,
        CalendarAuthorizationDisposition::NotDetermined
            | CalendarAuthorizationDisposition::WriteOnly
    )
}

#[tauri::command]
pub fn open_calendar_privacy_settings(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        app.opener()
            .open_url(CALENDAR_PRIVACY_SETTINGS_URL, None::<&str>)
            .map_err(|error| {
                eprintln!("OOMU_CALENDAR_PRIVACY_SETTINGS_OPEN_FAILED error={error}");
                "OOMU couldn’t open Calendar privacy settings. Open System Settings, then choose Privacy & Security and Calendars."
                    .to_string()
            })
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Calendar privacy settings are available only on macOS.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::eventkit_calendar::CalendarAuthorizationDisposition;

    #[test]
    fn recovery_requests_only_native_requestable_calendar_states() {
        for (disposition, should_request) in [
            (CalendarAuthorizationDisposition::FullAccess, false),
            (CalendarAuthorizationDisposition::NotDetermined, true),
            (CalendarAuthorizationDisposition::WriteOnly, true),
            (CalendarAuthorizationDisposition::Denied, false),
            (CalendarAuthorizationDisposition::Restricted, false),
            (CalendarAuthorizationDisposition::Unavailable, false),
        ] {
            assert_eq!(
                super::should_request_calendar_full_access(disposition),
                should_request,
                "{disposition:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn calendar_privacy_repair_opens_the_exact_system_settings_pane() {
        assert_eq!(
            super::CALENDAR_PRIVACY_SETTINGS_URL,
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars"
        );
    }
}
