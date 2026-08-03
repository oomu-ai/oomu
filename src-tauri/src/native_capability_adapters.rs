#[cfg(target_os = "macos")]
mod camera;
#[cfg(target_os = "macos")]
mod full_disk_access;
#[cfg(target_os = "macos")]
mod notification;
#[cfg(target_os = "macos")]
mod screen_capture;

#[cfg(target_os = "macos")]
pub(crate) use camera::open_camera_preview_without_retention;
#[cfg(target_os = "macos")]
pub(crate) use full_disk_access::probe_full_disk_access;
#[cfg(target_os = "macos")]
pub(crate) use notification::{
    deliver_notification_and_verify, deliver_notification_and_verify_blocking,
};
#[cfg(target_os = "macos")]
pub(crate) use screen_capture::capture_disposable_oomu_window;

#[derive(Clone, Debug)]
pub(crate) struct ScreenCaptureCopy {
    pub title: String,
    pub body: String,
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn open_camera_preview_without_retention(
    _app: &tauri::AppHandle,
) -> Result<CameraPreviewEvidence, String> {
    Err("camera_preview_unsupported".to_string())
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn deliver_notification_and_verify(
    _title: String,
    _subtitle: String,
    _body: String,
) -> Result<NotificationDeliveryEvidence, String> {
    Err("notification_delivery_unsupported".to_string())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn deliver_notification_and_verify_blocking(
    _title: &str,
    _subtitle: &str,
    _body: &str,
) -> Result<NotificationDeliveryEvidence, String> {
    Err("notification_delivery_unsupported".to_string())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn probe_full_disk_access() -> FullDiskAccessProbe {
    FullDiskAccessProbe::Unsupported
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn capture_disposable_oomu_window(
    _app: &tauri::AppHandle,
    _copy: ScreenCaptureCopy,
) -> Result<ScreenCaptureEvidence, String> {
    Err("screen_capture_unsupported".to_string())
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CameraPreviewEvidence {
    pub preview_opened: bool,
    pub preview_closed: bool,
    pub preview_layer_attached: bool,
    pub capture_outputs: usize,
    pub frame_retained: bool,
}

impl CameraPreviewEvidence {
    pub(crate) fn verified(&self) -> bool {
        self.preview_opened
            && self.preview_closed
            && self.preview_layer_attached
            && self.capture_outputs == 0
            && !self.frame_retained
    }
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationDeliveryEvidence {
    pub notification_id: String,
    pub submitted: bool,
    pub delivered: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScreenCaptureEvidence {
    pub width: u32,
    pub height: u32,
    pub png_byte_count: usize,
    pub pixel_digest_sha256: String,
    pub non_uniform_pixels: bool,
    pub captured_window_count: usize,
    pub retained_byte_count: usize,
}

impl ScreenCaptureEvidence {
    pub(crate) fn verified(&self) -> bool {
        (320..=2_048).contains(&self.width)
            && (180..=2_048).contains(&self.height)
            && (2_048..=16 * 1024 * 1024).contains(&self.png_byte_count)
            && self.pixel_digest_sha256.len() == 64
            && self
                .pixel_digest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            && self.non_uniform_pixels
            && self.captured_window_count == 1
            && self.retained_byte_count == 0
    }
}

impl NotificationDeliveryEvidence {
    pub(crate) fn verified(&self) -> bool {
        self.submitted && self.delivered && !self.notification_id.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FullDiskAccessProbe {
    Allowed { bytes_read: usize },
    PermissionRequired,
    Stale,
    Unsupported,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_receipt_requires_open_close_and_zero_capture_outputs() {
        let evidence = CameraPreviewEvidence {
            preview_opened: true,
            preview_closed: true,
            preview_layer_attached: true,
            capture_outputs: 0,
            frame_retained: false,
        };
        assert!(evidence.verified());
        assert!(!CameraPreviewEvidence {
            capture_outputs: 1,
            ..evidence.clone()
        }
        .verified());
        assert!(!CameraPreviewEvidence {
            preview_closed: false,
            ..evidence
        }
        .verified());
    }

    #[test]
    fn notification_receipt_requires_submission_and_delivered_postcondition() {
        let evidence = NotificationDeliveryEvidence {
            notification_id: "oomu-native-notification-1".to_string(),
            submitted: true,
            delivered: true,
        };
        assert!(evidence.verified());
        assert!(!NotificationDeliveryEvidence {
            delivered: false,
            ..evidence
        }
        .verified());
    }

    #[test]
    fn screen_capture_receipt_requires_bounded_verified_pixels_without_retention() {
        let evidence = ScreenCaptureEvidence {
            width: 520,
            height: 280,
            png_byte_count: 8_192,
            pixel_digest_sha256: "a".repeat(64),
            non_uniform_pixels: true,
            captured_window_count: 1,
            retained_byte_count: 0,
        };
        assert!(evidence.verified());
        assert!(!ScreenCaptureEvidence {
            non_uniform_pixels: false,
            ..evidence.clone()
        }
        .verified());
        assert!(!ScreenCaptureEvidence {
            retained_byte_count: 1,
            ..evidence
        }
        .verified());
    }
}
