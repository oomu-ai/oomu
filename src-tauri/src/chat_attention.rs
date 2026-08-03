use tauri::Manager;
use tauri_plugin_notification::{NotificationExt, PermissionState};

pub(crate) fn set_dock_unread_count(
    app: &tauri::AppHandle,
    unread_count: i64,
) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "chat_attention_main_window_unavailable".to_string())?;
    window
        .set_badge_count((unread_count > 0).then_some(unread_count))
        .map_err(|error| format!("chat_attention_badge_failed:{error}"))
}

pub(crate) fn show_background_completion(app: &tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // The notification plugin otherwise attributes development notifications
        // to Terminal. Bind the process once to OOMU's own bundle identifier so
        // the development and packaged paths exercise the same native behavior.
        let _ = mac_notification_sys::set_application(&app.config().identifier);
        install_macos_foreground_notification_presentation()?;
    }
    let notification = app.notification();
    let permission = match notification
        .permission_state()
        .map_err(|error| format!("chat_attention_notification_state_failed:{error}"))?
    {
        PermissionState::Granted => PermissionState::Granted,
        PermissionState::Prompt | PermissionState::PromptWithRationale => notification
            .request_permission()
            .map_err(|error| format!("chat_attention_notification_permission_failed:{error}"))?,
        PermissionState::Denied => return Err("chat_attention_notification_denied".to_string()),
    };
    if permission != PermissionState::Granted {
        return Err("chat_attention_notification_not_granted".to_string());
    }
    notification
        .builder()
        .title("OOMU — Answer ready")
        .body("Your background response is ready.")
        .group("oomu-chat-completions")
        .auto_cancel()
        .show()
        .map_err(|error| format!("chat_attention_notification_failed:{error}"))
}

#[cfg(target_os = "macos")]
fn install_macos_foreground_notification_presentation() -> Result<(), String> {
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::{ffi, sel};
    use std::sync::OnceLock;

    static INSTALLATION: OnceLock<Result<(), String>> = OnceLock::new();
    INSTALLATION
        .get_or_init(|| {
            // mac-notification-sys owns this delegate and installs it before
            // delivering a notification. Its delegate does not implement the
            // optional foreground-presentation callback, so macOS stores the
            // notification without showing a banner while OOMU is active.
            // Adding that one optional method preserves the dependency's
            // delivery/activation callbacks while making foreground banners
            // explicit and deterministic.
            unsafe extern "C-unwind" fn should_present_notification(
                _delegate: *mut AnyObject,
                _selector: Sel,
                _center: *mut AnyObject,
                _notification: *mut AnyObject,
            ) -> Bool {
                Bool::YES
            }

            let delegate_class = AnyClass::get(c"NotificationCenterDelegate")
                .ok_or_else(|| "chat_attention_notification_delegate_unavailable".to_string())?;
            let selector = sel!(userNotificationCenter:shouldPresentNotification:);
            if !unsafe { ffi::class_getInstanceMethod(delegate_class, selector) }.is_null() {
                return Ok(());
            }

            // SAFETY: Objective-C IMP values are untyped function pointers.
            // The implementation exactly matches the delegate callback ABI:
            // BOOL self _cmd NSUserNotificationCenter* NSUserNotification*.
            let implementation: Imp = unsafe {
                std::mem::transmute::<*const (), Imp>(should_present_notification as *const ())
            };
            let added = unsafe {
                ffi::class_addMethod(
                    delegate_class as *const AnyClass as *mut AnyClass,
                    selector,
                    implementation,
                    c"B@:@@".as_ptr(),
                )
            };
            if added.as_bool() {
                Ok(())
            } else {
                Err("chat_attention_foreground_notification_hook_failed".to_string())
            }
        })
        .clone()
}

#[cfg(target_os = "macos")]
pub(crate) fn clear_delivered_chat_notifications() {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};

    // SAFETY: NSUserNotificationCenter is process-global on macOS. The class and
    // selectors are available on every deployment target supported by OOMU.
    unsafe {
        let center: *mut AnyObject = msg_send![
            class!(NSUserNotificationCenter),
            defaultUserNotificationCenter
        ];
        if !center.is_null() {
            let _: () = msg_send![center, removeAllDeliveredNotifications];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn clear_delivered_chat_notifications() {}
