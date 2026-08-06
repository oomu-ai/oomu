use crate::{db::PersistenceEngine, persistence_health::DegradedModeState};
use reqwest::header::CONTENT_TYPE;
use rusqlite::OptionalExtension;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::{Mutex, Notify};
use url::Url;

const POLICY_KEY: &str = "application_updates.policy.v1";
const OFFICIAL_UPDATE_ENDPOINT: &str =
    "https://github.com/oomu-ai/oomu/releases/latest/download/latest.json";
const OFFICIAL_RELEASE_ROOT: &str = "https://github.com/oomu-ai/oomu/releases";
const AUTOMATIC_WINDOW_MS: i64 = 86_400_000;
const UPDATE_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_NOTES_BYTES: usize = 64 * 1024;
const MAX_NOTES_CHARS: usize = 12_000;
const INSTALL_EVENT: &str = "oomu://application-update-install";
const AUTOMATIC_RESULT_EVENT: &str = "oomu://application-update-result";
const UI_READINESS_EVENT: &str = "oomu://application-update-readiness";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUpdatePolicy {
    #[serde(default = "policy_schema_version")]
    pub schema_version: u8,
    #[serde(default)]
    pub last_successful_check_at_ms: i64,
    #[serde(default)]
    pub last_automatic_attempt_at_ms: i64,
    #[serde(default)]
    pub remind_version: Option<String>,
    #[serde(default)]
    pub remind_after_ms: Option<i64>,
    #[serde(default)]
    pub skipped_version: Option<String>,
}

const fn policy_schema_version() -> u8 {
    1
}

impl Default for ApplicationUpdatePolicy {
    fn default() -> Self {
        Self {
            schema_version: policy_schema_version(),
            last_successful_check_at_ms: 0,
            last_automatic_attempt_at_ms: 0,
            remind_version: None,
            remind_after_ms: None,
            skipped_version: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckOrigin {
    Automatic,
    Manual,
}

impl CheckOrigin {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUpdateCheckResult {
    pub status: String,
    pub origin: String,
    pub current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_notes_available: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

impl ApplicationUpdateCheckResult {
    fn up_to_date(current_version: String, origin: CheckOrigin) -> Self {
        Self::simple("up_to_date", current_version, origin)
    }

    fn simple(status: &str, current_version: String, origin: CheckOrigin) -> Self {
        Self {
            status: status.to_string(),
            origin: origin.as_str().to_string(),
            current_version,
            available_version: None,
            notes: None,
            full_notes_available: None,
            public_code: None,
            retryable: None,
        }
    }

    fn available(
        current_version: String,
        available_version: String,
        notes: String,
        origin: CheckOrigin,
    ) -> Self {
        Self {
            status: "update_available".to_string(),
            origin: origin.as_str().to_string(),
            current_version,
            available_version: Some(available_version),
            notes: Some(notes),
            full_notes_available: Some(true),
            public_code: None,
            retryable: None,
        }
    }

    fn failed(
        current_version: String,
        origin: CheckOrigin,
        public_code: &str,
        retryable: bool,
    ) -> Self {
        Self {
            status: "failed".to_string(),
            origin: origin.as_str().to_string(),
            current_version,
            available_version: None,
            notes: None,
            full_notes_available: None,
            public_code: Some(public_code.to_string()),
            retryable: Some(retryable),
        }
    }

    fn with_origin(mut self, origin: CheckOrigin) -> Self {
        self.origin = origin.as_str().to_string();
        self
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationUpdateDecision {
    Remind,
    Skip,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUpdateDecisionResult {
    pub recorded: bool,
    pub version: String,
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationUpdateInstallEvent {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloaded_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

impl ApplicationUpdateInstallEvent {
    fn state(status: &str) -> Self {
        Self {
            status: status.to_string(),
            downloaded_bytes: None,
            total_bytes: None,
            public_code: None,
            retryable: None,
        }
    }

    fn failed(code: &str, retryable: bool) -> Self {
        Self {
            status: "failed".to_string(),
            downloaded_bytes: None,
            total_bytes: None,
            public_code: Some(code.to_string()),
            retryable: Some(retryable),
        }
    }
}

#[derive(Clone)]
struct PendingApplicationUpdate {
    update: Update,
    version: String,
}

#[derive(Clone)]
struct CachedCheck {
    completed_at: Instant,
    result: ApplicationUpdateCheckResult,
}

pub struct ApplicationUpdateService {
    check_gate: Mutex<()>,
    policy_gate: Mutex<()>,
    pending: Mutex<Option<PendingApplicationUpdate>>,
    last_check: Mutex<Option<CachedCheck>>,
    ui_ready: AtomicBool,
    scheduler_started: AtomicBool,
    ready_to_restart: AtomicBool,
    scheduler_wake: Notify,
}

impl Default for ApplicationUpdateService {
    fn default() -> Self {
        Self {
            check_gate: Mutex::new(()),
            policy_gate: Mutex::new(()),
            pending: Mutex::new(None),
            last_check: Mutex::new(None),
            ui_ready: AtomicBool::new(false),
            scheduler_started: AtomicBool::new(false),
            ready_to_restart: AtomicBool::new(false),
            scheduler_wake: Notify::new(),
        }
    }
}

impl ApplicationUpdateService {
    pub(crate) fn ui_ready(&self) -> bool {
        self.ui_ready.load(Ordering::Acquire)
    }

    fn set_ui_ready(&self, ready: bool) {
        self.ui_ready.store(ready, Ordering::Release);
        self.scheduler_wake.notify_one();
    }

    fn start_scheduler_once(&self, app: tauri::AppHandle) {
        if self
            .scheduler_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        tauri::async_runtime::spawn(async move {
            automatic_scheduler(app).await;
        });
    }
}

pub(crate) fn updater_public_key() -> &'static str {
    env!("OOMU_UPDATER_PUBLIC_KEY")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn elapsed_at_least(now: i64, previous: i64, duration: i64) -> bool {
    previous <= 0 || (now >= previous && now.saturating_sub(previous) >= duration)
}

fn automatic_due(policy: &ApplicationUpdatePolicy, now: i64) -> bool {
    elapsed_at_least(now, policy.last_successful_check_at_ms, AUTOMATIC_WINDOW_MS)
        && elapsed_at_least(
            now,
            policy.last_automatic_attempt_at_ms,
            AUTOMATIC_WINDOW_MS,
        )
}

fn next_automatic_delay(policy: &ApplicationUpdatePolicy, now: i64) -> Duration {
    if automatic_due(policy, now) {
        return Duration::ZERO;
    }
    let next_success = next_eligible_at(policy.last_successful_check_at_ms, now);
    let next_attempt = next_eligible_at(policy.last_automatic_attempt_at_ms, now);
    let next = next_success.max(next_attempt);
    Duration::from_millis(next.saturating_sub(now).max(1_000) as u64)
}

fn next_eligible_at(timestamp: i64, now: i64) -> i64 {
    if timestamp <= 0 {
        now
    } else if timestamp > now {
        now.saturating_add(AUTOMATIC_WINDOW_MS)
    } else {
        timestamp.saturating_add(AUTOMATIC_WINDOW_MS)
    }
}

fn load_policy(persistence: &PersistenceEngine) -> Result<ApplicationUpdatePolicy, String> {
    let Some(value) = persistence
        .select_app_preference(POLICY_KEY)
        .map_err(|_| "application_update_policy_unavailable".to_string())?
    else {
        return Ok(ApplicationUpdatePolicy::default());
    };
    match serde_json::from_str::<ApplicationUpdatePolicy>(&value) {
        Ok(policy) if policy.schema_version == policy_schema_version() => Ok(policy),
        _ => {
            let repaired = ApplicationUpdatePolicy::default();
            write_policy(persistence, &repaired)?;
            Ok(repaired)
        }
    }
}

fn write_policy(
    persistence: &PersistenceEngine,
    policy: &ApplicationUpdatePolicy,
) -> Result<(), String> {
    let value = serde_json::to_string(policy)
        .map_err(|_| "application_update_policy_invalid".to_string())?;
    persistence
        .upsert_app_preference(POLICY_KEY, &value)
        .map_err(|_| "application_update_policy_unavailable".to_string())
}

fn current_version(app: &tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

fn update_endpoint() -> Result<Url, String> {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("OOMU_UPDATE_ENDPOINT") {
        let url = Url::parse(value.trim()).map_err(|_| "metadata_invalid".to_string())?;
        if is_loopback_url(&url) {
            return Ok(url);
        }
        return Err("metadata_invalid".to_string());
    }
    Url::parse(OFFICIAL_UPDATE_ENDPOINT).map_err(|_| "metadata_invalid".to_string())
}

fn is_loopback_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
        && url.username().is_empty()
        && url.password().is_none()
}

fn validate_public_version(version: &str) -> Result<Version, String> {
    let parsed = Version::parse(version.trim_start_matches('v'))
        .map_err(|_| "metadata_invalid".to_string())?;
    if !parsed.pre.is_empty() || !parsed.build.is_empty() {
        return Err("metadata_invalid".to_string());
    }
    Ok(parsed)
}

fn validate_update(update: &Update) -> Result<(), String> {
    let version = validate_public_version(&update.version)?;
    if serde_json::to_vec(&update.raw_json)
        .map_err(|_| "metadata_invalid".to_string())?
        .len()
        > MAX_MANIFEST_BYTES
        || update.signature.len() > 4096
        || update.signature.trim().is_empty()
    {
        return Err("metadata_invalid".to_string());
    }
    #[cfg(debug_assertions)]
    if is_loopback_url(&update.download_url) {
        return Ok(());
    }
    let expected_prefix = format!("/oomu-ai/oomu/releases/download/v{version}/");
    #[cfg(target_arch = "aarch64")]
    let expected_suffix = format!("_{version}_darwin-aarch64.app.tar.gz");
    #[cfg(target_arch = "x86_64")]
    let expected_suffix = format!("_{version}_darwin-x86_64.app.tar.gz");
    let path = update.download_url.path();
    if update.download_url.scheme() != "https"
        || update.download_url.host_str() != Some("github.com")
        || !update.download_url.username().is_empty()
        || update.download_url.password().is_some()
        || update.download_url.query().is_some()
        || update.download_url.fragment().is_some()
        || !path.starts_with(&expected_prefix)
        || !path.ends_with(&expected_suffix)
    {
        return Err("metadata_invalid".to_string());
    }
    Ok(())
}

fn sanitize_notes(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(MAX_NOTES_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

fn release_notes_content_type(value: &str) -> bool {
    value.starts_with("application/json") || value.starts_with("application/octet-stream")
}

fn release_notes_response_url_allowed(url: &Url) -> bool {
    #[cfg(debug_assertions)]
    if is_loopback_url(url) {
        return true;
    }
    url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some("github.com" | "release-assets.githubusercontent.com")
        )
        && url.username().is_empty()
        && url.password().is_none()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalizedReleaseNotes {
    schema_version: u8,
    version: String,
    notes: std::collections::BTreeMap<String, String>,
}

async fn localized_release_notes(version: &str, locale: &str, fallback: Option<&str>) -> String {
    let fallback = sanitize_notes(fallback.unwrap_or_default());
    let endpoint = release_notes_endpoint(version);
    let Ok(endpoint) = endpoint else {
        return fallback;
    };
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
    else {
        return fallback;
    };
    let Ok(response) = client.get(endpoint).send().await else {
        return fallback;
    };
    if !response.status().is_success()
        || !release_notes_response_url_allowed(response.url())
        || response
            .content_length()
            .is_some_and(|length| length > MAX_NOTES_BYTES as u64)
        || !response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(release_notes_content_type)
    {
        return fallback;
    }
    let Ok(bytes) = response.bytes().await else {
        return fallback;
    };
    if bytes.len() > MAX_NOTES_BYTES {
        return fallback;
    }
    let Ok(document) = serde_json::from_slice::<LocalizedReleaseNotes>(&bytes) else {
        return fallback;
    };
    if document.schema_version != 1 || document.version != version {
        return fallback;
    }
    document
        .notes
        .get(locale)
        .or_else(|| document.notes.get("en-US"))
        .map(|value| sanitize_notes(value))
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
}

fn release_notes_endpoint(version: &str) -> Result<Url, String> {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("OOMU_UPDATE_NOTES_ENDPOINT") {
        let url = Url::parse(value.trim()).map_err(|_| "metadata_invalid".to_string())?;
        return is_loopback_url(&url)
            .then_some(url)
            .ok_or_else(|| "metadata_invalid".to_string());
    }
    Url::parse(&format!(
        "{OFFICIAL_RELEASE_ROOT}/download/v{version}/release-notes.json"
    ))
    .map_err(|_| "metadata_invalid".to_string())
}

fn active_locale(persistence: &PersistenceEngine) -> String {
    persistence
        .select_app_preference("ui.active_locale")
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "en-US".to_string())
}

fn classify_check_failure(error: &str) -> (&'static str, bool) {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("network")
        || normalized.contains("request")
        || normalized.contains("dns")
        || normalized.contains("timeout")
        || normalized.contains("connect")
        || normalized.contains("http")
    {
        ("network_unavailable", true)
    } else {
        ("metadata_invalid", false)
    }
}

async fn perform_network_check(
    app: &tauri::AppHandle,
    persistence: &PersistenceEngine,
    service: &ApplicationUpdateService,
) -> ApplicationUpdateCheckResult {
    let installed = current_version(app);
    let endpoint = match update_endpoint() {
        Ok(endpoint) => endpoint,
        Err(code) => {
            return ApplicationUpdateCheckResult::failed(
                installed,
                CheckOrigin::Manual,
                &code,
                false,
            )
        }
    };
    let updater_builder = match app.updater_builder().endpoints(vec![endpoint]) {
        Ok(builder) => builder,
        Err(error) => {
            let (code, retryable) = classify_check_failure(&error.to_string());
            return ApplicationUpdateCheckResult::failed(
                installed,
                CheckOrigin::Manual,
                code,
                retryable,
            );
        }
    };
    let updater = match updater_builder.timeout(UPDATE_TIMEOUT).build() {
        Ok(updater) => updater,
        Err(error) => {
            let (code, retryable) = classify_check_failure(&error.to_string());
            return ApplicationUpdateCheckResult::failed(
                installed,
                CheckOrigin::Manual,
                code,
                retryable,
            );
        }
    };
    let checked = match updater.check().await {
        Ok(checked) => checked,
        Err(error) => {
            let (code, retryable) = classify_check_failure(&error.to_string());
            return ApplicationUpdateCheckResult::failed(
                installed,
                CheckOrigin::Manual,
                code,
                retryable,
            );
        }
    };
    let Some(update) = checked else {
        if let Err(code) = record_successful_check(persistence, now_ms()) {
            return ApplicationUpdateCheckResult::failed(
                installed,
                CheckOrigin::Manual,
                &code,
                true,
            );
        }
        *service.pending.lock().await = None;
        return ApplicationUpdateCheckResult::up_to_date(installed, CheckOrigin::Manual);
    };
    if let Err(code) = validate_update(&update) {
        return ApplicationUpdateCheckResult::failed(installed, CheckOrigin::Manual, &code, false);
    }
    if let Err(code) = record_successful_check(persistence, now_ms()) {
        return ApplicationUpdateCheckResult::failed(installed, CheckOrigin::Manual, &code, true);
    }
    let version = update.version.trim_start_matches('v').to_string();
    let notes = localized_release_notes(
        &version,
        &active_locale(persistence),
        update.body.as_deref(),
    )
    .await;
    *service.pending.lock().await = Some(PendingApplicationUpdate {
        update,
        version: version.clone(),
    });
    ApplicationUpdateCheckResult::available(installed, version, notes, CheckOrigin::Manual)
}

fn record_successful_check(persistence: &PersistenceEngine, now: i64) -> Result<(), String> {
    let mut policy = load_policy(persistence)?;
    policy.last_successful_check_at_ms = now;
    write_policy(persistence, &policy)
}

fn automatic_projection(
    result: &ApplicationUpdateCheckResult,
    persistence: &PersistenceEngine,
    now: i64,
) -> Result<Option<ApplicationUpdateCheckResult>, String> {
    if result.status != "update_available" {
        return Ok(None);
    }
    let version = result
        .available_version
        .as_deref()
        .ok_or_else(|| "metadata_invalid".to_string())?;
    let mut policy = load_policy(persistence)?;
    let (show, changed) = project_automatic_offer(&mut policy, version, now);
    if changed {
        write_policy(persistence, &policy)?;
    }
    Ok(show.then(|| result.clone().with_origin(CheckOrigin::Automatic)))
}

fn project_automatic_offer(
    policy: &mut ApplicationUpdatePolicy,
    version: &str,
    now: i64,
) -> (bool, bool) {
    let mut changed = false;
    let remind_active = if policy.remind_version.as_deref() == Some(version) {
        policy.remind_after_ms.is_some_and(|until| now < until)
    } else {
        if policy.remind_version.is_some() || policy.remind_after_ms.is_some() {
            policy.remind_version = None;
            policy.remind_after_ms = None;
            changed = true;
        }
        false
    };
    let skipped = if policy.skipped_version.as_deref() == Some(version) {
        true
    } else {
        if policy.skipped_version.is_some() {
            policy.skipped_version = None;
            changed = true;
        }
        false
    };
    if !remind_active && policy.remind_version.as_deref() == Some(version) {
        policy.remind_version = None;
        policy.remind_after_ms = None;
        changed = true;
    }
    (!remind_active && !skipped, changed)
}

async fn check_shared(app: &tauri::AppHandle, origin: CheckOrigin) -> ApplicationUpdateCheckResult {
    let service = app.state::<ApplicationUpdateService>();
    let persistence = app.state::<PersistenceEngine>();
    let requested_at = Instant::now();
    let _check = service.check_gate.lock().await;
    if let Some(cached) = service.last_check.lock().await.clone() {
        if cached.completed_at >= requested_at {
            return cached.result.with_origin(origin);
        }
    }
    let result = perform_network_check(app, &persistence, &service).await;
    *service.last_check.lock().await = Some(CachedCheck {
        completed_at: Instant::now(),
        result: result.clone(),
    });
    result.with_origin(origin)
}

fn native_startup_ready(
    app: &tauri::AppHandle,
    persistence: &PersistenceEngine,
    degraded: &DegradedModeState,
) -> bool {
    if !crate::settings::license_is_currently_accepted(app) {
        return false;
    }
    let setup_finished = persistence
        .open_connection()
        .ok()
        .and_then(|connection| {
            connection
                .query_row(
                    "SELECT current_step FROM setup_progress WHERE singleton=1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .ok()
                .flatten()
        })
        .is_some_and(|step| step == "finished");
    if !setup_finished {
        return false;
    }
    let feature_local = [
        "artifactPipeline",
        "autoRouteClassifier",
        "autoRouteSessionBaselines",
        "backgroundHooks",
        "gateway",
        "identity",
        "mcpRuntime",
        "workflowScheduler",
    ];
    let status = degraded.snapshot();
    !status.has_volatile_storage
        && (!status.active
            || status
                .subsystems
                .iter()
                .filter(|item| item.active)
                .all(|item| feature_local.contains(&item.subsystem.as_str())))
}

async fn claim_automatic_attempt(
    persistence: &PersistenceEngine,
    service: &ApplicationUpdateService,
    now: i64,
) -> Result<bool, String> {
    let _policy = service.policy_gate.lock().await;
    let mut policy = load_policy(persistence)?;
    if !automatic_due(&policy, now) {
        return Ok(false);
    }
    policy.last_automatic_attempt_at_ms = now;
    write_policy(persistence, &policy)?;
    Ok(true)
}

async fn run_automatic_check(app: &tauri::AppHandle) {
    let service = app.state::<ApplicationUpdateService>();
    let persistence = app.state::<PersistenceEngine>();
    if !service.ui_ready() {
        return;
    }
    match claim_automatic_attempt(&persistence, &service, now_ms()).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(_) => return,
    }
    let result = check_shared(app, CheckOrigin::Automatic).await;
    if let Ok(Some(result)) = automatic_projection(&result, &persistence, now_ms()) {
        let _ = app.emit(AUTOMATIC_RESULT_EVENT, result);
    }
}

async fn automatic_scheduler(app: tauri::AppHandle) {
    loop {
        let service = app.state::<ApplicationUpdateService>();
        if !service.ui_ready() {
            tokio::select! {
                _ = service.scheduler_wake.notified() => {},
                _ = tokio::time::sleep(Duration::from_secs(60)) => {},
            }
            continue;
        }
        let policy = app
            .state::<PersistenceEngine>()
            .select_app_preference(POLICY_KEY)
            .ok()
            .and_then(|value| value)
            .map(|value| serde_json::from_str::<ApplicationUpdatePolicy>(&value))
            .transpose();
        let delay = match policy {
            Ok(Some(policy)) if policy.schema_version == policy_schema_version() => {
                next_automatic_delay(&policy, now_ms())
            }
            Ok(None) => Duration::ZERO,
            _ => Duration::from_secs(86_400),
        };
        if !delay.is_zero() {
            let service = app.state::<ApplicationUpdateService>();
            tokio::select! {
                _ = service.scheduler_wake.notified() => {},
                _ = tokio::time::sleep(delay.min(Duration::from_secs(86_400))) => {},
            }
            continue;
        }
        run_automatic_check(&app).await;
    }
}

#[tauri::command]
pub async fn set_application_update_ui_ready(
    ready: bool,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
    degraded: tauri::State<'_, DegradedModeState>,
    service: tauri::State<'_, ApplicationUpdateService>,
) -> Result<bool, String> {
    let ready = ready && native_startup_ready(&app, &persistence, &degraded);
    service.set_ui_ready(ready);
    let _ = app.emit(UI_READINESS_EVENT, ready);
    let translations = crate::settings::locale_state_for_engine(&persistence, None)
        .ok()
        .map(|state| state.translations);
    crate::refresh_oomu_menu(&app, translations.as_ref())
        .map_err(|_| "application_update_menu_unavailable".to_string())?;
    if ready {
        service.start_scheduler_once(app.clone());
    }
    Ok(ready)
}

#[tauri::command]
pub async fn check_for_application_update(app: tauri::AppHandle) -> ApplicationUpdateCheckResult {
    let installed = current_version(&app);
    if !app.state::<ApplicationUpdateService>().ui_ready() {
        return ApplicationUpdateCheckResult::failed(
            installed,
            CheckOrigin::Manual,
            "update_ui_not_ready",
            true,
        );
    }
    check_shared(&app, CheckOrigin::Manual).await
}

#[tauri::command]
pub async fn record_application_update_decision(
    version: String,
    decision: ApplicationUpdateDecision,
    persistence: tauri::State<'_, PersistenceEngine>,
    service: tauri::State<'_, ApplicationUpdateService>,
) -> Result<ApplicationUpdateDecisionResult, String> {
    let version = validate_public_version(&version)?.to_string();
    let pending_matches = service
        .pending
        .lock()
        .await
        .as_ref()
        .is_some_and(|pending| pending.version == version);
    if !pending_matches {
        return Err("application_update_offer_expired".to_string());
    }
    let _policy = service.policy_gate.lock().await;
    let mut policy = load_policy(&persistence)?;
    let decision_name = match decision {
        ApplicationUpdateDecision::Remind => {
            policy.remind_version = Some(version.clone());
            policy.remind_after_ms = Some(now_ms().saturating_add(AUTOMATIC_WINDOW_MS));
            "remind"
        }
        ApplicationUpdateDecision::Skip => {
            policy.skipped_version = Some(version.clone());
            "skip"
        }
    };
    write_policy(&persistence, &policy)?;
    *service.pending.lock().await = None;
    Ok(ApplicationUpdateDecisionResult {
        recorded: true,
        version,
        decision: decision_name.to_string(),
    })
}

fn emit_install(app: &tauri::AppHandle, event: &ApplicationUpdateInstallEvent) {
    let _ = app.emit(INSTALL_EVENT, event);
}

#[tauri::command]
pub async fn install_pending_application_update(
    version: String,
    app: tauri::AppHandle,
) -> ApplicationUpdateInstallEvent {
    let service = app.state::<ApplicationUpdateService>();
    let Ok(version) = validate_public_version(&version).map(|value| value.to_string()) else {
        return ApplicationUpdateInstallEvent::failed("application_update_offer_expired", true);
    };
    let pending_matches = service
        .pending
        .lock()
        .await
        .as_ref()
        .is_some_and(|pending| pending.version == version);
    if !pending_matches {
        return ApplicationUpdateInstallEvent::failed("application_update_offer_expired", true);
    }
    let refreshed = check_shared(&app, CheckOrigin::Manual).await;
    if refreshed.status != "update_available" {
        let code = if refreshed.status == "failed" {
            "download_failed"
        } else {
            "application_update_offer_expired"
        };
        return ApplicationUpdateInstallEvent::failed(code, true);
    }
    if refreshed.available_version.as_deref() != Some(version.as_str()) {
        return ApplicationUpdateInstallEvent::failed("application_update_offer_expired", true);
    }
    let pending = service.pending.lock().await.clone();
    let Some(pending) = pending else {
        return ApplicationUpdateInstallEvent::failed("application_update_offer_expired", true);
    };
    let mut downloaded = 0_u64;
    let downloading = ApplicationUpdateInstallEvent::state("downloading");
    emit_install(&app, &downloading);
    let progress_app = app.clone();
    let verifying_app = app.clone();
    let bytes = pending
        .update
        .download(
            move |chunk, total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                emit_install(
                    &progress_app,
                    &ApplicationUpdateInstallEvent {
                        status: "downloading".to_string(),
                        downloaded_bytes: Some(downloaded),
                        total_bytes: total,
                        public_code: None,
                        retryable: None,
                    },
                );
            },
            move || {
                emit_install(
                    &verifying_app,
                    &ApplicationUpdateInstallEvent::state("verifying"),
                )
            },
        )
        .await;
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(error) => {
            let normalized = error.to_string().to_ascii_lowercase();
            let event = if normalized.contains("signature") || normalized.contains("minisign") {
                ApplicationUpdateInstallEvent::failed("signature_invalid", false)
            } else {
                ApplicationUpdateInstallEvent::failed("download_failed", true)
            };
            emit_install(&app, &event);
            return event;
        }
    };
    if pending.update.install(bytes).is_err() {
        let event = ApplicationUpdateInstallEvent::failed("install_failed", true);
        emit_install(&app, &event);
        return event;
    }
    service.ready_to_restart.store(true, Ordering::Release);
    *service.pending.lock().await = None;
    let event = ApplicationUpdateInstallEvent::state("ready_to_restart");
    emit_install(&app, &event);
    event
}

#[tauri::command]
pub async fn open_application_update_release_notes(
    version: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let version = validate_public_version(&version)?.to_string();
    let pending_matches = app
        .state::<ApplicationUpdateService>()
        .pending
        .lock()
        .await
        .as_ref()
        .is_some_and(|pending| pending.version == version);
    if !pending_matches {
        return Err("application_update_offer_expired".to_string());
    }
    app.opener()
        .open_url(
            format!("{OFFICIAL_RELEASE_ROOT}/tag/v{version}"),
            None::<&str>,
        )
        .map_err(|_| "application_update_release_notes_unavailable".to_string())
}

#[tauri::command]
pub fn restart_after_application_update(app: tauri::AppHandle) -> Result<(), String> {
    if !app
        .state::<ApplicationUpdateService>()
        .ready_to_restart
        .load(Ordering::Acquire)
    {
        return Err("application_update_restart_not_ready".to_string());
    }
    app.request_restart();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> ApplicationUpdatePolicy {
        ApplicationUpdatePolicy::default()
    }

    #[test]
    fn daily_window_is_due_at_exactly_twenty_four_hours() {
        let mut value = policy();
        value.last_successful_check_at_ms = 1_000;
        value.last_automatic_attempt_at_ms = 1_000;
        assert!(!automatic_due(&value, 1_000 + AUTOMATIC_WINDOW_MS - 1));
        assert!(automatic_due(&value, 1_000 + AUTOMATIC_WINDOW_MS));
    }

    #[test]
    fn clock_rollback_fails_safe_without_a_rapid_retry() {
        let mut value = policy();
        value.last_automatic_attempt_at_ms = 2_000;
        assert!(!automatic_due(&value, 1_000));
        assert_eq!(
            next_automatic_delay(&value, 1_000),
            Duration::from_millis(AUTOMATIC_WINDOW_MS as u64)
        );
    }

    #[test]
    fn public_versions_follow_complete_semver_and_reject_prereleases() {
        assert!(
            validate_public_version("0.9.9").unwrap() < validate_public_version("1.0.0").unwrap()
        );
        assert!(
            validate_public_version("1.9.0").unwrap() < validate_public_version("2.0.0").unwrap()
        );
        assert!(validate_public_version("1.0.0-rc.1").is_err());
        assert!(validate_public_version("1.0.0+candidate").is_err());
    }

    #[test]
    fn reminder_and_skip_apply_only_to_the_exact_automatic_version() {
        let mut value = policy();
        value.remind_version = Some("0.1.3".to_string());
        value.remind_after_ms = Some(5_000);
        assert_eq!(
            project_automatic_offer(&mut value, "0.1.3", 4_999),
            (false, false)
        );
        assert_eq!(
            project_automatic_offer(&mut value, "0.1.4", 4_999),
            (true, true)
        );
        assert!(value.remind_version.is_none());

        value.skipped_version = Some("0.1.4".to_string());
        assert_eq!(
            project_automatic_offer(&mut value, "0.1.4", 5_000),
            (false, false)
        );
        assert_eq!(
            project_automatic_offer(&mut value, "1.0.0", 5_000),
            (true, true)
        );
        assert!(value.skipped_version.is_none());
    }

    #[test]
    fn expired_reminder_becomes_eligible_once() {
        let mut value = policy();
        value.remind_version = Some("0.1.3".to_string());
        value.remind_after_ms = Some(5_000);
        assert_eq!(
            project_automatic_offer(&mut value, "0.1.3", 5_000),
            (true, true)
        );
        assert!(value.remind_version.is_none());
        assert!(value.remind_after_ms.is_none());
    }

    #[test]
    fn only_loopback_hosts_can_override_the_development_feed() {
        assert!(is_loopback_url(
            &Url::parse("http://127.0.0.1:4312/latest.json").unwrap()
        ));
        assert!(is_loopback_url(
            &Url::parse("https://localhost/latest.json").unwrap()
        ));
        assert!(!is_loopback_url(
            &Url::parse("https://example.com/latest.json").unwrap()
        ));
        assert!(!is_loopback_url(
            &Url::parse("http://user@localhost/latest.json").unwrap()
        ));
    }

    #[test]
    fn remote_notes_are_plain_bounded_text() {
        assert_eq!(
            sanitize_notes("Hello\u{0} world\nNext"),
            "Hello world\nNext"
        );
        assert_eq!(
            sanitize_notes("<script>alert(1)</script>"),
            "<script>alert(1)</script>"
        );
        assert!(sanitize_notes(&"x".repeat(MAX_NOTES_CHARS + 50)).len() <= MAX_NOTES_CHARS);
        assert!(release_notes_content_type(
            "application/json; charset=utf-8"
        ));
        assert!(release_notes_content_type("application/octet-stream"));
        assert!(!release_notes_content_type("text/html"));
        assert!(release_notes_response_url_allowed(
            &Url::parse("https://release-assets.githubusercontent.com/object").unwrap()
        ));
    }
}
