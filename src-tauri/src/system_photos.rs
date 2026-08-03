use crate::mcp_result::McpToolCallResult;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;

const PHOTOS_BACKEND: &str = "photokit";
const DEFAULT_PHOTO_LIMIT: u32 = 1;
const MAX_PHOTO_LIMIT: u32 = 20;
const PHOTO_READ_TIMEOUT_SECONDS: u64 = 75;
const PHOTO_AUTHORIZATION_TIMEOUT_SECONDS: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PhotoAuthorization {
    Authorized,
    Limited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemPhotoAsset {
    local_identifier: String,
    original_filename: Option<String>,
    creation_date: Option<String>,
    creation_timestamp_ms: Option<i64>,
    pixel_width: u64,
    pixel_height: u64,
    favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PhotoAssetCandidate {
    local_identifier: String,
    original_filename: Option<String>,
    creation_timestamp_ms: Option<i64>,
    pixel_width: u64,
    pixel_height: u64,
    favorite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PhotosFailure {
    code: &'static str,
    message: &'static str,
    retryable: bool,
}

impl PhotosFailure {
    const fn new(code: &'static str, message: &'static str, retryable: bool) -> Self {
        Self {
            code,
            message,
            retryable,
        }
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn read_system_photos(max_photos: Option<u32>) -> McpToolCallResult {
    read_system_photos_bounded(bounded_photo_limit(max_photos)).await
}

pub(crate) async fn read_system_photos_bounded(max_photos: u32) -> McpToolCallResult {
    let max_photos = bounded_photo_limit(Some(max_photos));
    match tokio::time::timeout(
        std::time::Duration::from_secs(PHOTO_READ_TIMEOUT_SECONDS),
        native_photo_assets(max_photos),
    )
    .await
    {
        Ok(Ok((authorization, candidates, has_more))) => {
            let photos = finalize_photo_assets(candidates, max_photos as usize);
            photos_success_result(authorization, photos, has_more)
        }
        Ok(Err(failure)) => photos_error_result(&failure),
        Err(_) => photos_error_result(&photos_read_timeout_failure()),
    }
}

fn photos_read_timeout_failure() -> PhotosFailure {
    PhotosFailure::new(
        "photos_read_timeout",
        "Photos took too long to respond. Try again.",
        true,
    )
}

pub(crate) fn photo_limit_from_arguments(arguments: &Value) -> Result<u32, String> {
    let object = arguments
        .as_object()
        .ok_or_else(|| "Photos arguments must be a JSON object.".to_string())?;
    let max_photos = object
        .get("max_photos")
        .or_else(|| object.get("maxPhotos"))
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| "Photos maxPhotos must be a positive whole number.".to_string())
        })
        .transpose()?;
    Ok(bounded_photo_limit(max_photos))
}

fn bounded_photo_limit(limit: Option<u32>) -> u32 {
    limit
        .unwrap_or(DEFAULT_PHOTO_LIMIT)
        .clamp(1, MAX_PHOTO_LIMIT)
}

fn finalize_photo_assets(
    mut candidates: Vec<PhotoAssetCandidate>,
    limit: usize,
) -> Vec<SystemPhotoAsset> {
    candidates.sort_by(|left, right| {
        right
            .creation_timestamp_ms
            .cmp(&left.creation_timestamp_ms)
            .then_with(|| left.local_identifier.cmp(&right.local_identifier))
    });
    candidates
        .into_iter()
        .take(limit)
        .map(|candidate| SystemPhotoAsset {
            creation_date: candidate
                .creation_timestamp_ms
                .and_then(rfc3339_from_millis),
            creation_timestamp_ms: candidate.creation_timestamp_ms,
            local_identifier: candidate.local_identifier,
            original_filename: candidate.original_filename,
            pixel_width: candidate.pixel_width,
            pixel_height: candidate.pixel_height,
            favorite: candidate.favorite,
        })
        .collect()
}

fn rfc3339_from_millis(timestamp_ms: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .map(|date| date.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn bounded_original_filename(value: &str) -> Option<String> {
    let filename = value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim()
        .replace('\0', "");
    let bounded = filename.chars().take(255).collect::<String>();
    (!bounded.is_empty() && bounded != "." && bounded != "..").then_some(bounded)
}

fn bounded_local_identifier(value: &str) -> String {
    value.chars().take(512).collect()
}

fn photos_success_result(
    authorization: PhotoAuthorization,
    photos: Vec<SystemPhotoAsset>,
    truncated: bool,
) -> McpToolCallResult {
    let returned_count = photos.len();
    let structured = serde_json::json!({
        "backend": PHOTOS_BACKEND,
        "code": "photos_read_ok",
        "authorization": authorization,
        "photos": photos,
        "returnedCount": returned_count,
        "truncated": truncated,
    });
    McpToolCallResult {
        content: vec![serde_json::json!({
            "type": "text",
            "text": serde_json::to_string_pretty(&structured["photos"])
                .unwrap_or_else(|_| "[]".to_string()),
        })],
        structured_content: Some(structured),
        is_error: false,
        meta: None,
        raw: None,
    }
}

fn photos_error_result(failure: &PhotosFailure) -> McpToolCallResult {
    let structured = serde_json::json!({
        "backend": PHOTOS_BACKEND,
        "code": failure.code,
        "message": failure.message,
        "retryable": failure.retryable,
        "photos": [],
    });
    McpToolCallResult {
        content: vec![serde_json::json!({"type": "text", "text": failure.message})],
        structured_content: Some(structured),
        is_error: true,
        meta: None,
        raw: None,
    }
}

#[cfg(target_os = "macos")]
async fn native_photo_assets(
    max_photos: u32,
) -> Result<(PhotoAuthorization, Vec<PhotoAssetCandidate>, bool), PhotosFailure> {
    use block2::RcBlock;
    use objc2_photos::{
        PHAccessLevel, PHAsset, PHAssetMediaType, PHAssetResource, PHAuthorizationStatus,
        PHFetchOptions, PHPhotoLibrary,
    };
    use std::sync::Mutex;

    let mut status =
        unsafe { PHPhotoLibrary::authorizationStatusForAccessLevel(PHAccessLevel::ReadWrite) };
    if status == PHAuthorizationStatus::NotDetermined {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let sender = Mutex::new(Some(sender));
        {
            let handler = RcBlock::new(move |next_status: PHAuthorizationStatus| {
                if let Ok(mut sender) = sender.lock() {
                    if let Some(sender) = sender.take() {
                        let _ = sender.send(next_status);
                    }
                }
            });
            unsafe {
                PHPhotoLibrary::requestAuthorizationForAccessLevel_handler(
                    PHAccessLevel::ReadWrite,
                    &handler,
                );
            }
        }
        status = tokio::time::timeout(
            std::time::Duration::from_secs(PHOTO_AUTHORIZATION_TIMEOUT_SECONDS),
            receiver,
        )
        .await
        .map_err(|_| {
            PhotosFailure::new(
                "photos_authorization_timeout",
                "Photos took too long to respond. Try again.",
                true,
            )
        })?
        .map_err(|_| {
            PhotosFailure::new(
                "photos_authorization_cancelled",
                "Photos did not finish the access request. Try again.",
                true,
            )
        })?;
    }

    let authorization = if status == PHAuthorizationStatus::Authorized {
        PhotoAuthorization::Authorized
    } else if status == PHAuthorizationStatus::Limited {
        PhotoAuthorization::Limited
    } else if status == PHAuthorizationStatus::Denied {
        return Err(PhotosFailure::new(
            "photos_permission_denied",
            "Photos access is off. Allow OOMU in System Settings, then try again.",
            false,
        ));
    } else if status == PHAuthorizationStatus::Restricted {
        return Err(PhotosFailure::new(
            "photos_permission_restricted",
            "Photos access is restricted on this Mac.",
            false,
        ));
    } else {
        return Err(PhotosFailure::new(
            "photos_authorization_unknown",
            "Photos access could not be verified.",
            true,
        ));
    };

    // PhotoKit fetches are synchronous. Keep them off the async runtime so the
    // whole-read deadline above can still resolve with a typed failure if the
    // photo library stalls.
    let candidates = tokio::task::spawn_blocking(move || unsafe {
        use objc2_foundation::{NSArray, NSSortDescriptor, NSString};

        let library = PHPhotoLibrary::sharedPhotoLibrary();
        if library.unavailabilityReason().is_some() {
            return Err(PhotosFailure::new(
                "photos_library_unavailable",
                "The photo library is not available right now.",
                true,
            ));
        }

        let options = PHFetchOptions::new();
        let key = NSString::from_str("creationDate");
        let descriptor = NSSortDescriptor::sortDescriptorWithKey_ascending(Some(&key), false);
        let descriptors = NSArray::from_slice(&[&*descriptor]);
        options.setSortDescriptors(Some(&descriptors));
        options.setIncludeHiddenAssets(false);
        options.setFetchLimit(max_photos.saturating_add(1) as usize);
        let result =
            PHAsset::fetchAssetsWithMediaType_options(PHAssetMediaType::Image, Some(&options));
        let count = result.count();
        let mut candidates = Vec::with_capacity(count.min(max_photos as usize + 1));
        for index in 0..count {
            let asset = result.objectAtIndex(index);
            let resources = PHAssetResource::assetResourcesForAsset(&asset);
            let original_filename = resources.firstObject().and_then(|resource| {
                bounded_original_filename(&resource.originalFilename().to_string())
            });
            let timestamp_ms = asset.creationDate().and_then(|date| {
                let seconds = date.timeIntervalSince1970();
                seconds
                    .is_finite()
                    .then(|| (seconds * 1_000.0).round() as i64)
            });
            candidates.push(PhotoAssetCandidate {
                local_identifier: bounded_local_identifier(&asset.localIdentifier().to_string()),
                original_filename,
                creation_timestamp_ms: timestamp_ms,
                pixel_width: asset.pixelWidth() as u64,
                pixel_height: asset.pixelHeight() as u64,
                favorite: asset.isFavorite(),
            });
        }
        Ok(candidates)
    })
    .await
    .map_err(|_| {
        PhotosFailure::new(
            "photos_library_read_failed",
            "Your photo library couldn't be read right now. Try again.",
            true,
        )
    })??;
    let has_more = candidates.len() > max_photos as usize;
    Ok((authorization, candidates, has_more))
}

#[cfg(not(target_os = "macos"))]
async fn native_photo_assets(
    _max_photos: u32,
) -> Result<(PhotoAuthorization, Vec<PhotoAssetCandidate>, bool), PhotosFailure> {
    Err(PhotosFailure::new(
        "photos_unavailable",
        "Photos access is available only in the OOMU app on macOS.",
        false,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(identifier: &str, timestamp_ms: Option<i64>) -> PhotoAssetCandidate {
        PhotoAssetCandidate {
            local_identifier: identifier.to_string(),
            original_filename: Some(format!("{identifier}.jpg")),
            creation_timestamp_ms: timestamp_ms,
            pixel_width: 1_920,
            pixel_height: 1_080,
            favorite: false,
        }
    }

    #[test]
    fn photo_limits_are_bounded_before_native_access() {
        assert_eq!(bounded_photo_limit(None), 1);
        assert_eq!(bounded_photo_limit(Some(0)), 1);
        assert_eq!(bounded_photo_limit(Some(8)), 8);
        assert_eq!(bounded_photo_limit(Some(200)), 20);
        assert_eq!(
            photo_limit_from_arguments(&serde_json::json!({"maxPhotos": 3})).unwrap(),
            3
        );
        assert!(photo_limit_from_arguments(&serde_json::json!({"maxPhotos": "all"})).is_err());
    }

    #[test]
    fn newest_photo_metadata_is_sorted_and_bounded() {
        let photos = finalize_photo_assets(
            vec![
                candidate("old", Some(1_000)),
                candidate("unknown", None),
                candidate("newest", Some(3_000)),
                candidate("middle", Some(2_000)),
            ],
            2,
        );
        assert_eq!(photos.len(), 2);
        assert_eq!(photos[0].local_identifier, "newest");
        assert_eq!(photos[1].local_identifier, "middle");
        assert_eq!(
            photos[0].creation_date.as_deref(),
            Some("1970-01-01T00:00:03.000Z")
        );
    }

    #[test]
    fn typed_results_never_expose_raw_native_payloads() {
        let success = photos_success_result(
            PhotoAuthorization::Authorized,
            vec![SystemPhotoAsset {
                local_identifier: "asset".to_string(),
                original_filename: Some("IMG_0001.JPG".to_string()),
                creation_date: None,
                creation_timestamp_ms: None,
                pixel_width: 10,
                pixel_height: 10,
                favorite: false,
            }],
            false,
        );
        assert!(!success.is_error);
        assert!(success.raw.is_none());
        assert_eq!(
            success.structured_content.unwrap()["code"],
            "photos_read_ok"
        );

        let error = photos_error_result(&PhotosFailure::new(
            "photos_permission_denied",
            "Photos access is off.",
            false,
        ));
        assert!(error.is_error);
        assert!(error.raw.is_none());
        assert_eq!(
            error.structured_content.unwrap()["code"],
            "photos_permission_denied"
        );
    }

    #[test]
    fn original_filenames_are_bounded_and_never_expose_paths() {
        assert_eq!(
            bounded_original_filename("/private/library/IMG_0001.JPG").as_deref(),
            Some("IMG_0001.JPG")
        );
        assert_eq!(bounded_original_filename("../").as_deref(), None);
        assert_eq!(
            bounded_original_filename(&format!("{}.jpg", "a".repeat(300)))
                .unwrap()
                .chars()
                .count(),
            255
        );
        assert_eq!(bounded_local_identifier(&"a".repeat(600)).len(), 512);
    }

    #[test]
    fn overall_photo_read_deadline_outlives_authorization_deadline() {
        assert_eq!(PHOTO_READ_TIMEOUT_SECONDS, 75);
        assert!(PHOTO_AUTHORIZATION_TIMEOUT_SECONDS < PHOTO_READ_TIMEOUT_SECONDS);
        let result = photos_error_result(&photos_read_timeout_failure());
        assert!(result.is_error);
        assert_eq!(
            result.structured_content.unwrap()["code"],
            "photos_read_timeout"
        );
    }
}
