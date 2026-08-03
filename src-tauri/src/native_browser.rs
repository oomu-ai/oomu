use crate::browser_proxy::{start_browser_connect_proxy, BrowserProxyHandle};
use crate::foundation::clock::unix_time_ms_u64 as unix_time_ms;
use crate::network_policy::{
    resolve_destination, revalidate_destination, validate_browser_navigation_blocking,
    CanonicalDestination, DestinationTransport,
};
use crate::shield_gate::{self, RequestedAction, ShieldApprovalManager};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Manager;

pub(crate) const BROWSER_WEBVIEW_LABEL: &str = "oomu-browser-mod";
const BROWSER_AUTHORIZATION_TTL_MS: u64 = 60_000;
const BROWSER_TOKEN_BYTES: usize = 32;
const MAX_BROWSER_VIEW_DIMENSION: f64 = 8192.0;
const BROWSER_BRIDGE_ACK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Default)]
pub struct NativeBrowserManager {
    inner: Arc<Mutex<NativeBrowserState>>,
}

#[derive(Default)]
struct NativeBrowserState {
    pending: HashMap<String, PendingBrowserAuthorization>,
    active: Option<ActiveBrowserSession>,
    epoch: u64,
    pending_downloads: HashMap<String, NativeBrowserDownload>,
    completed_downloads: Vec<NativeBrowserDownload>,
}

struct PendingBrowserAuthorization {
    destination: CanonicalDestination,
    expires_at_ms: u64,
    epoch: u64,
}

struct ActiveBrowserSession {
    destination: CanonicalDestination,
    _proxy: BrowserProxyHandle,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeBrowserAutomationBinding {
    pub canonical_url: String,
    pub canonical_origin: String,
    pub destination_binding: String,
    pub epoch: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeBrowserDownload {
    pub download_id: String,
    pub source_url: String,
    pub private_path: PathBuf,
    pub file_name: String,
    pub completed: bool,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserNavigationAuthorization {
    approval_token: String,
    canonical_url: String,
    canonical_origin: String,
    destination_binding: String,
    expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeBrowserBridgeError {
    code: &'static str,
    stage: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeBrowserBridgeReady {
    status: &'static str,
    canonical_url: String,
}

impl NativeBrowserBridgeError {
    fn new(code: &'static str, stage: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            stage,
            message: message.into(),
        }
    }

    fn authorization(error: crate::shield_gate::ShieldGateError) -> Self {
        let code = match error.code {
            "shield_approval_denied" => "browser_authorization_denied",
            "shield_approval_timeout" => "browser_dispatch_timeout",
            _ => "browser_command_unavailable",
        };
        Self::new(
            code,
            "authorization",
            "Browser permission could not be completed.",
        )
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeBrowserBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl NativeBrowserManager {
    fn epoch(&self) -> Result<u64, String> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| "Native browser state is unavailable.".to_string())?
            .epoch)
    }

    fn insert_authorization(
        &self,
        destination: CanonicalDestination,
        expected_epoch: u64,
    ) -> Result<BrowserNavigationAuthorization, String> {
        let now = unix_time_ms();
        let expires_at_ms = now.saturating_add(BROWSER_AUTHORIZATION_TTL_MS);
        let approval_token = generate_browser_token();
        let response = BrowserNavigationAuthorization {
            approval_token: approval_token.clone(),
            canonical_url: destination.canonical_url().to_string(),
            canonical_origin: destination.canonical_origin().to_string(),
            destination_binding: destination.binding_fingerprint().to_string(),
            expires_at_ms,
        };
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "Native browser authorization state is unavailable.".to_string())?;
        if state.epoch != expected_epoch {
            return Err(
                "Native browser route changed while approval was pending; approval was revoked."
                    .to_string(),
            );
        }
        state
            .pending
            .retain(|_, pending| pending.expires_at_ms >= now);
        state.pending.insert(
            approval_token,
            PendingBrowserAuthorization {
                destination,
                expires_at_ms,
                epoch: expected_epoch,
            },
        );
        Ok(response)
    }

    fn consume_authorization(&self, token: &str) -> Result<(CanonicalDestination, u64), String> {
        let token = token.trim();
        if token.is_empty() {
            return Err("Native browser approval token must be non-empty.".to_string());
        }
        let now = unix_time_ms();
        let pending = self
            .inner
            .lock()
            .map_err(|_| "Native browser authorization state is unavailable.".to_string())?
            .pending
            .remove(token)
            .ok_or_else(|| {
                "Native browser approval is missing, expired, or already consumed.".to_string()
            })?;
        if pending.expires_at_ms < now {
            return Err("Native browser approval has expired.".to_string());
        }
        let current_epoch = self.epoch()?;
        if current_epoch != pending.epoch {
            return Err(
                "Native browser route changed after approval; approval was revoked.".to_string(),
            );
        }
        Ok((pending.destination, pending.epoch))
    }

    fn set_active_if_epoch(
        &self,
        destination: CanonicalDestination,
        proxy: BrowserProxyHandle,
        expected_epoch: u64,
    ) -> Result<bool, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "Native browser state is unavailable.".to_string())?;
        if state.epoch != expected_epoch {
            return Ok(false);
        }
        state.active = Some(ActiveBrowserSession {
            destination,
            _proxy: proxy,
        });
        Ok(true)
    }

    pub(crate) fn active(&self) -> Result<CanonicalDestination, String> {
        self.inner
            .lock()
            .map_err(|_| "Native browser state is unavailable.".to_string())?
            .active
            .as_ref()
            .map(|active| active.destination.clone())
            .ok_or_else(|| "No native browser destination is active.".to_string())
    }

    fn clear_active(&self) {
        if let Ok(mut state) = self.inner.lock() {
            state.active = None;
            state.pending.clear();
            state.epoch = state.epoch.wrapping_add(1);
        }
    }

    pub(crate) fn automation_binding(&self) -> Result<NativeBrowserAutomationBinding, String> {
        let state = self
            .inner
            .lock()
            .map_err(|_| "Native browser state is unavailable.".to_string())?;
        let active = state
            .active
            .as_ref()
            .ok_or_else(|| "No native browser destination is active.".to_string())?;
        Ok(NativeBrowserAutomationBinding {
            canonical_url: active.destination.canonical_url().to_string(),
            canonical_origin: active.destination.canonical_origin().to_string(),
            destination_binding: active.destination.binding_fingerprint().to_string(),
            epoch: state.epoch,
        })
    }

    fn begin_quarantined_download(
        &self,
        source_url: String,
        quarantine_root: &std::path::Path,
    ) -> Result<PathBuf, String> {
        fs::create_dir_all(quarantine_root)
            .map_err(|error| format!("Unable to prepare browser download quarantine: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(quarantine_root, fs::Permissions::from_mode(0o700)).map_err(
                |error| format!("Unable to protect browser download quarantine: {error}"),
            )?;
        }
        let file_name = source_url
            .parse::<url::Url>()
            .ok()
            .and_then(|url| url.path_segments()?.next_back().map(str::to_string))
            .filter(|name| !name.is_empty())
            .map(|name| sanitize_download_name(&name))
            .unwrap_or_else(|| "download.bin".to_string());
        let download_id = format!("download_{}", generate_browser_token());
        let private_path = quarantine_root.join(format!("{download_id}-{file_name}"));
        let record = NativeBrowserDownload {
            download_id,
            source_url: source_url.clone(),
            private_path: private_path.clone(),
            file_name,
            completed: false,
            success: false,
        };
        self.inner
            .lock()
            .map_err(|_| "Native browser download state is unavailable.".to_string())?
            .pending_downloads
            .insert(source_url, record);
        Ok(private_path)
    }

    fn finish_quarantined_download(&self, source_url: &str, success: bool) {
        if let Ok(mut state) = self.inner.lock() {
            if let Some(mut record) = state.pending_downloads.remove(source_url) {
                record.completed = true;
                record.success = success;
                state.completed_downloads.push(record);
            }
        }
    }

    pub(crate) fn take_completed_downloads(&self) -> Vec<NativeBrowserDownload> {
        self.inner
            .lock()
            .map(|mut state| std::mem::take(&mut state.completed_downloads))
            .unwrap_or_default()
    }
}

fn sanitize_download_name(value: &str) -> String {
    let name = value
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' => character,
            _ => '_',
        })
        .collect::<String>();
    let trimmed = name.trim_matches(['.', '_']);
    if trimmed.is_empty() {
        "download.bin".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

fn handle_native_browser_download(
    manager: &NativeBrowserManager,
    quarantine_root: &Path,
    event: tauri::webview::DownloadEvent<'_>,
) -> bool {
    match event {
        tauri::webview::DownloadEvent::Requested { url, destination } => {
            match manager.begin_quarantined_download(url.to_string(), quarantine_root) {
                Ok(path) => *destination = path,
                Err(_) => return false,
            }
        }
        tauri::webview::DownloadEvent::Finished { url, success, .. } => {
            manager.finish_quarantined_download(url.as_str(), success);
        }
        _ => {}
    }
    true
}

fn bound_navigation_is_allowed(binding: &CanonicalDestination, url: &url::Url) -> bool {
    let decision = validate_browser_navigation_blocking(binding, url.as_str());
    if let Err(error) = decision.as_ref() {
        eprintln!(
            "NATIVE_BROWSER_SECURITY_EVENT operation=navigation destination_binding={} decision=blocked code={}",
            binding.binding_fingerprint(),
            error.code
        );
    }
    decision.is_ok()
}

#[tauri::command(rename_all = "camelCase")]
pub async fn authorize_native_browser_navigation(
    url: String,
    manager: tauri::State<'_, NativeBrowserManager>,
    approvals: tauri::State<'_, ShieldApprovalManager>,
    app: tauri::AppHandle,
) -> Result<BrowserNavigationAuthorization, NativeBrowserBridgeError> {
    let authorization_epoch = manager.epoch().map_err(|error| {
        NativeBrowserBridgeError::new("browser_route_unavailable", "authorization", error)
    })?;
    let destination = resolve_destination(&url, DestinationTransport::NativeBrowser, None)
        .await
        .map_err(|error| {
            NativeBrowserBridgeError::new(
                "browser_navigation_blocked",
                "authorization",
                error.message,
            )
        })?;
    let action = native_browser_shield_action(&destination);
    let approval = shield_gate::build_shield_approval_request(&action).ok_or_else(|| {
        NativeBrowserBridgeError::new(
            "browser_command_unavailable",
            "authorization",
            "Browser permission could not be prepared.",
        )
    })?;
    shield_gate::request_user_approval(&app, approvals.inner(), approval)
        .await
        .map_err(NativeBrowserBridgeError::authorization)?;

    eprintln!(
        "NATIVE_BROWSER_SECURITY_EVENT operation=authorize destination_binding={} decision=approved",
        destination.binding_fingerprint()
    );
    manager
        .insert_authorization(destination, authorization_epoch)
        .map_err(|error| NativeBrowserBridgeError::new("browser_cancelled", "authorization", error))
}

fn native_browser_shield_action(destination: &CanonicalDestination) -> RequestedAction {
    RequestedAction {
        kind: "web_fetch".to_string(),
        principal: Some("model_directed_native_browser".to_string()),
        path: Some(destination.canonical_origin().to_string()),
        content: Some(format!(
            "User confirmation is required before native browser navigation. {}",
            destination.redacted_summary()
        )),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn open_authorized_native_browser(
    approval_token: String,
    bounds: NativeBrowserBounds,
    manager: tauri::State<'_, NativeBrowserManager>,
    app: tauri::AppHandle,
) -> Result<NativeBrowserBridgeReady, NativeBrowserBridgeError> {
    let bounds = validate_bounds(bounds).map_err(|error| {
        NativeBrowserBridgeError::new("browser_native_open_failed", "open", error)
    })?;
    let (approved, authorization_epoch) =
        manager
            .consume_authorization(&approval_token)
            .map_err(|error| {
                NativeBrowserBridgeError::new("browser_authorization_denied", "open", error)
            })?;
    let manager_for_open = manager.inner().clone();
    let app_for_open = app.clone();
    let open = async move {
        let destination = revalidate_destination(&approved).await.map_err(|error| {
            NativeBrowserBridgeError::new("browser_navigation_blocked", "open", error.message)
        })?;

        if let Some(existing) = app_for_open.get_webview(BROWSER_WEBVIEW_LABEL) {
            existing.close().map_err(|error| {
                NativeBrowserBridgeError::new(
                    "browser_native_open_failed",
                    "open",
                    format!("Failed to close the prior native browser view: {error}"),
                )
            })?;
        }
        let window = app_for_open.get_window("main").ok_or_else(|| {
            NativeBrowserBridgeError::new(
                "browser_route_unavailable",
                "open",
                "The main application window is unavailable.",
            )
        })?;
        let proxy = start_browser_connect_proxy(destination.clone())
            .await
            .map_err(|error| {
                NativeBrowserBridgeError::new("browser_native_open_failed", "open", error)
            })?;
        let proxy_url = proxy.proxy_url();
        let navigation_binding = destination.clone();
        let quarantine_root = app_for_open
            .path()
            .app_data_dir()
            .map_err(|error| {
                NativeBrowserBridgeError::new(
                    "browser_native_open_failed",
                    "open",
                    format!("Unable to resolve browser quarantine: {error}"),
                )
            })?
            .join("browser-quarantine");
        let download_manager = manager_for_open.clone();
        let builder = tauri::webview::WebviewBuilder::new(
            BROWSER_WEBVIEW_LABEL,
            tauri::WebviewUrl::External(destination.url().clone()),
        )
        .incognito(true)
        .proxy_url(proxy_url)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .on_download(move |_, event| {
            handle_native_browser_download(&download_manager, &quarantine_root, event)
        })
        .on_navigation(move |url| bound_navigation_is_allowed(&navigation_binding, url));
        window
            .add_child(
                builder,
                tauri::LogicalPosition::new(bounds.x, bounds.y),
                tauri::LogicalSize::new(bounds.width, bounds.height),
            )
            .map_err(|error| {
                NativeBrowserBridgeError::new(
                    "browser_native_open_failed",
                    "open",
                    format!("Failed to create the isolated native browser view: {error}"),
                )
            })?;
        if !manager_for_open
            .set_active_if_epoch(destination.clone(), proxy, authorization_epoch)
            .map_err(|error| {
                NativeBrowserBridgeError::new("browser_route_unavailable", "open", error)
            })?
        {
            if let Some(stale) = app_for_open.get_webview(BROWSER_WEBVIEW_LABEL) {
                let _ = stale.close();
            }
            return Err(NativeBrowserBridgeError::new(
                "browser_cancelled",
                "open",
                "The browser request was replaced before it opened.",
            ));
        }
        eprintln!(
            "NATIVE_BROWSER_SECURITY_EVENT operation=open destination_binding={} decision=allowed",
            destination.binding_fingerprint()
        );
        Ok(NativeBrowserBridgeReady {
            status: "ready",
            canonical_url: destination.canonical_url().to_string(),
        })
    };

    match tokio::time::timeout(BROWSER_BRIDGE_ACK_TIMEOUT, open).await {
        Ok(result) => result,
        Err(_) => {
            manager.clear_active();
            if let Some(stale) = app.get_webview(BROWSER_WEBVIEW_LABEL) {
                let _ = stale.close();
            }
            Err(NativeBrowserBridgeError::new(
                "browser_dispatch_timeout",
                "open",
                "The native browser did not acknowledge the request in time.",
            ))
        }
    }
}

#[tauri::command(rename_all = "camelCase")]
pub fn resize_native_browser(
    bounds: NativeBrowserBounds,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let bounds = validate_bounds(bounds)?;
    let webview = app
        .get_webview(BROWSER_WEBVIEW_LABEL)
        .ok_or_else(|| "The native browser view is not open.".to_string())?;
    webview
        .set_position(tauri::LogicalPosition::new(bounds.x, bounds.y))
        .map_err(|error| format!("Failed to reposition the native browser view: {error}"))?;
    webview
        .set_size(tauri::LogicalSize::new(bounds.width, bounds.height))
        .map_err(|error| format!("Failed to resize the native browser view: {error}"))
}

#[tauri::command]
pub async fn reload_native_browser(
    manager: tauri::State<'_, NativeBrowserManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let active = manager.active()?;
    revalidate_destination(&active)
        .await
        .map_err(|error| format!("Native browser reload blocked: {}", error.message))?;
    app.get_webview(BROWSER_WEBVIEW_LABEL)
        .ok_or_else(|| "The native browser view is not open.".to_string())?
        .reload()
        .map_err(|error| format!("Failed to reload the native browser view: {error}"))
}

#[tauri::command]
pub fn close_native_browser(
    manager: tauri::State<'_, NativeBrowserManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let close_result = if let Some(webview) = app.get_webview(BROWSER_WEBVIEW_LABEL) {
        webview
            .close()
            .map_err(|error| format!("Failed to close the native browser view: {error}"))
    } else {
        Ok(())
    };
    // Revocation is fail-closed even if the platform reports a webview close
    // error: dropping the proxy immediately cuts the native network path.
    manager.clear_active();
    close_result
}

fn validate_bounds(bounds: NativeBrowserBounds) -> Result<NativeBrowserBounds, String> {
    if !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || bounds.x < 0.0
        || bounds.y < 0.0
        || bounds.width < 1.0
        || bounds.height < 1.0
        || bounds.width > MAX_BROWSER_VIEW_DIMENSION
        || bounds.height > MAX_BROWSER_VIEW_DIMENSION
    {
        return Err("Native browser bounds are outside the allowed finite range.".to_string());
    }
    Ok(bounds)
}

fn generate_browser_token() -> String {
    let mut token = [0_u8; BROWSER_TOKEN_BYTES];
    OsRng.fill_bytes(&mut token);
    hex::encode(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_native_browser_bounds() {
        assert!(validate_bounds(NativeBrowserBounds {
            x: 0.0,
            y: 0.0,
            width: f64::NAN,
            height: 100.0,
        })
        .is_err());
        assert!(validate_bounds(NativeBrowserBounds {
            x: 0.0,
            y: 0.0,
            width: 600.0,
            height: 400.0,
        })
        .is_ok());
    }

    #[test]
    fn bridge_contract_uses_stable_typed_codes_and_camel_case_payloads() {
        let error = NativeBrowserBridgeError::new(
            "browser_dispatch_timeout",
            "open",
            "The native browser did not acknowledge the request in time.",
        );
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({
                "code": "browser_dispatch_timeout",
                "stage": "open",
                "message": "The native browser did not acknowledge the request in time."
            })
        );

        let ready = NativeBrowserBridgeReady {
            status: "ready",
            canonical_url: "https://example.com/".to_string(),
        };
        assert_eq!(
            serde_json::to_value(ready).unwrap(),
            serde_json::json!({
                "status": "ready",
                "canonicalUrl": "https://example.com/"
            })
        );
        assert_eq!(BROWSER_BRIDGE_ACK_TIMEOUT, Duration::from_secs(5));
    }

    #[test]
    fn shield_receipt_uses_origin_and_binding_without_browser_path_or_query() {
        let destination = crate::network_policy::resolve_destination_blocking(
            "https://93.184.216.34/private/account?token=browser-query-canary",
            DestinationTransport::NativeBrowser,
            None,
        )
        .unwrap();
        let action = native_browser_shield_action(&destination);
        let serialized = serde_json::to_string(&action).unwrap();
        assert_eq!(action.path.as_deref(), Some("https://93.184.216.34"));
        assert!(serialized.contains(destination.binding_fingerprint()));
        assert!(!serialized.contains("/private/account"));
        assert!(!serialized.contains("browser-query-canary"));
    }
}
