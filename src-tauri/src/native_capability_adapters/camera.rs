use super::CameraPreviewEvidence;
use objc2::{msg_send, runtime::AnyClass, runtime::AnyObject, runtime::Bool};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::{ffi::c_void, sync::mpsc, time::Duration};

const PREVIEW_VISIBLE_FOR: Duration = Duration::from_millis(900);
const AV_AUTHORIZED: isize = 3;

struct CameraResources {
    session: usize,
    window: usize,
    preview_layer: usize,
    capture_outputs: usize,
}

pub(crate) async fn open_camera_preview_without_retention(
    app: &tauri::AppHandle,
) -> Result<CameraPreviewEvidence, String> {
    camera_permission_is_allowed()?;
    let (opened_tx, opened_rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = opened_tx.send(unsafe { open_preview_on_main_thread() });
    })
    .map_err(|_| "camera_preview_main_thread_unavailable".to_string())?;
    let resources = opened_rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "camera_preview_open_timeout".to_string())??;

    let resources = tauri::async_runtime::spawn_blocking(move || unsafe {
        start_session_off_main_thread(resources)
    })
    .await
    .map_err(|_| "camera_preview_start_interrupted".to_string())??;

    tokio::time::sleep(PREVIEW_VISIBLE_FOR).await;
    let capture_outputs = resources.capture_outputs;
    let resources = tauri::async_runtime::spawn_blocking(move || unsafe {
        stop_session_off_main_thread(resources)
    })
    .await
    .map_err(|_| "camera_preview_stop_interrupted".to_string())??;
    let (closed_tx, closed_rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = closed_tx.send(unsafe { close_preview_on_main_thread(resources) });
    })
    .map_err(|_| "camera_preview_main_thread_unavailable".to_string())?;
    let preview_closed = closed_rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "camera_preview_close_timeout".to_string())??;

    let evidence = CameraPreviewEvidence {
        preview_opened: true,
        preview_closed,
        preview_layer_attached: true,
        capture_outputs,
        frame_retained: false,
    };
    evidence
        .verified()
        .then_some(evidence)
        .ok_or_else(|| "camera_preview_postcondition_failed".to_string())
}

fn camera_permission_is_allowed() -> Result<(), String> {
    unsafe {
        let class = AnyClass::get(c"AVCaptureDevice")
            .ok_or_else(|| "camera_framework_unavailable".to_string())?;
        let media = NSString::from_str("vide");
        let status: isize = msg_send![class, authorizationStatusForMediaType: &*media];
        if status == AV_AUTHORIZED {
            Ok(())
        } else {
            Err("camera_permission_required".to_string())
        }
    }
}

unsafe fn open_preview_on_main_thread() -> Result<CameraResources, String> {
    let session_class = AnyClass::get(c"AVCaptureSession")
        .ok_or_else(|| "camera_framework_unavailable".to_string())?;
    let session: *mut AnyObject = unsafe { msg_send![session_class, new] };
    if session.is_null() {
        return Err("camera_session_unavailable".to_string());
    }
    let failure = |code: &str| {
        unsafe {
            let _: () = msg_send![session, release];
        }
        Err(code.to_string())
    };
    let device_class = AnyClass::get(c"AVCaptureDevice")
        .ok_or_else(|| "camera_framework_unavailable".to_string())?;
    let media = NSString::from_str("vide");
    let device: *mut AnyObject =
        unsafe { msg_send![device_class, defaultDeviceWithMediaType: &*media] };
    if device.is_null() {
        return failure("camera_device_unavailable");
    }
    let input_class = AnyClass::get(c"AVCaptureDeviceInput")
        .ok_or_else(|| "camera_framework_unavailable".to_string())?;
    let mut native_error: *mut AnyObject = std::ptr::null_mut();
    let input: *mut AnyObject =
        unsafe { msg_send![input_class, deviceInputWithDevice: device, error: &mut native_error] };
    if input.is_null() || !native_error.is_null() {
        return failure("camera_input_unavailable");
    }
    let can_add: Bool = unsafe { msg_send![session, canAddInput: input] };
    if !can_add.as_bool() {
        return failure("camera_input_unavailable");
    }
    unsafe {
        let _: () = msg_send![session, addInput: input];
    }

    let window_class = AnyClass::get(c"NSWindow")
        .ok_or_else(|| "camera_preview_window_unavailable".to_string())?;
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(480.0, 320.0));
    let allocated: *mut AnyObject = unsafe { msg_send![window_class, alloc] };
    let window: *mut AnyObject = unsafe {
        msg_send![allocated,
            initWithContentRect: rect,
            styleMask: 3_usize,
            backing: 2_usize,
            defer: false
        ]
    };
    if window.is_null() {
        return failure("camera_preview_window_unavailable");
    }
    let content_view: *mut AnyObject = unsafe { msg_send![window, contentView] };
    if content_view.is_null() {
        unsafe {
            let _: () = msg_send![window, close];
            let _: () = msg_send![window, release];
        }
        return failure("camera_preview_window_unavailable");
    }
    unsafe {
        let _: () = msg_send![content_view, setWantsLayer: true];
    }
    let root_layer: *mut AnyObject = unsafe { msg_send![content_view, layer] };
    let layer_class = AnyClass::get(c"AVCaptureVideoPreviewLayer")
        .ok_or_else(|| "camera_preview_layer_unavailable".to_string())?;
    let preview_layer: *mut AnyObject =
        unsafe { msg_send![layer_class, layerWithSession: session] };
    if root_layer.is_null() || preview_layer.is_null() {
        unsafe {
            let _: () = msg_send![window, close];
            let _: () = msg_send![window, release];
        }
        return failure("camera_preview_layer_unavailable");
    }
    let bounds: NSRect = unsafe { msg_send![content_view, bounds] };
    unsafe {
        let _: () = msg_send![preview_layer, setFrame: bounds];
        let _: () = msg_send![root_layer, addSublayer: preview_layer];
        let title = NSString::from_str("OOMU");
        let _: () = msg_send![window, setTitle: &*title];
        let _: () = msg_send![window, center];
        let _: () = msg_send![window, orderFrontRegardless];
    }
    let outputs: *mut AnyObject = unsafe { msg_send![session, outputs] };
    let capture_outputs = if outputs.is_null() {
        usize::MAX
    } else {
        unsafe { msg_send![outputs, count] }
    };
    if capture_outputs != 0 {
        unsafe {
            let _: () = msg_send![preview_layer, removeFromSuperlayer];
            let _: () = msg_send![window, close];
            let _: () = msg_send![window, release];
        }
        return failure("camera_preview_open_failed");
    }
    Ok(CameraResources {
        session: session.cast::<c_void>() as usize,
        window: window.cast::<c_void>() as usize,
        preview_layer: preview_layer.cast::<c_void>() as usize,
        capture_outputs,
    })
}

unsafe fn start_session_off_main_thread(
    resources: CameraResources,
) -> Result<CameraResources, String> {
    let session = resources.session as *mut AnyObject;
    if session.is_null() {
        return Err("camera_preview_state_invalid".to_string());
    }
    unsafe {
        let _: () = msg_send![session, startRunning];
    }
    let running: Bool = unsafe { msg_send![session, isRunning] };
    running
        .as_bool()
        .then_some(resources)
        .ok_or_else(|| "camera_preview_open_failed".to_string())
}

unsafe fn stop_session_off_main_thread(
    resources: CameraResources,
) -> Result<CameraResources, String> {
    let session = resources.session as *mut AnyObject;
    if session.is_null() {
        return Err("camera_preview_state_invalid".to_string());
    }
    unsafe {
        let _: () = msg_send![session, stopRunning];
    }
    let running: Bool = unsafe { msg_send![session, isRunning] };
    (!running.as_bool())
        .then_some(resources)
        .ok_or_else(|| "camera_preview_close_failed".to_string())
}

unsafe fn close_preview_on_main_thread(resources: CameraResources) -> Result<bool, String> {
    let session = resources.session as *mut AnyObject;
    let window = resources.window as *mut AnyObject;
    let preview_layer = resources.preview_layer as *mut AnyObject;
    if session.is_null() || window.is_null() || preview_layer.is_null() {
        return Err("camera_preview_state_invalid".to_string());
    }
    unsafe {
        let _: () = msg_send![preview_layer, removeFromSuperlayer];
        let _: () = msg_send![window, close];
    }
    let running: Bool = unsafe { msg_send![session, isRunning] };
    unsafe {
        let _: () = msg_send![window, release];
        let _: () = msg_send![session, release];
    }
    Ok(!running.as_bool())
}
