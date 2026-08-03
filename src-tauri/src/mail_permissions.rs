use serde::Serialize;

#[cfg(target_os = "macos")]
use std::{ffi::c_void, ptr, time::Duration};
#[cfg(target_os = "macos")]
use tauri_plugin_opener::OpenerExt;

#[cfg(target_os = "macos")]
const MAIL_AUTOMATION_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation";
#[cfg(target_os = "macos")]
const MAIL_BUNDLE_ID: &[u8] = b"com.apple.mail";
#[cfg(target_os = "macos")]
const AUTOMATION_CHECK_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MailAutomationStatus {
    Authorized,
    PermissionRequired,
    TargetNotRunning,
    Unavailable,
    Timeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailAutomationAccessStatus {
    status: MailAutomationStatus,
    authorized: bool,
    retry_supported: bool,
}

impl MailAutomationAccessStatus {
    fn from_status(status: MailAutomationStatus) -> Self {
        Self {
            status,
            authorized: status == MailAutomationStatus::Authorized,
            retry_supported: matches!(
                status,
                MailAutomationStatus::Authorized | MailAutomationStatus::TargetNotRunning
            ),
        }
    }
}

#[tauri::command]
pub async fn check_mail_automation_access() -> MailAutomationAccessStatus {
    #[cfg(target_os = "macos")]
    {
        match tokio::time::timeout(
            AUTOMATION_CHECK_TIMEOUT,
            tauri::async_runtime::spawn_blocking(native_mail_automation_status),
        )
        .await
        {
            Ok(Ok(status)) => MailAutomationAccessStatus::from_status(status),
            Ok(Err(error)) => {
                eprintln!("OOMU_MAIL_AUTOMATION_CHECK_FAILED error={error}");
                MailAutomationAccessStatus::from_status(MailAutomationStatus::Unavailable)
            }
            Err(_) => MailAutomationAccessStatus::from_status(MailAutomationStatus::Timeout),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        MailAutomationAccessStatus::from_status(MailAutomationStatus::Unavailable)
    }
}

#[tauri::command]
pub fn open_mail_automation_settings(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        app.opener()
            .open_url(MAIL_AUTOMATION_SETTINGS_URL, None::<&str>)
            .map_err(|error| {
                eprintln!("OOMU_MAIL_AUTOMATION_SETTINGS_OPEN_FAILED error={error}");
                "OOMU couldn’t open Automation settings. Open System Settings, then choose Privacy & Security and Automation."
                    .to_string()
            })
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Mail Automation settings are available only on macOS.".to_string())
    }
}

#[cfg(target_os = "macos")]
type OSStatus = i32;
#[cfg(target_os = "macos")]
type DescType = u32;

#[cfg(target_os = "macos")]
const NO_ERR: OSStatus = 0;
#[cfg(target_os = "macos")]
const PROC_NOT_FOUND: OSStatus = -600;
#[cfg(target_os = "macos")]
const ERR_AE_EVENT_NOT_PERMITTED: OSStatus = -1743;
#[cfg(target_os = "macos")]
const ERR_AE_EVENT_WOULD_REQUIRE_USER_CONSENT: OSStatus = -1744;
#[cfg(target_os = "macos")]
const TYPE_APPLICATION_BUNDLE_ID: DescType = u32::from_be_bytes(*b"bund");
#[cfg(target_os = "macos")]
const TYPE_WILDCARD: u32 = u32::from_be_bytes(*b"****");

#[cfg(target_os = "macos")]
#[repr(C)]
struct AEDesc {
    descriptor_type: DescType,
    data_handle: *mut c_void,
}

#[cfg(target_os = "macos")]
impl Default for AEDesc {
    fn default() -> Self {
        Self {
            descriptor_type: 0,
            data_handle: ptr::null_mut(),
        }
    }
}

#[cfg(target_os = "macos")]
struct OwnedDesc(AEDesc);

#[cfg(target_os = "macos")]
impl Drop for OwnedDesc {
    fn drop(&mut self) {
        if self.0.descriptor_type != 0 || !self.0.data_handle.is_null() {
            // SAFETY: this wrapper owns the descriptor initialized by AECreateDesc.
            unsafe { AEDisposeDesc(&mut self.0) };
        }
    }
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AECreateDesc(
        descriptor_type: DescType,
        data: *const c_void,
        size: isize,
        result: *mut AEDesc,
    ) -> OSStatus;
    fn AEDeterminePermissionToAutomateTarget(
        target: *const AEDesc,
        event_class: u32,
        event_id: u32,
        ask_user_if_needed: u8,
    ) -> OSStatus;
    fn AEDisposeDesc(descriptor: *mut AEDesc) -> OSStatus;
}

#[cfg(target_os = "macos")]
fn native_mail_automation_status() -> MailAutomationStatus {
    let mut target = OwnedDesc(AEDesc::default());
    // SAFETY: the target descriptor is constructed from the fixed Mail bundle id.
    let create_status = unsafe {
        AECreateDesc(
            TYPE_APPLICATION_BUNDLE_ID,
            MAIL_BUNDLE_ID.as_ptr().cast(),
            MAIL_BUNDLE_ID.len() as isize,
            &mut target.0,
        )
    };
    if create_status != NO_ERR {
        return MailAutomationStatus::Unavailable;
    }

    // SAFETY: the target and event codes are fixed; `false` prevents a surprise prompt.
    let status = unsafe {
        AEDeterminePermissionToAutomateTarget(&target.0, TYPE_WILDCARD, TYPE_WILDCARD, 0)
    };
    status_from_os_status(status)
}

#[cfg(target_os = "macos")]
fn status_from_os_status(status: OSStatus) -> MailAutomationStatus {
    match status {
        NO_ERR => MailAutomationStatus::Authorized,
        ERR_AE_EVENT_NOT_PERMITTED | ERR_AE_EVENT_WOULD_REQUIRE_USER_CONSENT => {
            MailAutomationStatus::PermissionRequired
        }
        PROC_NOT_FOUND => MailAutomationStatus::TargetNotRunning,
        _ => MailAutomationStatus::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_contract_never_prompts_or_overstates_access() {
        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                status_from_os_status(NO_ERR),
                MailAutomationStatus::Authorized
            );
            assert_eq!(
                status_from_os_status(ERR_AE_EVENT_NOT_PERMITTED),
                MailAutomationStatus::PermissionRequired
            );
            assert_eq!(
                status_from_os_status(ERR_AE_EVENT_WOULD_REQUIRE_USER_CONSENT),
                MailAutomationStatus::PermissionRequired
            );
            assert_eq!(
                status_from_os_status(PROC_NOT_FOUND),
                MailAutomationStatus::TargetNotRunning
            );
            assert_eq!(status_from_os_status(-1), MailAutomationStatus::Unavailable);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn recovery_is_fixed_to_mail_and_the_exact_automation_pane() {
        assert_eq!(MAIL_BUNDLE_ID, b"com.apple.mail");
        assert_eq!(TYPE_WILDCARD, u32::from_be_bytes(*b"****"));
        assert_eq!(
            MAIL_AUTOMATION_SETTINGS_URL,
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation"
        );
    }

    #[test]
    fn only_authorized_or_not_running_targets_can_retry_the_exact_checkpoint() {
        for status in [
            MailAutomationStatus::PermissionRequired,
            MailAutomationStatus::Unavailable,
            MailAutomationStatus::Timeout,
        ] {
            let response = MailAutomationAccessStatus::from_status(status);
            assert!(!response.authorized);
            assert!(!response.retry_supported);
        }
        assert!(
            MailAutomationAccessStatus::from_status(MailAutomationStatus::Authorized)
                .retry_supported
        );
        assert!(
            MailAutomationAccessStatus::from_status(MailAutomationStatus::TargetNotRunning)
                .retry_supported
        );
    }
}
