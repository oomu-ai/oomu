use crate::foundation::clock::unix_time_ms_i64;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

#[path = "macos_permission_broker/contract.rs"]
mod contract;

static NEXT_PERMISSION_ATTEMPT: AtomicU64 = AtomicU64::new(1);

#[cfg(target_os = "macos")]
use tauri_plugin_opener::OpenerExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MacosPermissionState {
    NotRequested,
    Allowed,
    Limited,
    Denied,
    Restricted,
    RequiresSettings,
    Stale,
    WhenUsed,
    Unsupported,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacosPermissionStatus {
    pub capability_id: String,
    pub state: MacosPermissionState,
    pub can_request: bool,
    pub operation_available: bool,
    pub settings_pane: Option<String>,
    pub authority_owner: String,
    pub framework: String,
    pub checked_at_ms: i64,
    pub lifecycle: contract::PermissionLifecycleContract,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_receipt: Option<MacosPermissionRequestReceipt>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacosPermissionRequestReceipt {
    pub request_attempt_id: String,
    pub phase: &'static str,
    pub result: String,
    pub granted: Option<bool>,
    pub native_error_code: Option<i64>,
    pub native_error_domain: Option<String>,
    pub elapsed_ms: u64,
    pub retryable: bool,
    pub state_before: Option<MacosPermissionState>,
    pub state_after: MacosPermissionState,
    pub can_request_before: Option<bool>,
    pub can_request_after: bool,
    pub native_request_invoked: bool,
    pub(crate) process_identity: crate::macos_process_identity::MacosProcessIdentityEvidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_reset: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources_refreshed: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MacosPermissionRequest {
    pub capability_id: String,
}

impl MacosPermissionStatus {
    fn with_request_receipt(
        mut self,
        started: std::time::Instant,
        result: &str,
        granted: Option<bool>,
        native_error_code: Option<i64>,
        native_error_domain: Option<String>,
    ) -> Self {
        self.request_receipt = Some(MacosPermissionRequestReceipt {
            request_attempt_id: next_permission_attempt_id(),
            phase: "permission_decision",
            result: result.to_string(),
            granted,
            native_error_code,
            native_error_domain,
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            retryable: matches!(self.state, MacosPermissionState::Stale),
            state_before: None,
            state_after: self.state,
            can_request_before: None,
            can_request_after: self.can_request,
            native_request_invoked: true,
            process_identity: crate::macos_process_identity::current(),
            operation_id: None,
            store_reset: None,
            sources_refreshed: None,
        });
        self
    }

    fn with_calendar_request_receipt(
        mut self,
        receipt: crate::eventkit_calendar::CalendarOperationReceipt,
        retryable: bool,
    ) -> Self {
        use crate::eventkit_calendar::{CalendarOperationOutcome, CalendarOperationPhase};
        self.request_receipt = Some(MacosPermissionRequestReceipt {
            request_attempt_id: next_permission_attempt_id(),
            phase: match receipt.phase {
                CalendarOperationPhase::CheckingAccess => "checking_access",
                CalendarOperationPhase::WaitingForPermission => "waiting_for_permission",
                CalendarOperationPhase::ResettingStore => "resetting_store",
                CalendarOperationPhase::RefreshingSources => "refreshing_sources",
                CalendarOperationPhase::ReadingWindow => "reading_window",
                CalendarOperationPhase::VerifyingResult => "verifying_result",
                CalendarOperationPhase::Writing => "writing",
            },
            result: match receipt.outcome {
                CalendarOperationOutcome::Succeeded => "succeeded",
                CalendarOperationOutcome::Failed => "failed",
                CalendarOperationOutcome::TimedOut => "timed_out",
            }
            .to_string(),
            granted: receipt
                .permission_granted
                .or(Some(self.state == MacosPermissionState::Allowed)),
            native_error_code: receipt.native_error_code,
            native_error_domain: receipt.native_error_domain,
            elapsed_ms: receipt.elapsed_ms,
            retryable,
            state_before: None,
            state_after: self.state,
            can_request_before: None,
            can_request_after: self.can_request,
            native_request_invoked: true,
            process_identity: crate::macos_process_identity::current(),
            operation_id: Some(receipt.operation_id),
            store_reset: Some(receipt.store_reset),
            sources_refreshed: Some(receipt.sources_refreshed),
        });
        self
    }

    fn with_attempt_transition(mut self, before: &MacosPermissionStatus) -> Self {
        if let Some(receipt) = self.request_receipt.as_mut() {
            receipt.state_before = Some(before.state);
            receipt.state_after = self.state;
            receipt.can_request_before = Some(before.can_request);
            receipt.can_request_after = self.can_request;
        }
        self
    }
}

fn next_permission_attempt_id() -> String {
    let sequence = NEXT_PERMISSION_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    format!(
        "permission-attempt-{}-{sequence}",
        unix_time_ms_i64().max(0)
    )
}

#[tauri::command]
pub async fn list_macos_permission_states(_app: tauri::AppHandle) -> Vec<MacosPermissionStatus> {
    #[cfg(target_os = "macos")]
    {
        let mut states = native::snapshot();
        states.push(calendar_status(
            crate::eventkit_calendar::calendar_full_access_status().await,
        ));
        states.push(native::notification_status());
        states.sort_by(|left, right| left.capability_id.cmp(&right.capability_id));
        for state in &states {
            emit_functional_permission_receipt("snapshot", state);
        }
        states
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = _app;
        vec![status(
            "macos_permissions",
            MacosPermissionState::Unsupported,
            false,
            None,
            "main_app",
            "macOS",
        )]
    }
}

#[tauri::command]
pub async fn request_macos_permission(
    request: MacosPermissionRequest,
    _app: tauri::AppHandle,
) -> Result<MacosPermissionStatus, String> {
    let capability_id = request.capability_id.trim().to_string();
    if capability_id.is_empty() || capability_id.len() > 80 {
        return Err("permission_request_invalid".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        if capability_id == "calendar" {
            let started = std::time::Instant::now();
            let before =
                calendar_status(crate::eventkit_calendar::calendar_full_access_status().await);
            let operation =
                crate::eventkit_calendar::ensure_full_calendar_access_with_receipt().await;
            let status =
                calendar_status(crate::eventkit_calendar::calendar_full_access_status().await);
            let updated = match operation {
                Ok(receipt) => status.with_calendar_request_receipt(receipt, false),
                Err(failure) => match failure.receipt {
                    Some(receipt) => {
                        status.with_calendar_request_receipt(receipt, failure.retryable)
                    }
                    None => {
                        status.with_request_receipt(started, &failure.code, Some(false), None, None)
                    }
                },
            }
            .with_attempt_transition(&before);
            emit_functional_permission_receipt("request", &updated);
            return Ok(updated);
        }
        let before = native::status_for(&capability_id).unwrap_or_else(|| {
            status(
                &capability_id,
                MacosPermissionState::Unsupported,
                false,
                settings_pane(&capability_id),
                "main_app",
                "macOS",
            )
        });
        let requested_id = capability_id.clone();
        let updated = tauri::async_runtime::spawn_blocking(move || native::request(&requested_id))
            .await
            .map_err(|_| "permission_request_interrupted".to_string())??
            .with_attempt_transition(&before);
        emit_functional_permission_receipt("request", &updated);
        Ok(updated)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = _app;
        Ok(status(
            &capability_id,
            MacosPermissionState::Unsupported,
            false,
            None,
            "main_app",
            "macOS",
        ))
    }
}

fn emit_functional_permission_receipt(source: &str, status: &MacosPermissionStatus) {
    if !crate::diagnostic_output::native_acceptance_enabled() {
        return;
    }
    let evidence = serde_json::json!({
        "source": source,
        "status": status,
        "processIdentity": crate::macos_process_identity::current(),
    });
    eprintln!("OOMU_APPLE_PERMISSION_RECEIPT {evidence}");
}

#[tauri::command]
pub fn open_macos_permission_settings(
    request: MacosPermissionRequest,
    app: tauri::AppHandle,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let capability_id = request.capability_id.trim();
        let pane = settings_pane(capability_id)
            .ok_or_else(|| "permission_settings_unavailable".to_string())?;
        let url = if capability_id == "notifications" {
            "x-apple.systempreferences:com.apple.Notifications-Settings.extension".to_string()
        } else {
            format!("x-apple.systempreferences:com.apple.preference.security?{pane}")
        };
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|_| "permission_settings_open_failed".to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (request, app);
        Err("permission_settings_unsupported".to_string())
    }
}

fn status(
    capability_id: &str,
    state: MacosPermissionState,
    can_request: bool,
    settings_pane: Option<&str>,
    authority_owner: &str,
    framework: &str,
) -> MacosPermissionStatus {
    let lifecycle = contract::lifecycle(capability_id);
    MacosPermissionStatus {
        capability_id: capability_id.to_string(),
        state,
        can_request,
        operation_available: !matches!(
            lifecycle.request_method,
            contract::PermissionRequestMethod::Unsupported
        ),
        settings_pane: settings_pane.map(str::to_string),
        authority_owner: authority_owner.to_string(),
        framework: framework.to_string(),
        checked_at_ms: unix_time_ms_i64(),
        lifecycle,
        request_receipt: None,
    }
}

fn when_used_status(
    capability_id: &str,
    authority_owner: &str,
    framework: &str,
) -> MacosPermissionStatus {
    status(
        capability_id,
        MacosPermissionState::WhenUsed,
        false,
        settings_pane(capability_id),
        authority_owner,
        framework,
    )
}

pub(crate) async fn status_for_operation(capability_id: &str) -> MacosPermissionStatus {
    #[cfg(target_os = "macos")]
    {
        if capability_id == "calendar" {
            return calendar_status(crate::eventkit_calendar::calendar_full_access_status().await);
        }
        native::status_for(capability_id).unwrap_or_else(|| {
            status(
                capability_id,
                MacosPermissionState::Unsupported,
                false,
                settings_pane(capability_id),
                "main_app",
                "macOS",
            )
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        status(
            capability_id,
            MacosPermissionState::Unsupported,
            false,
            settings_pane(capability_id),
            "main_app",
            "macOS",
        )
    }
}

#[cfg(target_os = "macos")]
fn calendar_status(
    value: crate::eventkit_calendar::CalendarFullAccessStatus,
) -> MacosPermissionStatus {
    use crate::eventkit_calendar::CalendarAuthorizationDisposition as Disposition;
    let (state, can_request) = match value.status {
        Disposition::FullAccess => (MacosPermissionState::Allowed, false),
        Disposition::NotDetermined => (MacosPermissionState::NotRequested, true),
        Disposition::WriteOnly => (MacosPermissionState::Limited, true),
        Disposition::Denied => (MacosPermissionState::Denied, false),
        Disposition::Restricted => (MacosPermissionState::Restricted, false),
        Disposition::Unavailable => (MacosPermissionState::Unsupported, false),
    };
    status(
        "calendar",
        state,
        can_request,
        Some("Privacy_Calendars"),
        "main_app",
        "EventKit",
    )
}

fn settings_pane(capability_id: &str) -> Option<&'static str> {
    match capability_id {
        "accessibility" | "screen_control" => Some("Privacy_Accessibility"),
        "screen_capture" => Some("Privacy_ScreenCapture"),
        "microphone" => Some("Privacy_Microphone"),
        "camera" => Some("Privacy_Camera"),
        "speech_recognition" => Some("Privacy_SpeechRecognition"),
        "music" => Some("Privacy_Media"),
        "photos" => Some("Privacy_Photos"),
        "contacts" => Some("Privacy_Contacts"),
        "calendar" => Some("Privacy_Calendars"),
        "reminders" => Some("Privacy_Reminders"),
        "mail" | "notes" | "messages" | "finder" | "system_events" => Some("Privacy_Automation"),
        "files_and_folders" => Some("Privacy_FilesAndFolders"),
        "full_disk_access" => Some("Privacy_AllFiles"),
        "local_network" => Some("Privacy_LocalNetwork"),
        "notifications" => Some("Notifications"),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::{settings_pane, status, MacosPermissionState, MacosPermissionStatus};
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::AnyThread;
    use objc2_contacts::{CNAuthorizationStatus, CNContactStore, CNEntityType};
    use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore};
    use objc2_foundation::{NSError, NSString};
    use objc2_media_player::{MPMediaLibrary, MPMediaLibraryAuthorizationStatus};
    use objc2_photos::{PHAccessLevel, PHAuthorizationStatus, PHPhotoLibrary};
    use std::{
        ffi::c_void,
        sync::mpsc,
        time::{Duration, Instant},
    };

    const NO_ERR: i32 = 0;
    const PROCESS_NOT_FOUND: i32 = -600;
    const EVENT_NOT_PERMITTED: i32 = -1743;
    const EVENT_REQUIRES_CONSENT: i32 = -1744;

    #[derive(Debug)]
    struct CallbackEvidence {
        result: String,
        granted: Option<bool>,
        native_error_code: Option<i64>,
        native_error_domain: Option<String>,
        store_reset: bool,
        sources_refreshed: bool,
    }

    impl CallbackEvidence {
        fn granted(result: &str, granted: bool) -> Self {
            Self {
                result: result.to_string(),
                granted: Some(granted),
                native_error_code: None,
                native_error_domain: None,
                store_reset: false,
                sources_refreshed: false,
            }
        }
    }

    fn attach(
        status: MacosPermissionStatus,
        started: Instant,
        evidence: CallbackEvidence,
    ) -> MacosPermissionStatus {
        let mut status = status.with_request_receipt(
            started,
            &evidence.result,
            evidence.granted,
            evidence.native_error_code,
            evidence.native_error_domain,
        );
        if let Some(receipt) = status.request_receipt.as_mut() {
            receipt.store_reset = Some(evidence.store_reset);
            receipt.sources_refreshed = Some(evidence.sources_refreshed);
        }
        status
    }

    unsafe fn callback_error(error: *mut NSError) -> (Option<i64>, Option<String>) {
        error.as_ref().map_or((None, None), |error| {
            let mut domain = error.domain().to_string();
            domain.truncate(120);
            (Some(error.code() as i64), Some(domain))
        })
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
        fn AECreateDesc(
            descriptor_type: u32,
            data: *const c_void,
            size: isize,
            result: *mut AEDesc,
        ) -> i32;
        fn AEDeterminePermissionToAutomateTarget(
            target: *const AEDesc,
            event_class: u32,
            event_id: u32,
            ask_user_if_needed: u8,
        ) -> i32;
        fn AEDisposeDesc(descriptor: *mut AEDesc) -> i32;
        static kAXTrustedCheckOptionPrompt: *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: *const c_void;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            count: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(value: *const c_void);
    }

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "Speech", kind = "framework")]
    unsafe extern "C" {}

    #[repr(C)]
    struct AEDesc {
        descriptor_type: u32,
        data_handle: *mut c_void,
    }

    impl Default for AEDesc {
        fn default() -> Self {
            Self {
                descriptor_type: 0,
                data_handle: std::ptr::null_mut(),
            }
        }
    }

    impl Drop for AEDesc {
        fn drop(&mut self) {
            if self.descriptor_type != 0 || !self.data_handle.is_null() {
                unsafe { AEDisposeDesc(self) };
            }
        }
    }

    pub(super) fn snapshot() -> Vec<MacosPermissionStatus> {
        let mut values = vec![
            accessibility_status("accessibility"),
            accessibility_status("screen_control"),
            screen_capture_status(),
            contacts_status(),
            photos_status(),
            reminders_status(),
            music_status(),
            super::when_used_status("files_and_folders", "main_app", "Powerbox"),
            full_disk_access_status(),
            unsupported_local_network_status(),
        ];
        for (capability, bundle_id) in automation_targets() {
            values.push(automation_status(capability, bundle_id, false));
        }
        for capability in ["microphone", "camera", "speech_recognition"] {
            values.push(dynamic_media_status(capability));
        }
        values
    }

    pub(super) fn status_for(capability_id: &str) -> Option<MacosPermissionStatus> {
        let value = match capability_id {
            "accessibility" | "screen_control" => accessibility_status(capability_id),
            "screen_capture" => screen_capture_status(),
            "contacts" => contacts_status(),
            "photos" => photos_status(),
            "reminders" => reminders_status(),
            "music" => music_status(),
            "notifications" => notification_status(),
            "microphone" | "camera" | "speech_recognition" => dynamic_media_status(capability_id),
            "files_and_folders" => super::when_used_status(capability_id, "main_app", "Powerbox"),
            "full_disk_access" => full_disk_access_status(),
            "local_network" => unsupported_local_network_status(),
            _ => {
                let (_, bundle_id) = automation_targets()
                    .into_iter()
                    .find(|(candidate, _)| *candidate == capability_id)?;
                automation_status(capability_id, bundle_id, false)
            }
        };
        Some(value)
    }

    pub(super) fn request(capability_id: &str) -> Result<MacosPermissionStatus, String> {
        let started = Instant::now();
        if let Some(result) = request_direct_permission(capability_id, started) {
            return result;
        }
        request_automation_permission(capability_id, started)
    }

    fn request_direct_permission(
        capability_id: &str,
        started: Instant,
    ) -> Option<Result<MacosPermissionStatus, String>> {
        let result = match capability_id {
            "accessibility" | "screen_control" => request_accessibility(capability_id, started),
            "screen_capture" => request_screen_capture(started),
            "contacts" => {
                request_contacts().map(|evidence| attach(contacts_status(), started, evidence))
            }
            "photos" => request_photos().map(|evidence| attach(photos_status(), started, evidence)),
            "reminders" => {
                request_reminders().map(|evidence| attach(reminders_status(), started, evidence))
            }
            "music" => request_music().map(|evidence| attach(music_status(), started, evidence)),
            "camera" => request_dynamic_media(capability_id)
                .map(|evidence| attach(dynamic_media_status(capability_id), started, evidence)),
            "notifications" => request_notifications()
                .map(|evidence| attach(notification_status(), started, evidence)),
            "microphone" | "speech_recognition" => {
                Err("permission_request_uses_voice_input".to_string())
            }
            _ => return None,
        };
        Some(result)
    }

    fn request_accessibility(
        capability_id: &str,
        started: Instant,
    ) -> Result<MacosPermissionStatus, String> {
        request_accessibility_prompt();
        let status = accessibility_status(capability_id);
        let allowed = status.state == MacosPermissionState::Allowed;
        Ok(attach(
            status,
            started,
            CallbackEvidence::granted(
                if allowed {
                    "allowed"
                } else {
                    "settings_required"
                },
                allowed,
            ),
        ))
    }

    fn request_screen_capture(started: Instant) -> Result<MacosPermissionStatus, String> {
        let granted = unsafe { CGRequestScreenCaptureAccess() };
        Ok(attach(
            screen_capture_status(),
            started,
            CallbackEvidence::granted(if granted { "allowed" } else { "denied" }, granted),
        ))
    }

    fn request_automation_permission(
        capability_id: &str,
        started: Instant,
    ) -> Result<MacosPermissionStatus, String> {
        let (capability, bundle) = automation_targets()
            .into_iter()
            .find(|(candidate, _)| *candidate == capability_id)
            .ok_or_else(|| "permission_request_uses_contextual_system_prompt".to_string())?;
        let (status, native) = automation_evaluation(capability, bundle, true);
        let result = match status.state {
            MacosPermissionState::Allowed => "allowed",
            MacosPermissionState::Denied => "denied",
            MacosPermissionState::NotRequested => "not_requested",
            MacosPermissionState::Stale => "target_unavailable",
            _ => "unsupported",
        };
        Ok(attach(
            status,
            started,
            CallbackEvidence {
                result: result.to_string(),
                granted: Some(native == NO_ERR),
                native_error_code: (native != NO_ERR).then_some(i64::from(native)),
                native_error_domain: automation_error_domain(native),
                store_reset: false,
                sources_refreshed: false,
            },
        ))
    }

    fn automation_error_domain(native: i32) -> Option<String> {
        (native != NO_ERR).then(|| "NSOSStatusErrorDomain".to_string())
    }

    fn accessibility_status(capability: &str) -> MacosPermissionStatus {
        let allowed = unsafe { AXIsProcessTrusted() };
        status(
            capability,
            if allowed {
                MacosPermissionState::Allowed
            } else {
                MacosPermissionState::RequiresSettings
            },
            !allowed,
            settings_pane(capability),
            "main_app",
            "ApplicationServices",
        )
    }

    fn screen_capture_status() -> MacosPermissionStatus {
        let allowed = unsafe { CGPreflightScreenCaptureAccess() };
        status(
            "screen_capture",
            if allowed {
                MacosPermissionState::Allowed
            } else {
                MacosPermissionState::RequiresSettings
            },
            !allowed,
            settings_pane("screen_capture"),
            "main_app",
            "CoreGraphics",
        )
    }

    fn contacts_status() -> MacosPermissionStatus {
        let native =
            unsafe { CNContactStore::authorizationStatusForEntityType(CNEntityType::Contacts) };
        let (state, requestable) = if native == CNAuthorizationStatus::Authorized {
            (MacosPermissionState::Allowed, false)
        } else if native == CNAuthorizationStatus::Limited {
            (MacosPermissionState::Limited, false)
        } else if native == CNAuthorizationStatus::NotDetermined {
            (MacosPermissionState::NotRequested, true)
        } else if native == CNAuthorizationStatus::Denied {
            (MacosPermissionState::Denied, false)
        } else {
            (MacosPermissionState::Restricted, false)
        };
        status(
            "contacts",
            state,
            requestable,
            settings_pane("contacts"),
            "main_app",
            "Contacts",
        )
    }

    fn request_contacts() -> Result<CallbackEvidence, String> {
        let store = unsafe { CNContactStore::init(CNContactStore::alloc()) };
        let (sender, receiver) = mpsc::channel();
        let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            let (native_error_code, native_error_domain) = unsafe { callback_error(error) };
            let granted = granted.as_bool();
            let _ = sender.send(CallbackEvidence {
                result: if granted { "allowed" } else { "denied" }.to_string(),
                granted: Some(granted),
                native_error_code,
                native_error_domain,
                store_reset: false,
                sources_refreshed: false,
            });
        });
        unsafe {
            store.requestAccessForEntityType_completionHandler(CNEntityType::Contacts, &completion)
        };
        receiver
            .recv()
            .map_err(|_| "permission_request_interrupted".to_string())
    }

    fn photos_status() -> MacosPermissionStatus {
        let native =
            unsafe { PHPhotoLibrary::authorizationStatusForAccessLevel(PHAccessLevel::ReadWrite) };
        let (state, requestable) = if native == PHAuthorizationStatus::Authorized {
            (MacosPermissionState::Allowed, false)
        } else if native == PHAuthorizationStatus::Limited {
            (MacosPermissionState::Limited, false)
        } else if native == PHAuthorizationStatus::NotDetermined {
            (MacosPermissionState::NotRequested, true)
        } else if native == PHAuthorizationStatus::Denied {
            (MacosPermissionState::Denied, false)
        } else {
            (MacosPermissionState::Restricted, false)
        };
        status(
            "photos",
            state,
            requestable,
            settings_pane("photos"),
            "main_app",
            "PhotoKit",
        )
    }

    fn request_photos() -> Result<CallbackEvidence, String> {
        let (sender, receiver) = mpsc::channel();
        let completion = RcBlock::new(move |status: PHAuthorizationStatus| {
            let (result, granted) = if status == PHAuthorizationStatus::Authorized {
                ("allowed", true)
            } else if status == PHAuthorizationStatus::Limited {
                ("limited", true)
            } else if status == PHAuthorizationStatus::Denied {
                ("denied", false)
            } else if status == PHAuthorizationStatus::Restricted {
                ("restricted", false)
            } else {
                ("not_requested", false)
            };
            let _ = sender.send(CallbackEvidence::granted(result, granted));
        });
        unsafe {
            PHPhotoLibrary::requestAuthorizationForAccessLevel_handler(
                PHAccessLevel::ReadWrite,
                &completion,
            )
        };
        receiver
            .recv()
            .map_err(|_| "permission_request_interrupted".to_string())
    }

    fn reminders_status() -> MacosPermissionStatus {
        let native =
            unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Reminder) };
        let (state, requestable) = eventkit_state(native);
        status(
            "reminders",
            state,
            requestable,
            settings_pane("reminders"),
            "main_app",
            "EventKit",
        )
    }

    fn eventkit_state(native: EKAuthorizationStatus) -> (MacosPermissionState, bool) {
        if native == EKAuthorizationStatus::FullAccess {
            (MacosPermissionState::Allowed, false)
        } else if native == EKAuthorizationStatus::WriteOnly {
            (MacosPermissionState::Limited, true)
        } else if native == EKAuthorizationStatus::NotDetermined {
            (MacosPermissionState::NotRequested, true)
        } else if native == EKAuthorizationStatus::Denied {
            (MacosPermissionState::Denied, false)
        } else {
            (MacosPermissionState::Restricted, false)
        }
    }

    fn request_reminders() -> Result<CallbackEvidence, String> {
        let store = unsafe { EKEventStore::init(EKEventStore::alloc()) };
        let (sender, receiver) = mpsc::channel();
        let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            let (native_error_code, native_error_domain) = unsafe { callback_error(error) };
            let granted = granted.as_bool();
            let _ = sender.send(CallbackEvidence {
                result: if granted { "allowed" } else { "denied" }.to_string(),
                granted: Some(granted),
                native_error_code,
                native_error_domain,
                store_reset: false,
                sources_refreshed: false,
            });
        });
        unsafe { store.requestFullAccessToRemindersWithCompletion(RcBlock::as_ptr(&completion)) };
        let mut evidence = receiver
            .recv()
            .map_err(|_| "permission_request_interrupted".to_string())?;
        let full_access = unsafe {
            EKEventStore::authorizationStatusForEntityType(EKEntityType::Reminder)
                == EKAuthorizationStatus::FullAccess
        };
        if evidence.granted == Some(true) && evidence.native_error_code.is_none() && full_access {
            unsafe {
                store.reset();
                store.refreshSourcesIfNecessary();
            }
            evidence.store_reset = true;
            evidence.sources_refreshed = true;
        }
        Ok(evidence)
    }

    fn music_status() -> MacosPermissionStatus {
        let native = unsafe { MPMediaLibrary::authorizationStatus() };
        let (state, requestable) = if native == MPMediaLibraryAuthorizationStatus::Authorized {
            (MacosPermissionState::Allowed, false)
        } else if native == MPMediaLibraryAuthorizationStatus::NotDetermined {
            (MacosPermissionState::NotRequested, true)
        } else if native == MPMediaLibraryAuthorizationStatus::Denied {
            (MacosPermissionState::Denied, false)
        } else {
            (MacosPermissionState::Restricted, false)
        };
        status(
            "music",
            state,
            requestable,
            settings_pane("music"),
            "main_app",
            "MediaPlayer",
        )
    }

    fn request_music() -> Result<CallbackEvidence, String> {
        let (sender, receiver) = mpsc::channel();
        let completion = RcBlock::new(move |status: MPMediaLibraryAuthorizationStatus| {
            let (result, granted) = if status == MPMediaLibraryAuthorizationStatus::Authorized {
                ("allowed", true)
            } else if status == MPMediaLibraryAuthorizationStatus::Denied {
                ("denied", false)
            } else if status == MPMediaLibraryAuthorizationStatus::Restricted {
                ("restricted", false)
            } else {
                ("not_requested", false)
            };
            let _ = sender.send(CallbackEvidence::granted(result, granted));
        });
        unsafe { MPMediaLibrary::requestAuthorization(&completion) };
        receiver
            .recv()
            .map_err(|_| "permission_request_interrupted".to_string())
    }

    pub(super) fn notification_status() -> MacosPermissionStatus {
        if !crate::macos_process_identity::current_executable_is_bundled_app() {
            return notification_application_unavailable_status();
        }
        let (state, can_request) = match notification_authorization_status() {
            Some(0) => (MacosPermissionState::NotRequested, true),
            Some(1) => (MacosPermissionState::Denied, false),
            Some(2) => (MacosPermissionState::Allowed, false),
            Some(3) | Some(4) => (MacosPermissionState::Limited, false),
            _ => (MacosPermissionState::Stale, false),
        };
        status(
            "notifications",
            state,
            can_request,
            settings_pane("notifications"),
            "main_app",
            "UserNotifications",
        )
    }

    fn notification_authorization_status() -> Option<isize> {
        let center = notification_center()?;
        let (sender, receiver) = mpsc::channel();
        let completion = RcBlock::new(move |settings: *mut AnyObject| {
            let status = if settings.is_null() {
                None
            } else {
                Some(unsafe { msg_send![settings, authorizationStatus] })
            };
            let _ = sender.send(status);
        });
        unsafe {
            let _: () =
                msg_send![center, getNotificationSettingsWithCompletionHandler: &*completion];
        }
        receiver.recv_timeout(Duration::from_secs(5)).ok().flatten()
    }

    fn request_notifications() -> Result<CallbackEvidence, String> {
        let center =
            notification_center().ok_or_else(|| "permission_request_unavailable".to_string())?;
        let (sender, receiver) = mpsc::channel();
        let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            let (native_error_code, native_error_domain) = unsafe { callback_error(error) };
            let granted = granted.as_bool();
            let _ = sender.send(CallbackEvidence {
                result: if granted { "allowed" } else { "denied" }.to_string(),
                granted: Some(granted),
                native_error_code,
                native_error_domain,
                store_reset: false,
                sources_refreshed: false,
            });
        });
        unsafe {
            let _: () = msg_send![center, requestAuthorizationWithOptions: 7usize, completionHandler: &*completion];
        }
        receiver
            .recv()
            .map_err(|_| "permission_request_interrupted".to_string())
    }

    fn notification_center() -> Option<*mut AnyObject> {
        if !crate::macos_process_identity::current_executable_is_bundled_app() {
            return None;
        }
        let class = AnyClass::get(c"UNUserNotificationCenter")?;
        let center: *mut AnyObject = unsafe { msg_send![class, currentNotificationCenter] };
        (!center.is_null()).then_some(center)
    }

    fn notification_application_unavailable_status() -> MacosPermissionStatus {
        let mut value = status(
            "notifications",
            MacosPermissionState::Unsupported,
            false,
            settings_pane("notifications"),
            "main_app",
            "UserNotifications",
        );
        value.operation_available = false;
        value
    }

    fn automation_targets() -> [(&'static str, &'static [u8]); 5] {
        [
            ("mail", b"com.apple.mail"),
            ("notes", b"com.apple.Notes"),
            ("messages", b"com.apple.MobileSMS"),
            ("finder", b"com.apple.finder"),
            ("system_events", b"com.apple.systemevents"),
        ]
    }

    fn automation_status(capability: &str, bundle_id: &[u8], ask: bool) -> MacosPermissionStatus {
        automation_evaluation(capability, bundle_id, ask).0
    }

    fn automation_evaluation(
        capability: &str,
        bundle_id: &[u8],
        ask: bool,
    ) -> (MacosPermissionStatus, i32) {
        let mut target = AEDesc::default();
        let created = unsafe {
            AECreateDesc(
                u32::from_be_bytes(*b"bund"),
                bundle_id.as_ptr().cast(),
                bundle_id.len() as isize,
                &mut target,
            )
        };
        let native = if created == NO_ERR {
            unsafe {
                AEDeterminePermissionToAutomateTarget(
                    &target,
                    u32::from_be_bytes(*b"****"),
                    u32::from_be_bytes(*b"****"),
                    u8::from(ask),
                )
            }
        } else {
            created
        };
        let (state, can_request) = match native {
            NO_ERR => (MacosPermissionState::Allowed, false),
            EVENT_REQUIRES_CONSENT => (MacosPermissionState::NotRequested, true),
            EVENT_NOT_PERMITTED => (MacosPermissionState::Denied, false),
            PROCESS_NOT_FOUND => (MacosPermissionState::Stale, true),
            _ => (MacosPermissionState::Unsupported, false),
        };
        (
            status(
                capability,
                state,
                can_request,
                settings_pane(capability),
                "main_app",
                "AppleEvents",
            ),
            native,
        )
    }

    fn request_accessibility_prompt() {
        unsafe {
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                std::ptr::null(),
                std::ptr::null(),
            );
            if !options.is_null() {
                AXIsProcessTrustedWithOptions(options);
                CFRelease(options);
            }
        }
    }

    fn full_disk_access_status() -> MacosPermissionStatus {
        let state = match crate::native_capability_adapters::probe_full_disk_access() {
            crate::native_capability_adapters::FullDiskAccessProbe::Allowed { bytes_read: 16 } => {
                MacosPermissionState::Allowed
            }
            crate::native_capability_adapters::FullDiskAccessProbe::PermissionRequired => {
                MacosPermissionState::RequiresSettings
            }
            crate::native_capability_adapters::FullDiskAccessProbe::Unsupported => {
                MacosPermissionState::Unsupported
            }
            crate::native_capability_adapters::FullDiskAccessProbe::Allowed { .. }
            | crate::native_capability_adapters::FullDiskAccessProbe::Stale => {
                MacosPermissionState::Stale
            }
        };
        status(
            "full_disk_access",
            state,
            false,
            settings_pane("full_disk_access"),
            "main_app",
            "macOS",
        )
    }

    fn unsupported_local_network_status() -> MacosPermissionStatus {
        status(
            "local_network",
            MacosPermissionState::Unsupported,
            false,
            settings_pane("local_network"),
            "main_app",
            "Network",
        )
    }

    #[cfg(test)]
    pub(super) fn full_disk_state(probe: &std::io::Result<()>) -> MacosPermissionState {
        match probe {
            Ok(()) => MacosPermissionState::Allowed,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                MacosPermissionState::RequiresSettings
            }
            Err(_) => MacosPermissionState::Stale,
        }
    }

    pub(super) fn dynamic_media_status(capability: &str) -> MacosPermissionStatus {
        if matches!(capability, "microphone" | "speech_recognition") {
            return status(
                capability,
                MacosPermissionState::WhenUsed,
                false,
                settings_pane(capability),
                "oomu-speech-bridge",
                if capability == "speech_recognition" {
                    "Speech"
                } else {
                    "AVFoundation"
                },
            );
        }
        let native = media_authorization_status(capability);
        let (state, can_request) = match native {
            Some(0) => (MacosPermissionState::NotRequested, true),
            Some(1) => (MacosPermissionState::Restricted, false),
            Some(2) => (MacosPermissionState::Denied, false),
            Some(3) => (MacosPermissionState::Allowed, false),
            _ => (MacosPermissionState::Unsupported, false),
        };
        status(
            capability,
            state,
            can_request,
            settings_pane(capability),
            "main_app",
            if capability == "speech_recognition" {
                "Speech"
            } else {
                "AVFoundation"
            },
        )
    }

    fn media_authorization_status(capability: &str) -> Option<isize> {
        unsafe {
            if capability == "speech_recognition" {
                let class = AnyClass::get(c"SFSpeechRecognizer")?;
                return Some(msg_send![class, authorizationStatus]);
            }
            let class = AnyClass::get(c"AVCaptureDevice")?;
            let media = NSString::from_str(if capability == "camera" {
                "vide"
            } else {
                "soun"
            });
            Some(msg_send![class, authorizationStatusForMediaType: &*media])
        }
    }

    fn request_dynamic_media(capability: &str) -> Result<CallbackEvidence, String> {
        let (sender, receiver) = mpsc::channel();
        unsafe {
            if capability == "speech_recognition" {
                let class = AnyClass::get(c"SFSpeechRecognizer")
                    .ok_or_else(|| "permission_framework_unavailable".to_string())?;
                let completion = RcBlock::new(move |status: isize| {
                    let granted = status == 3;
                    let _ = sender.send(CallbackEvidence::granted(
                        if granted { "allowed" } else { "denied" },
                        granted,
                    ));
                });
                let _: () = msg_send![class, requestAuthorization: &*completion];
            } else {
                let class = AnyClass::get(c"AVCaptureDevice")
                    .ok_or_else(|| "permission_framework_unavailable".to_string())?;
                let media = NSString::from_str(if capability == "camera" {
                    "vide"
                } else {
                    "soun"
                });
                let completion = RcBlock::new(move |granted: Bool| {
                    let granted = granted.as_bool();
                    let _ = sender.send(CallbackEvidence::granted(
                        if granted { "allowed" } else { "denied" },
                        granted,
                    ));
                });
                let _: () = msg_send![class, requestAccessForMediaType: &*media, completionHandler: &*completion];
            }
        }
        receiver
            .recv()
            .map_err(|_| "permission_request_interrupted".to_string())
    }

    #[cfg(test)]
    mod callback_tests {
        use super::*;

        #[test]
        fn apple_permission_callbacks_preserve_native_result() {
            let status = status(
                "contacts",
                MacosPermissionState::Denied,
                false,
                settings_pane("contacts"),
                "main_app",
                "Contacts",
            );
            let receipt = attach(
                status,
                Instant::now(),
                CallbackEvidence {
                    result: "denied".to_string(),
                    granted: Some(false),
                    native_error_code: Some(100),
                    native_error_domain: Some("CNErrorDomain".to_string()),
                    store_reset: false,
                    sources_refreshed: false,
                },
            )
            .request_receipt
            .expect("request receipt");
            assert_eq!(receipt.granted, Some(false));
            assert_eq!(receipt.native_error_code, Some(100));
            assert_eq!(
                receipt.native_error_domain.as_deref(),
                Some("CNErrorDomain")
            );
        }

        #[test]
        fn apple_event_osstatus_is_not_labeled_as_an_applescript_error() {
            assert_eq!(automation_error_domain(NO_ERR), None);
            assert_eq!(
                automation_error_domain(EVENT_NOT_PERMITTED).as_deref(),
                Some("NSOSStatusErrorDomain")
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_state_vocabulary_is_bounded_and_plain() {
        let states = [
            MacosPermissionState::NotRequested,
            MacosPermissionState::Allowed,
            MacosPermissionState::Limited,
            MacosPermissionState::Denied,
            MacosPermissionState::Restricted,
            MacosPermissionState::RequiresSettings,
            MacosPermissionState::Stale,
            MacosPermissionState::WhenUsed,
            MacosPermissionState::Unsupported,
        ];
        let encoded = serde_json::to_value(states).unwrap();
        assert_eq!(encoded.as_array().unwrap().len(), 9);
        assert!(encoded.to_string().contains("requires_settings"));
    }

    #[test]
    fn every_repairable_capability_has_an_exact_settings_destination() {
        for capability in [
            "accessibility",
            "screen_control",
            "screen_capture",
            "microphone",
            "camera",
            "speech_recognition",
            "music",
            "photos",
            "contacts",
            "calendar",
            "reminders",
            "mail",
            "notes",
            "messages",
            "finder",
            "system_events",
            "files_and_folders",
            "full_disk_access",
            "local_network",
            "notifications",
        ] {
            assert!(settings_pane(capability).is_some(), "{capability}");
        }
    }

    #[test]
    fn contextual_permissions_make_no_false_preflight_claim() {
        let state = when_used_status("files_and_folders", "main_app", "Powerbox");
        assert_eq!(state.state, MacosPermissionState::WhenUsed);
        assert!(!state.can_request);
        assert!(state.settings_pane.is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unshipped_local_network_operation_is_never_advertised_as_available() {
        let state = native::status_for("local_network").expect("known capability");
        assert_eq!(state.state, MacosPermissionState::Unsupported);
        assert!(!state.can_request);
        assert!(!state.operation_available);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unbundled_dev_process_never_opens_user_notification_center() {
        if crate::macos_process_identity::current_executable_is_bundled_app() {
            return;
        }
        let state = native::notification_status();
        assert_eq!(state.state, MacosPermissionState::Unsupported);
        assert!(!state.can_request);
        assert!(!state.operation_available);
        assert_eq!(
            native::request("notifications").unwrap_err(),
            "permission_request_unavailable"
        );
    }

    #[test]
    fn permission_attempts_keep_distinct_before_and_after_evidence() {
        let before = status(
            "accessibility",
            MacosPermissionState::RequiresSettings,
            true,
            settings_pane("accessibility"),
            "main_app",
            "ApplicationServices",
        );
        let after = status(
            "accessibility",
            MacosPermissionState::Allowed,
            false,
            settings_pane("accessibility"),
            "main_app",
            "ApplicationServices",
        );
        let first = after
            .clone()
            .with_request_receipt(std::time::Instant::now(), "allowed", Some(true), None, None)
            .with_attempt_transition(&before);
        let second = after
            .with_request_receipt(std::time::Instant::now(), "allowed", Some(true), None, None)
            .with_attempt_transition(&before);
        let first = first.request_receipt.expect("first attempt");
        let second = second.request_receipt.expect("second attempt");
        assert_ne!(first.request_attempt_id, second.request_attempt_id);
        assert_eq!(
            first.state_before,
            Some(MacosPermissionState::RequiresSettings)
        );
        assert_eq!(first.state_after, MacosPermissionState::Allowed);
        assert_eq!(first.can_request_before, Some(true));
        assert!(!first.can_request_after);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn full_disk_access_uses_a_live_read_only_probe() {
        assert_eq!(
            native::full_disk_state(&Ok(())),
            MacosPermissionState::Allowed
        );
        assert_eq!(
            native::full_disk_state(&Err(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied
            ))),
            MacosPermissionState::RequiresSettings
        );
        assert_eq!(
            native::full_disk_state(&Err(std::io::Error::from(std::io::ErrorKind::NotFound))),
            MacosPermissionState::Stale
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn voice_permissions_are_never_reported_from_the_main_process() {
        for capability in ["microphone", "speech_recognition"] {
            let state = native::dynamic_media_status(capability);
            assert_eq!(state.state, MacosPermissionState::WhenUsed);
            assert_eq!(state.authority_owner, "oomu-speech-bridge");
            assert!(!state.can_request);
            assert_eq!(
                native::request(capability).unwrap_err(),
                "permission_request_uses_voice_input"
            );
        }
        assert_eq!(
            native::dynamic_media_status("camera").authority_owner,
            "main_app"
        );
    }
}
