use super::NotificationDeliveryEvidence;
use block2::RcBlock;
use objc2::{msg_send, runtime::AnyClass, runtime::AnyObject, runtime::Bool};
use objc2_foundation::NSString;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(5);
static NEXT_NOTIFICATION_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) async fn deliver_notification_and_verify(
    title: String,
    subtitle: String,
    body: String,
) -> Result<NotificationDeliveryEvidence, String> {
    tauri::async_runtime::spawn_blocking(move || {
        deliver_notification_and_verify_blocking(&title, &subtitle, &body)
    })
    .await
    .map_err(|_| "notification_delivery_interrupted".to_string())?
}

pub(crate) fn deliver_notification_and_verify_blocking(
    title: &str,
    subtitle: &str,
    body: &str,
) -> Result<NotificationDeliveryEvidence, String> {
    if body.trim().is_empty() {
        return Err("notification_body_required".to_string());
    }
    if !crate::macos_process_identity::current_executable_is_bundled_app() {
        return Err("notification_application_bundle_required".to_string());
    }
    let notification_id = format!(
        "oomu-{}-{}-{}",
        crate::foundation::clock::unix_time_ms_i64().max(0),
        std::process::id(),
        NEXT_NOTIFICATION_ID.fetch_add(1, Ordering::Relaxed),
    );
    unsafe {
        let center_class = AnyClass::get(c"UNUserNotificationCenter")
            .ok_or_else(|| "notification_framework_unavailable".to_string())?;
        let center: *mut AnyObject = msg_send![center_class, currentNotificationCenter];
        let content_class = AnyClass::get(c"UNMutableNotificationContent")
            .ok_or_else(|| "notification_framework_unavailable".to_string())?;
        let content: *mut AnyObject = msg_send![content_class, new];
        if center.is_null() || content.is_null() {
            return Err("notification_framework_unavailable".to_string());
        }
        let title_value = NSString::from_str(if title.trim().is_empty() {
            "OOMU"
        } else {
            title
        });
        let subtitle_value = NSString::from_str(subtitle);
        let body_value = NSString::from_str(body);
        let identifier = NSString::from_str(&notification_id);
        let _: () = msg_send![content, setTitle: &*title_value];
        let _: () = msg_send![content, setSubtitle: &*subtitle_value];
        let _: () = msg_send![content, setBody: &*body_value];
        let request_class = AnyClass::get(c"UNNotificationRequest")
            .ok_or_else(|| "notification_framework_unavailable".to_string())?;
        let request: *mut AnyObject = msg_send![request_class,
            requestWithIdentifier: &*identifier,
            content: content,
            trigger: std::ptr::null_mut::<AnyObject>()
        ];
        if request.is_null() {
            let _: () = msg_send![content, release];
            return Err("notification_request_unavailable".to_string());
        }
        let (submitted_tx, submitted_rx) = mpsc::channel();
        let completion = RcBlock::new(move |error: *mut AnyObject| {
            let _ = submitted_tx.send(error.is_null());
        });
        let _: () = msg_send![center,
            addNotificationRequest: request,
            withCompletionHandler: &*completion
        ];
        let submitted = submitted_rx
            .recv_timeout(DELIVERY_TIMEOUT)
            .map_err(|_| "notification_submission_timeout".to_string())?;
        let _: () = msg_send![content, release];
        if !submitted {
            return Err("notification_submission_failed".to_string());
        }

        let deadline = std::time::Instant::now() + DELIVERY_TIMEOUT;
        while std::time::Instant::now() < deadline {
            if delivered(center, &identifier)? {
                return Ok(NotificationDeliveryEvidence {
                    notification_id,
                    submitted: true,
                    delivered: true,
                });
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Err("notification_delivery_not_verified".to_string())
}

unsafe fn delivered(center: *mut AnyObject, identifier: &NSString) -> Result<bool, String> {
    let (sender, receiver) = mpsc::channel();
    let expected = identifier.to_string();
    let completion = RcBlock::new(move |notifications: *mut AnyObject| {
        let expected = NSString::from_str(&expected);
        let mut found = false;
        if !notifications.is_null() {
            let count: usize = unsafe { msg_send![notifications, count] };
            for index in 0..count {
                let notification: *mut AnyObject =
                    unsafe { msg_send![notifications, objectAtIndex: index] };
                let request: *mut AnyObject = unsafe { msg_send![notification, request] };
                let actual: *mut AnyObject = unsafe { msg_send![request, identifier] };
                if !actual.is_null() {
                    let matches: Bool = unsafe { msg_send![actual, isEqualToString: &*expected] };
                    if matches.as_bool() {
                        found = true;
                        break;
                    }
                }
            }
        }
        let _ = sender.send(found);
    });
    unsafe {
        let _: () = msg_send![center, getDeliveredNotificationsWithCompletionHandler: &*completion];
    }
    receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "notification_verification_timeout".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn unbundled_dev_process_fails_without_touching_user_notifications() {
        if crate::macos_process_identity::current_executable_is_bundled_app() {
            return;
        }
        assert_eq!(
            deliver_notification_and_verify_blocking("OOMU", "", "Ready").unwrap_err(),
            "notification_application_bundle_required"
        );
    }
}
