use super::*;
use crate::tools::native_operation_receipt::NativeScreenCaptureProof;

pub(super) async fn execute(
    app: &tauri::AppHandle,
    arguments: &Value,
    persistence: &PersistenceEngine,
) -> Result<McpToolCallResult, String> {
    if !arguments.as_object().is_some_and(serde_json::Map::is_empty) {
        return Err("screen_capture_arguments_invalid".to_string());
    }
    require_permission().await?;
    let copy = localized_copy(persistence)?;
    let native = crate::native_capability_adapters::capture_disposable_oomu_window(
        app,
        crate::native_capability_adapters::ScreenCaptureCopy {
            title: copy.title,
            body: copy.body,
        },
    )
    .await?;
    Ok(result(
        &copy.result,
        serde_json::json!({
            "status": "capture_verified",
            "evidenceKind": "disposable_window_capture",
            "verified": native.verified(),
            "method": "core_graphics_exact_window",
            "requestingProcessId": std::process::id(),
            "capturedWindowCount": native.captured_window_count,
            "width": native.width,
            "height": native.height,
            "pngByteCount": native.png_byte_count,
            "pixelDigestSha256": native.pixel_digest_sha256,
            "nonUniformPixels": native.non_uniform_pixels,
            "retainedByteCount": native.retained_byte_count,
        }),
    ))
}

struct LocalizedScreenCaptureCopy {
    title: String,
    body: String,
    result: String,
}

fn localized_copy(persistence: &PersistenceEngine) -> Result<LocalizedScreenCaptureCopy, String> {
    let state = crate::settings::locale_state_for_engine(persistence, None)?;
    let text = |key: &str| {
        state
            .translations
            .pointer(&format!("/sprint_301/screen_capture/{key}"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "screen_capture_copy_unavailable".to_string())
    };
    Ok(LocalizedScreenCaptureCopy {
        title: text("window_title")?,
        body: text("window_body")?,
        result: text("result")?,
    })
}

async fn require_permission() -> Result<(), String> {
    use crate::macos_permission_broker::MacosPermissionState;
    let permission = crate::macos_permission_broker::status_for_operation("screen_capture").await;
    matches!(
        permission.state,
        MacosPermissionState::Allowed
            | MacosPermissionState::Limited
            | MacosPermissionState::WhenUsed
    )
    .then_some(())
    .ok_or_else(|| "screen_capture_permission_required".to_string())
}

pub(super) fn evidence(result: &McpToolCallResult) -> NativePostconditionEvidence {
    let proof = result.structured_content.as_ref().and_then(capture_proof);
    let verified = result
        .structured_content
        .as_ref()
        .and_then(|value| value.get("verified"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && proof.as_ref().is_some_and(valid_capture_proof);
    NativePostconditionEvidence {
        evidence_kind: "disposable_window_capture",
        operation_succeeded: !result.is_error,
        verified,
        bounded_count: verified.then_some(1),
        truncated: Some(false),
        native_result_code: Some(if verified {
            "capture_verified".to_string()
        } else {
            "capture_unverified".to_string()
        }),
        durable_operation_binding: None,
        capture_proof: proof,
    }
}

fn capture_proof(value: &Value) -> Option<NativeScreenCaptureProof> {
    let method = (value.get("method")?.as_str()? == "core_graphics_exact_window")
        .then_some("core_graphics_exact_window")?;
    Some(NativeScreenCaptureProof {
        method,
        requesting_process_id: u32::try_from(value.get("requestingProcessId")?.as_u64()?).ok()?,
        captured_window_count: count(value, "capturedWindowCount")?,
        width: u32::try_from(value.get("width")?.as_u64()?).ok()?,
        height: u32::try_from(value.get("height")?.as_u64()?).ok()?,
        png_byte_count: count(value, "pngByteCount")?,
        pixel_digest_sha256: value.get("pixelDigestSha256")?.as_str()?.to_string(),
        non_uniform_pixels: value.get("nonUniformPixels")?.as_bool()?,
        retained_byte_count: count(value, "retainedByteCount")?,
    })
}

fn count(value: &Value, key: &str) -> Option<usize> {
    usize::try_from(value.get(key)?.as_u64()?).ok()
}

fn valid_capture_proof(proof: &NativeScreenCaptureProof) -> bool {
    proof.requesting_process_id == std::process::id()
        && proof.captured_window_count == 1
        && proof.width >= 320
        && proof.height >= 180
        && proof.png_byte_count >= 2_048
        && proof.pixel_digest_sha256.len() == 64
        && proof.non_uniform_pixels
        && proof.retained_byte_count == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_proof_rejects_wrong_process_or_retained_pixels() {
        let value = serde_json::json!({
            "method": "core_graphics_exact_window",
            "requestingProcessId": std::process::id(),
            "capturedWindowCount": 1,
            "width": 520,
            "height": 280,
            "pngByteCount": 4096,
            "pixelDigestSha256": "a".repeat(64),
            "nonUniformPixels": true,
            "retainedByteCount": 0,
        });
        let proof = capture_proof(&value).unwrap();
        assert!(valid_capture_proof(&proof));
        assert!(!valid_capture_proof(&NativeScreenCaptureProof {
            retained_byte_count: 1,
            ..proof
        }));
    }
}
