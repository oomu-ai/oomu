use super::{ScreenCaptureCopy, ScreenCaptureEvidence};
use objc2::{msg_send, runtime::AnyClass, runtime::AnyObject, AnyThread};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
use objc2_core_graphics::{CGImage, CGRectNull, CGWindowImageOption, CGWindowListOption};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};
use std::{ffi::c_void, ptr::NonNull, sync::mpsc, time::Duration};

const CAPTURE_SETTLE_TIME: Duration = Duration::from_millis(180);
const WINDOW_WIDTH: f64 = 520.0;
const WINDOW_HEIGHT: f64 = 280.0;

pub(crate) async fn capture_disposable_oomu_window(
    app: &tauri::AppHandle,
    copy: ScreenCaptureCopy,
) -> Result<ScreenCaptureEvidence, String> {
    let window = open_window(app, copy)?;
    tokio::time::sleep(CAPTURE_SETTLE_TIME).await;
    let captured = capture_and_close_window(app, window)?;
    captured
        .verified()
        .then_some(captured)
        .ok_or_else(|| "screen_capture_postcondition_failed".to_string())
}

fn open_window(app: &tauri::AppHandle, copy: ScreenCaptureCopy) -> Result<usize, String> {
    let (sender, receiver) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(unsafe { open_window_on_main_thread(&copy) });
    })
    .map_err(|_| "screen_capture_main_thread_unavailable".to_string())?;
    receiver
        .recv_timeout(Duration::from_secs(8))
        .map_err(|_| "screen_capture_window_open_timeout".to_string())?
}

fn capture_and_close_window(
    app: &tauri::AppHandle,
    window: usize,
) -> Result<ScreenCaptureEvidence, String> {
    let (sender, receiver) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(unsafe { capture_and_close_on_main_thread(window) });
    })
    .map_err(|_| "screen_capture_main_thread_unavailable".to_string())?;
    receiver
        .recv_timeout(Duration::from_secs(8))
        .map_err(|_| "screen_capture_window_timeout".to_string())?
}

unsafe fn open_window_on_main_thread(copy: &ScreenCaptureCopy) -> Result<usize, String> {
    let class = AnyClass::get(c"NSWindow")
        .ok_or_else(|| "screen_capture_window_unavailable".to_string())?;
    let frame = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(WINDOW_WIDTH, WINDOW_HEIGHT),
    );
    let allocated: *mut AnyObject = unsafe { msg_send![class, alloc] };
    let window: *mut AnyObject = unsafe {
        msg_send![allocated,
            initWithContentRect: frame,
            styleMask: 3_usize,
            backing: 2_usize,
            defer: false
        ]
    };
    if window.is_null() {
        return Err("screen_capture_window_unavailable".to_string());
    }
    let title = NSString::from_str(&copy.title);
    let content: *mut AnyObject = unsafe { msg_send![window, contentView] };
    if content.is_null() {
        unsafe {
            let _: () = msg_send![window, close];
            let _: () = msg_send![window, release];
        }
        return Err("screen_capture_window_unavailable".to_string());
    }
    unsafe {
        let _: () = msg_send![window, setTitle: &*title];
        let _: () = msg_send![window, center];
        add_explanatory_label(content, &copy.body)?;
        let _: () = msg_send![window, orderFrontRegardless];
        let _: () = msg_send![window, displayIfNeeded];
    }
    Ok(window.cast::<c_void>() as usize)
}

unsafe fn add_explanatory_label(content: *mut AnyObject, body: &str) -> Result<(), String> {
    let class = AnyClass::get(c"NSTextField")
        .ok_or_else(|| "screen_capture_window_unavailable".to_string())?;
    let text = NSString::from_str(body);
    let label: *mut AnyObject = unsafe { msg_send![class, labelWithString: &*text] };
    if label.is_null() {
        return Err("screen_capture_window_unavailable".to_string());
    }
    let frame = NSRect::new(NSPoint::new(48.0, 108.0), NSSize::new(424.0, 48.0));
    unsafe {
        let _: () = msg_send![label, setFrame: frame];
        let _: () = msg_send![content, addSubview: label];
    }
    Ok(())
}

unsafe fn capture_and_close_on_main_thread(window: usize) -> Result<ScreenCaptureEvidence, String> {
    let window = window as *mut AnyObject;
    if window.is_null() {
        return Err("screen_capture_window_unavailable".to_string());
    }
    let window_id: isize = unsafe { msg_send![window, windowNumber] };
    let capture = capture_exact_window(window_id);
    unsafe {
        let _: () = msg_send![window, orderOut: std::ptr::null::<AnyObject>()];
        let _: () = msg_send![window, close];
        let _: () = msg_send![window, release];
    }
    capture
}

#[allow(deprecated)]
fn capture_exact_window(window_id: isize) -> Result<ScreenCaptureEvidence, String> {
    let window_id = u32::try_from(window_id)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "screen_capture_window_unavailable".to_string())?;
    let image = objc2_core_graphics::CGWindowListCreateImage(
        unsafe { CGRectNull },
        CGWindowListOption::OptionIncludingWindow,
        window_id,
        CGWindowImageOption::BoundsIgnoreFraming | CGWindowImageOption::BestResolution,
    )
    .ok_or_else(|| "screen_capture_pixels_unavailable".to_string())?;
    let width = u32::try_from(CGImage::width(Some(&image)))
        .map_err(|_| "screen_capture_dimensions_invalid".to_string())?;
    let height = u32::try_from(CGImage::height(Some(&image)))
        .map_err(|_| "screen_capture_dimensions_invalid".to_string())?;
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
    let properties = NSDictionary::new();
    let data = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
    }
    .ok_or_else(|| "screen_capture_encoding_failed".to_string())?;
    let mut png = vec![0_u8; data.length()];
    if png.is_empty() {
        return Err("screen_capture_encoding_failed".to_string());
    }
    let pointer = NonNull::new(png.as_mut_ptr().cast())
        .ok_or_else(|| "screen_capture_encoding_failed".to_string())?;
    unsafe { data.getBytes_length(pointer, png.len()) };
    verify_pixels(width, height, &png)
}

fn verify_pixels(width: u32, height: u32, png: &[u8]) -> Result<ScreenCaptureEvidence, String> {
    let decoded = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|_| "screen_capture_pixels_invalid".to_string())?
        .to_rgba8();
    if decoded.width() != width || decoded.height() != height {
        return Err("screen_capture_dimensions_invalid".to_string());
    }
    let first = decoded
        .pixels()
        .next()
        .ok_or_else(|| "screen_capture_pixels_invalid".to_string())?;
    let non_uniform_pixels = decoded.pixels().step_by(17).any(|pixel| pixel != first);
    let evidence = ScreenCaptureEvidence {
        width,
        height,
        png_byte_count: png.len(),
        pixel_digest_sha256: crate::foundation::digest::sha256_hex(png),
        non_uniform_pixels,
        captured_window_count: 1,
        retained_byte_count: 0,
    };
    evidence
        .verified()
        .then_some(evidence)
        .ok_or_else(|| "screen_capture_postcondition_failed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_or_fabricated_capture_cannot_pass_pixel_verification() {
        assert!(verify_pixels(520, 280, b"not-a-png").is_err());
    }
}
