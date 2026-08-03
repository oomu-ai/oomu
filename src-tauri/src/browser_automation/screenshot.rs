use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::Manager;

const BROWSER_WEBVIEW_LABEL: &str = "oomu-browser-mod";

#[cfg(target_os = "macos")]
pub(super) async fn capture(app: &tauri::AppHandle, path: PathBuf) -> Result<PathBuf, String> {
    use block2::RcBlock;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError};
    use objc2_web_kit::WKWebView;

    let webview = app
        .get_webview(BROWSER_WEBVIEW_LABEL)
        .ok_or_else(|| "The controlled browser view is not open.".to_string())?;
    let (sender, receiver) = tokio::sync::oneshot::channel::<Result<Vec<u8>, String>>();
    let sender = std::sync::Mutex::new(Some(sender));
    webview
        .with_webview(move |platform| unsafe {
            let view: &WKWebView = &*platform.inner().cast();
            let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                let result = if image.is_null() {
                    let _ = error;
                    Err("Native browser screenshot failed.".to_string())
                } else {
                    let image = &*image;
                    image
                        .TIFFRepresentation()
                        .ok_or_else(|| {
                            "Native browser screenshot did not produce image data.".to_string()
                        })
                        .and_then(|tiff| {
                            NSBitmapImageRep::imageRepWithData(&tiff).ok_or_else(|| {
                                "Native browser screenshot encoding failed.".to_string()
                            })
                        })
                        .and_then(|bitmap| {
                            let properties = NSDictionary::new();
                            bitmap
                                .representationUsingType_properties(
                                    NSBitmapImageFileType::PNG,
                                    &properties,
                                )
                                .ok_or_else(|| {
                                    "Native browser screenshot PNG encoding failed.".to_string()
                                })
                        })
                        .map(|data| {
                            let mut bytes = vec![0_u8; data.length()];
                            if let Some(pointer) = std::ptr::NonNull::new(bytes.as_mut_ptr().cast())
                            {
                                data.getBytes_length(pointer, bytes.len());
                            }
                            bytes
                        })
                };
                if let Ok(mut guard) = sender.lock() {
                    if let Some(sender) = guard.take() {
                        let _ = sender.send(result);
                    }
                }
            });
            view.takeSnapshotWithConfiguration_completionHandler(None, &block);
        })
        .map_err(|error| format!("Native browser screenshot bridge failed: {error}"))?;
    let bytes = tokio::time::timeout(std::time::Duration::from_secs(8), receiver)
        .await
        .map_err(|_| "Native browser screenshot timed out.".to_string())?
        .map_err(|_| "Native browser screenshot callback was cancelled.".to_string())??;
    if bytes.is_empty() || bytes.len() as u64 > super::MAX_SCREENSHOT_BYTES {
        return Err("Native browser screenshot failed its bounded size check.".to_string());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to create screenshot evidence directory: {error}"))?;
    }
    fs::write(&path, &bytes)
        .map_err(|error| format!("Unable to persist screenshot evidence: {error}"))?;
    verify_png(&path)?;
    Ok(path)
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn capture(_app: &tauri::AppHandle, _path: PathBuf) -> Result<PathBuf, String> {
    Err("Native pixel capture is unavailable on this platform.".to_string())
}

fn verify_png(path: &Path) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|_| "Screenshot evidence is unavailable.".to_string())?;
    if metadata.len() == 0 || metadata.len() > super::MAX_SCREENSHOT_BYTES {
        return Err("Screenshot evidence failed size validation.".to_string());
    }
    let image = image::ImageReader::open(path)
        .map_err(|error| format!("Screenshot evidence cannot be opened: {error}"))?
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| format!("Screenshot evidence is not a valid image: {error}"))?;
    if image.width() == 0 || image.height() == 0 || image.width() > 8_192 || image.height() > 8_192
    {
        return Err("Screenshot evidence dimensions are invalid.".to_string());
    }
    Ok(())
}
