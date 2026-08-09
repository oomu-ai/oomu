use crate::{db::PersistenceEngine, foundation::clock::unix_time_ms_i64 as unix_time_ms, gemma};
use rfd::AsyncFileDialog;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};
use tauri::Manager;
const SETTINGS_FILE: &str = "oomu_settings.json";
#[cfg(test)]
const TEST_APP_IDENTIFIER: &str = "ai.eldris.oomu.gpd.test";
pub const APP_DATA_ROOT_ENV: &str = "OOMU_APP_DATA_DIR";
pub const LOCALES_DIR_ENV: &str = "OOMU_LOCALES_DIR";
const DEFAULT_LOCALE_ID: &str = "en-US";
const ACTIVE_LOCALE_SETTING_KEY: &str = "ui.active_locale";
pub const CURRENT_LICENSE_VERSION: &str = "1.2";
pub const LICENSE_EFFECTIVE_DATE: &str = "July 10, 2026";
pub const DEFAULT_CONTEXT_BUDGET: usize = 12_288;
pub const DEFAULT_CLOUD_CONTEXT_BUDGET: usize = 12_288;
pub const DEFAULT_AUTO_COMPACTION_THRESHOLD_PERCENT: u8 = 70;
pub const DYNAMIC_CLOUD_FALLBACK_MODEL_ID: &str = "gemini-3.5-flash";
const LICENSE_TEXT: &str = include_str!("../../LICENSE.md");
static SETTINGS_WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub context_budget: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            context_budget: DEFAULT_CONTEXT_BUDGET,
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct AppSettings {
    local_model_directory: Option<String>,
    #[serde(default)]
    default_prewarmed_model_id: Option<String>,
    #[serde(default)]
    privacy: PrivacySettings,
    #[serde(default)]
    license: LicenseSettings,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrivacySettings {
    #[serde(default)]
    automated_web_grounding_enabled: bool,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LicenseAcceptanceState {
    #[default]
    NotPresented,
    #[serde(alias = "declined")]
    Presented,
    Accepted,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LicenseSettings {
    updated_at_ms: Option<i64>,
    #[serde(default)]
    state: LicenseAcceptanceState,
    #[serde(default)]
    accepted_version: Option<String>,
    #[serde(default)]
    accepted_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelDirectorySetting {
    pub path: String,
    pub is_default: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultPrewarmedModelSetting {
    pub model_id: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocaleAsset {
    pub id: String,
    pub label: String,
    pub file_name: String,
    pub is_default: bool,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocaleState {
    pub active_locale: String,
    pub available_locales: Vec<LocaleAsset>,
    pub translations: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacySettingsState {
    pub automated_web_grounding_enabled: bool,
    pub license_accepted: bool,
    pub license_state: LicenseAcceptanceState,
    pub accepted_license_version: Option<String>,
    pub acceptance_timestamp_ms: Option<i64>,
    pub license_version: String,
    pub license_effective_date: String,
    pub license_text: String,
}

#[tauri::command]
pub fn get_privacy_settings(app: tauri::AppHandle) -> Result<PrivacySettingsState, String> {
    update_settings(&app, |settings| {
        reconcile_license_version(settings);
        if settings.license.state == LicenseAcceptanceState::NotPresented {
            settings.license.state = LicenseAcceptanceState::Presented;
            settings.license.updated_at_ms = Some(unix_time_ms());
        }
        Ok(privacy_settings_from(settings))
    })
}

#[tauri::command]
pub fn set_automated_web_grounding_enabled(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<PrivacySettingsState, String> {
    update_settings(&app, |settings| {
        settings.privacy.automated_web_grounding_enabled = enabled;
        Ok(privacy_settings_from(settings))
    })
}

#[tauri::command]
pub fn accept_license(app: tauri::AppHandle) -> Result<PrivacySettingsState, String> {
    let _guard = SETTINGS_WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut settings = read_settings(&app)?;
    reconcile_license_version(&mut settings);
    accept_current_license(&mut settings, unix_time_ms());
    write_settings_to_path(&settings_path(&app)?, &settings)?;

    let verified = read_settings(&app)?;
    if !license_is_accepted(&verified) {
        return Err("The license acceptance could not be verified. Nothing changed.".to_string());
    }
    Ok(privacy_settings_from(&verified))
}

#[tauri::command]
pub fn decline_license(app: tauri::AppHandle) -> Result<(), String> {
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub fn get_local_model_directory(
    app: tauri::AppHandle,
) -> Result<LocalModelDirectorySetting, String> {
    let default_path = default_local_model_directory();
    let selected_path = configured_local_model_directory(&app)?;
    let path = selected_path.as_ref().unwrap_or(&default_path);

    Ok(LocalModelDirectorySetting {
        path: path.display().to_string(),
        is_default: selected_path.is_none(),
    })
}

#[tauri::command]
pub async fn choose_local_model_directory(
    app: tauri::AppHandle,
) -> Result<Option<LocalModelDirectorySetting>, String> {
    let current = get_local_model_directory(app.clone())?;
    let initial_directory = existing_directory_or_parent(Path::new(&current.path));
    let mut dialog = AsyncFileDialog::new().set_title("Choose Local Models Directory");

    if let Some(initial_directory) = initial_directory {
        dialog = dialog.set_directory(initial_directory);
    }

    let Some(selected_directory) = dialog.pick_folder().await else {
        return Ok(None);
    };

    let selected_path = selected_directory.path().to_path_buf();
    let mut settings = read_settings(&app)?;
    settings.local_model_directory = Some(selected_path.display().to_string());
    write_settings(&app, &settings)?;

    Ok(Some(LocalModelDirectorySetting {
        path: selected_path.display().to_string(),
        is_default: false,
    }))
}

/// Opens the operating system's folder picker without importing, copying, or
/// otherwise mutating the selected directory. The returned path is exactly the
/// path the user approved in the native dialog.
#[tauri::command(rename_all = "camelCase")]
pub async fn choose_directory_path(
    title: String,
    initial_path: Option<String>,
) -> Result<Option<String>, String> {
    let title = sanitize_folder_picker_title(&title)?;
    let mut dialog = AsyncFileDialog::new().set_title(title);

    if let Some(initial_directory) = initial_path
        .as_deref()
        .and_then(initial_folder_picker_directory)
    {
        dialog = dialog.set_directory(initial_directory);
    }

    Ok(dialog
        .pick_folder()
        .await
        .map(|folder| folder.path().display().to_string()))
}

#[tauri::command]
pub fn get_default_prewarmed_model(
    app: tauri::AppHandle,
) -> Result<DefaultPrewarmedModelSetting, String> {
    let selected_model_id = configured_default_prewarmed_model_id(&app)?;
    Ok(DefaultPrewarmedModelSetting {
        model_id: selected_model_id
            .clone()
            .unwrap_or_else(default_prewarmed_model_id),
        is_default: selected_model_id.is_none(),
    })
}

#[tauri::command]
pub async fn set_default_prewarmed_model(
    app: tauri::AppHandle,
    model_id: String,
) -> Result<DefaultPrewarmedModelSetting, String> {
    let model_id = normalize_local_model_id(&model_id)?;
    let model_root = resolved_local_model_directory(&app)?;
    let requested_model_id = model_id.clone();
    let model_root_for_resolution = model_root.clone();
    let assignment = tauri::async_runtime::spawn_blocking(move || {
        gemma::resolve_verified_startup_model_assignment(
            &model_root_for_resolution,
            &gemma::StartupModelPreference {
                requested_model_id,
                selection_source: gemma::StartupModelSelectionSource::ExplicitUserSelection,
            },
        )
    })
    .await
    .map_err(|_| "OOMU couldn't check this on-device model. Try again.".to_string())?
    .map_err(|error| {
        eprintln!(
            "DEFAULT_PREWARMED_MODEL_RESOLUTION_FAILED code={} message={}",
            crate::redaction::redacted_log_text(error.code),
            crate::redaction::redacted_log_text(&error.message),
        );
        "OOMU couldn't prepare this on-device model. Make sure it is fully installed, then try again."
            .to_string()
    })?;
    let service = app
        .try_state::<gemma::GemmaService>()
        .ok_or_else(|| {
            "OOMU is still preparing on-device models. Try again in a moment.".to_string()
        })?
        .inner()
        .clone();
    let runtime_matches = service
        .startup_model_assignment()
        .as_ref()
        .is_some_and(|current| current == &assignment);
    let previous_assignment = service.startup_model_assignment();
    let mut settings = read_settings(&app)?;
    let previous_model_id = settings.default_prewarmed_model_id.clone();
    settings.default_prewarmed_model_id = Some(model_id.clone());
    if let Err(error) = write_settings(&app, &settings) {
        eprintln!(
            "DEFAULT_PREWARMED_MODEL_SAVE_FAILED message={}",
            crate::redaction::redacted_log_text(&error),
        );
        return Err("OOMU couldn't save this model choice. Nothing changed.".to_string());
    }
    if !runtime_matches {
        if let Err(failure) = reconfigure_prewarmed_model(&service, assignment).await {
            let rollback = rollback_default_prewarmed_model(&app, previous_model_id);
            let restoration = match (rollback.as_ref(), previous_assignment) {
                (Ok(()), Some(previous)) => reconfigure_prewarmed_model(&service, previous).await,
                _ => Err(failure.clone()),
            };
            let restoration_succeeded = restoration.is_ok();
            if !restoration_succeeded {
                let final_failure = restoration.err().unwrap_or_else(|| failure.clone());
                service.mark_classifier_failure(
                    final_failure.0,
                    "default_prewarmed_model_change",
                    &final_failure.1,
                );
            }
            eprintln!(
                "DEFAULT_PREWARMED_MODEL_RECONFIGURATION_FAILED code={} message={} rollback={}",
                crate::redaction::redacted_log_text(failure.0),
                crate::redaction::redacted_log_text(&failure.1),
                rollback.is_ok(),
            );
            return Err(if rollback.is_ok() && restoration_succeeded {
                "OOMU couldn't prepare this on-device model. Your previous model is still ready."
                    .to_string()
            } else {
                "OOMU couldn't prepare this on-device model. Auto-route is paused until you try again."
                    .to_string()
            });
        }
    }

    Ok(DefaultPrewarmedModelSetting {
        model_id,
        is_default: false,
    })
}

type PrewarmedModelFailure = (&'static str, String);

async fn reconfigure_prewarmed_model(
    service: &gemma::GemmaService,
    assignment: gemma::StartupModelAssignment,
) -> Result<(), PrewarmedModelFailure> {
    let service_for_worker = service.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        service_for_worker.reconfigure_startup_model_assignment(assignment)
    })
    .await;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err((error.code, error.message)),
        Err(_) => Err((
            "classifier_reconfiguration_worker_failed",
            "The on-device model worker ended before preparation finished.".to_string(),
        )),
    }
}

fn rollback_default_prewarmed_model(
    app: &tauri::AppHandle,
    previous_model_id: Option<String>,
) -> Result<(), String> {
    let mut settings = read_settings(app)?;
    settings.default_prewarmed_model_id = previous_model_id;
    write_settings(app, &settings)
}

#[tauri::command]
pub async fn get_locale_state(
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<LocaleState, String> {
    let persistence = persistence.inner().clone();
    let state =
        tauri::async_runtime::spawn_blocking(move || locale_state_for_engine(&persistence, None))
            .await
            .map_err(|error| error.to_string())??;
    crate::refresh_oomu_menu(&app, Some(&state.translations)).map_err(|error| error.to_string())?;
    Ok(state)
}

#[tauri::command]
pub async fn set_active_locale(
    locale_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
    app: tauri::AppHandle,
) -> Result<LocaleState, String> {
    let persistence = persistence.inner().clone();
    let state = tauri::async_runtime::spawn_blocking(move || {
        let requested = normalize_locale_id(&locale_id)?;
        let (directory, assets) = scan_locale_assets()?;
        if !assets.iter().any(|asset| asset.id == requested) {
            return Err(format!(
                "Locale '{requested}' is not available in the launch-ready locale set."
            ));
        }
        persistence
            .upsert_app_preference(ACTIVE_LOCALE_SETTING_KEY, &requested)
            .map_err(|error| error.to_string())?;
        locale_state_from_assets(&persistence, &directory, assets, Some(requested))
    })
    .await
    .map_err(|error| error.to_string())??;
    crate::refresh_background_tray_menu(&app, &state.translations)?;
    crate::refresh_oomu_menu(&app, Some(&state.translations)).map_err(|error| error.to_string())?;
    Ok(state)
}

pub(crate) fn resolved_local_model_directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(configured_local_model_directory(app)?.unwrap_or_else(default_local_model_directory))
}

pub(crate) fn snapshot_local_model_configuration(
    app: &tauri::AppHandle,
) -> Result<(Option<PathBuf>, Option<String>), String> {
    let settings = read_settings(app)?;
    Ok((
        settings.local_model_directory.map(PathBuf::from),
        settings.default_prewarmed_model_id,
    ))
}

/// Commits a model root only after the recommended installer has promoted and
/// verified its package. `None` restores OOMU's managed models directory.
pub(crate) fn commit_verified_local_model_directory(
    app: &tauri::AppHandle,
    selected_root: Option<&Path>,
) -> Result<(), String> {
    let selected_root = selected_root
        .map(|root| {
            if !root.is_absolute() {
                return Err("The verified model location must be an absolute path.".to_string());
            }
            Ok(root.display().to_string())
        })
        .transpose()?;
    update_settings(app, |settings| {
        settings.local_model_directory = selected_root;
        Ok(())
    })
}

pub(crate) fn restore_local_model_configuration(
    app: &tauri::AppHandle,
    active_models_root: Option<&Path>,
    prewarmed_model_id: Option<&str>,
) -> Result<(), String> {
    update_settings(app, |settings| {
        settings.local_model_directory = active_models_root.map(|path| path.display().to_string());
        settings.default_prewarmed_model_id = prewarmed_model_id.map(str::to_string);
        Ok(())
    })
}

pub(crate) fn resolved_default_prewarmed_model_id(
    app: &tauri::AppHandle,
) -> Result<String, String> {
    Ok(resolved_startup_model_preference(app)?.requested_model_id)
}

pub(crate) fn resolved_startup_model_preference(
    app: &tauri::AppHandle,
) -> Result<gemma::StartupModelPreference, String> {
    resolve_startup_model_preference(read_settings(app)?)
}

fn configured_local_model_directory(app: &tauri::AppHandle) -> Result<Option<PathBuf>, String> {
    Ok(read_settings(app)?
        .local_model_directory
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from))
}

fn configured_default_prewarmed_model_id(app: &tauri::AppHandle) -> Result<Option<String>, String> {
    Ok(read_settings(app)?
        .default_prewarmed_model_id
        .and_then(|model_id| normalize_local_model_id(&model_id).ok()))
}

fn default_local_model_directory() -> PathBuf {
    models_root()
}

fn default_prewarmed_model_id() -> String {
    gemma::CLEAN_INSTALL_STARTUP_MODEL_ID.to_string()
}

#[cfg(test)]
fn resolve_default_prewarmed_model_id_strict(settings: AppSettings) -> Result<String, String> {
    Ok(resolve_startup_model_preference(settings)?.requested_model_id)
}

fn resolve_startup_model_preference(
    settings: AppSettings,
) -> Result<gemma::StartupModelPreference, String> {
    match settings.default_prewarmed_model_id {
        Some(model_id) => Ok(gemma::StartupModelPreference {
            requested_model_id: normalize_local_model_id(&model_id)?,
            selection_source: gemma::StartupModelSelectionSource::ExplicitUserSelection,
        }),
        None => Ok(gemma::StartupModelPreference {
            requested_model_id: default_prewarmed_model_id(),
            selection_source: gemma::StartupModelSelectionSource::CleanDefault,
        }),
    }
}

fn normalize_local_model_id(model_id: &str) -> Result<String, String> {
    let model_id = model_id.trim();
    if model_id.is_empty()
        || model_id == "."
        || model_id == ".."
        || model_id.contains('/')
        || model_id.contains('\\')
    {
        return Err("Default prewarmed model must be a local model id.".to_string());
    }

    Ok(model_id.to_string())
}

/// Resolve the local model directory without a Tauri `AppHandle`, applying the
/// same precedence as [`resolved_local_model_directory`]: the user-configured
/// directory, else the default [`models_root`].
///
/// `GemmaService` is constructed at startup without an app handle, so its
/// workflow compiler could not previously read the configured directory and
/// fell back to the (often empty) default `app_data/models`. Settings live at a
/// fixed path under `app_data_root()`, so they can be read directly here.
pub fn resolved_local_model_directory_headless() -> PathBuf {
    resolve_model_root_from_settings(read_settings_from_disk().ok())
}

/// Apply the configured-directory-else-default precedence to an already-read
/// settings value. Split out from disk IO so the precedence is unit-testable.
fn resolve_model_root_from_settings(settings: Option<AppSettings>) -> PathBuf {
    settings
        .and_then(|settings| settings.local_model_directory)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(models_root)
}

#[cfg(test)]
fn resolve_default_prewarmed_model_id_from_settings(settings: Option<AppSettings>) -> String {
    settings
        .and_then(|settings| settings.default_prewarmed_model_id)
        .and_then(|model_id| normalize_local_model_id(&model_id).ok())
        .unwrap_or_else(default_prewarmed_model_id)
}

#[cfg(test)]
fn resolve_auto_route_classifier_model_id_from_settings(settings: Option<AppSettings>) -> String {
    settings
        .and_then(|settings| resolve_startup_model_preference(settings).ok())
        .map(|preference| preference.requested_model_id)
        .unwrap_or_else(default_prewarmed_model_id)
}

pub fn app_data_root() -> PathBuf {
    if let Some(root) = crate::launch_startup::sprint_294_isolated_profile::app_data_root() {
        return root;
    }
    #[cfg(test)]
    if let Some(root) = std::env::var_os(APP_DATA_ROOT_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return root;
    }
    default_app_data_root().unwrap_or_else(|| install_root().join("app_data"))
}

#[cfg(not(test))]
fn default_app_data_root() -> Option<PathBuf> {
    dirs::data_dir()
        .map(|directory| directory.join(crate::keychain_namespace::application_data_identifier()))
}

#[cfg(test)]
fn default_app_data_root() -> Option<PathBuf> {
    Some(
        std::env::temp_dir()
            .join(TEST_APP_IDENTIFIER)
            .join(std::process::id().to_string()),
    )
}

pub fn models_root() -> PathBuf {
    app_data_root().join("models")
}

pub fn install_root() -> PathBuf {
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => return PathBuf::from("."),
    };

    #[cfg(target_os = "macos")]
    if let Some(app_bundle) = executable.ancestors().find(|path| {
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    }) {
        return app_bundle.to_path_buf();
    }

    executable
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn existing_directory_or_parent(path: &Path) -> Option<&Path> {
    if path.is_dir() {
        return Some(path);
    }

    path.ancestors().find(|ancestor| ancestor.is_dir())
}

fn sanitize_folder_picker_title(title: &str) -> Result<&str, String> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 160 || title.chars().any(char::is_control) {
        return Err("Folder picker title is invalid.".to_string());
    }
    Ok(title)
}

fn initial_folder_picker_directory(path: &str) -> Option<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let expanded = if trimmed == "~" {
        dirs::home_dir()?
    } else if let Some(relative) = trimmed.strip_prefix("~/") {
        dirs::home_dir()?.join(relative)
    } else {
        PathBuf::from(trimmed)
    };
    existing_directory_or_parent(&expanded).map(Path::to_path_buf)
}

fn settings_path(_app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_root().join(SETTINGS_FILE))
}

fn read_settings(_app: &tauri::AppHandle) -> Result<AppSettings, String> {
    read_settings_from_disk()
}

/// Read settings straight from disk without a Tauri `AppHandle`. The settings
/// file lives at a fixed path under `app_data_root()`, so callers that lack an
/// app handle (e.g. the headless `GemmaService` workflow compiler) can still
/// read user configuration.
fn read_settings_from_disk() -> Result<AppSettings, String> {
    let isolated_path = app_data_root().join(SETTINGS_FILE);
    let path = if !isolated_path.exists() && crate::scenario_one_e2e_profile::enabled() {
        default_app_data_root()
            .map(|root| root.join(SETTINGS_FILE))
            .filter(|path| path.exists())
            .unwrap_or(isolated_path)
    } else {
        isolated_path
    };
    if !path.exists() {
        return Ok(AppSettings::default());
    }

    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("Unable to read OOMU settings: {error}"))?;
    serde_json::from_str(&contents)
        .map_err(|error| format!("Unable to parse OOMU settings: {error}"))
}

pub fn automated_web_grounding_enabled(app: &tauri::AppHandle) -> Result<bool, String> {
    Ok(read_settings(app)?.privacy.automated_web_grounding_enabled)
}

pub(crate) fn license_is_currently_accepted(app: &tauri::AppHandle) -> bool {
    read_settings(app)
        .map(|settings| license_is_accepted(&settings))
        .unwrap_or(false)
}

pub fn automated_web_grounding_enabled_from_disk() -> bool {
    read_settings_from_disk()
        .map(|settings| settings.privacy.automated_web_grounding_enabled)
        .unwrap_or(false)
}

pub(crate) fn locale_state_for_engine(
    persistence: &PersistenceEngine,
    requested_locale: Option<String>,
) -> Result<LocaleState, String> {
    let (directory, assets) = scan_locale_assets()?;
    locale_state_from_assets(persistence, &directory, assets, requested_locale)
}

fn locale_state_from_assets(
    persistence: &PersistenceEngine,
    directory: &Path,
    assets: Vec<LocaleAsset>,
    requested_locale: Option<String>,
) -> Result<LocaleState, String> {
    if assets.is_empty() {
        return Err("No verified locale files were found under src/locales.".to_string());
    }

    let persisted_locale = persistence
        .select_app_preference(ACTIVE_LOCALE_SETTING_KEY)
        .map_err(|error| error.to_string())?;
    let active_locale = requested_locale
        .or(persisted_locale)
        .filter(|locale| assets.iter().any(|asset| asset.id == *locale))
        .unwrap_or_else(|| DEFAULT_LOCALE_ID.to_string());
    let active_locale = if assets.iter().any(|asset| asset.id == active_locale) {
        active_locale
    } else {
        assets
            .first()
            .map(|asset| asset.id.clone())
            .unwrap_or_else(|| DEFAULT_LOCALE_ID.to_string())
    };
    let active_file = assets
        .iter()
        .find(|asset| asset.id == active_locale)
        .map(|asset| asset.file_name.clone())
        .ok_or_else(|| format!("Locale '{active_locale}' is not available."))?;
    let default_file = assets
        .iter()
        .find(|asset| asset.id == DEFAULT_LOCALE_ID)
        .map(|asset| asset.file_name.clone())
        .ok_or_else(|| {
            "The US English master locale en-US.json is missing or invalid.".to_string()
        })?;
    let default_translations = read_locale_dictionary_from_path(&directory.join(default_file))?;
    let active_translations = if active_locale == DEFAULT_LOCALE_ID {
        default_translations.clone()
    } else {
        read_locale_dictionary_from_path(&directory.join(active_file))?
    };
    let translations = merge_locale_fallbacks(&default_translations, active_translations);

    Ok(LocaleState {
        active_locale,
        available_locales: assets,
        translations,
    })
}

fn scan_locale_assets() -> Result<(PathBuf, Vec<LocaleAsset>), String> {
    let Some(directory) = locale_assets_directory() else {
        return Err("Unable to locate src/locales for static locale discovery.".to_string());
    };
    let assets = scan_locale_assets_from_dir(&directory)?;
    Ok((directory, assets))
}

fn locale_assets_directory() -> Option<PathBuf> {
    locale_directory_candidates()
        .into_iter()
        .find(|candidate| candidate.is_dir())
}

fn locale_directory_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os(LOCALES_DIR_ENV).filter(|value| !value.is_empty()) {
        candidates.push(PathBuf::from(path));
    }

    let manifest_locales =
        PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join("../src/locales");
    candidates.push(manifest_locales);

    if let Ok(current_directory) = std::env::current_dir() {
        candidates.push(current_directory.join("src/locales"));
    }

    let install_root = install_root();
    candidates.push(install_root.join("src/locales"));
    candidates.push(install_root.join("../src/locales"));
    candidates.push(install_root.join("Contents/Resources/src/locales"));
    candidates.push(install_root.join("Contents/Resources/_up_/src/locales"));
    candidates.push(install_root.join("Contents/Resources/locales"));
    candidates.push(install_root.join("Resources/src/locales"));
    candidates.push(install_root.join("locales"));
    candidates
}

fn scan_locale_assets_from_dir(directory: &Path) -> Result<Vec<LocaleAsset>, String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("Unable to read locale assets directory: {error}"))?;
    let mut assets = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|error| format!("Unable to inspect locale asset: {error}"))?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }

        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(locale_id) = file_name.strip_suffix(".json") else {
            continue;
        };
        let Ok(locale_id) = normalize_locale_id(locale_id) else {
            continue;
        };
        let Ok(dictionary) = read_locale_dictionary_from_path(&path) else {
            continue;
        };
        if !dictionary.is_object() {
            continue;
        }

        assets.push(LocaleAsset {
            id: locale_id.clone(),
            label: locale_display_label(&locale_id),
            file_name: file_name.to_string(),
            is_default: locale_id == DEFAULT_LOCALE_ID,
            verified: true,
        });
    }

    assets.sort_by(|left, right| {
        if left.id == DEFAULT_LOCALE_ID {
            std::cmp::Ordering::Less
        } else if right.id == DEFAULT_LOCALE_ID {
            std::cmp::Ordering::Greater
        } else {
            left.label
                .to_ascii_lowercase()
                .cmp(&right.label.to_ascii_lowercase())
        }
    });

    if !assets.iter().any(|asset| asset.id == DEFAULT_LOCALE_ID) {
        return Err("The US English master locale en-US.json is missing or invalid.".to_string());
    }

    Ok(assets)
}

fn merge_locale_fallbacks(defaults: &Value, active: Value) -> Value {
    match (defaults, active) {
        (Value::Object(defaults), Value::Object(mut active)) => {
            let mut merged = serde_json::Map::new();
            for (key, default_value) in defaults {
                let value = match active.remove(key) {
                    Some(active_value) => merge_locale_fallbacks(default_value, active_value),
                    None => default_value.clone(),
                };
                merged.insert(key.clone(), value);
            }
            for (key, active_value) in active {
                merged.insert(key, active_value);
            }
            Value::Object(merged)
        }
        (_, active) => active,
    }
}

fn read_locale_dictionary_from_path(path: &Path) -> Result<Value, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("Unable to read locale asset {}: {error}", path.display()))?;
    let value = serde_json::from_str::<Value>(&contents)
        .map_err(|error| format!("Unable to parse locale asset {}: {error}", path.display()))?;
    if !value.is_object() {
        return Err(format!(
            "Locale asset {} must contain a JSON object.",
            path.display()
        ));
    }
    Ok(value)
}

fn normalize_locale_id(locale_id: &str) -> Result<String, String> {
    let trimmed = locale_id.trim();
    if trimmed.is_empty()
        || trimmed.len() > 40
        || trimmed.starts_with('.')
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("Locale id must be a safe static locale identifier.".to_string());
    }

    let parts: Vec<&str> = trimmed.split('-').collect();
    if parts.len() < 2
        || parts.iter().any(|part| {
            part.is_empty() || part.len() > 8 || !part.chars().all(|c| c.is_ascii_alphanumeric())
        })
    {
        return Err("Locale id must use a language-region format like en-US.".to_string());
    }

    let language = parts[0].to_ascii_lowercase();
    let mut normalized_parts = Vec::with_capacity(parts.len());
    normalized_parts.push(language);
    for part in parts.iter().skip(1) {
        normalized_parts.push(part.to_ascii_uppercase());
    }
    Ok(normalized_parts.join("-"))
}

fn locale_display_label(locale_id: &str) -> String {
    match locale_id {
        "en-US" => "English (US)".to_string(),
        "es-ES" => "Español (España)".to_string(),
        "de-DE" => "Deutsch".to_string(),
        "fr-FR" => "Français".to_string(),
        "pt-BR" => "Português (Brasil)".to_string(),
        "ru-RU" => "Русский".to_string(),
        "uk-UA" => "Українська".to_string(),
        "id-ID" => "Bahasa Indonesia".to_string(),
        "vi-VN" => "Tiếng Việt".to_string(),
        "ja-JP" => "日本語".to_string(),
        "zh-CN" => "简体中文".to_string(),
        "zh-TW" => "繁體中文".to_string(),
        _ => {
            let language = locale_id.split('-').next().unwrap_or(locale_id);
            format!("{} Language", title_case_ascii(language))
        }
    }
}

fn title_case_ascii(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    format!(
        "{}{}",
        first.to_ascii_uppercase(),
        chars.as_str().to_ascii_lowercase()
    )
}

fn privacy_settings_from(settings: &AppSettings) -> PrivacySettingsState {
    let license_accepted = license_is_accepted(settings);
    PrivacySettingsState {
        automated_web_grounding_enabled: settings.privacy.automated_web_grounding_enabled,
        license_accepted,
        license_state: settings.license.state,
        accepted_license_version: settings.license.accepted_version.clone(),
        acceptance_timestamp_ms: settings.license.accepted_at_ms,
        license_version: CURRENT_LICENSE_VERSION.to_string(),
        license_effective_date: LICENSE_EFFECTIVE_DATE.to_string(),
        license_text: LICENSE_TEXT.to_string(),
    }
}

fn write_settings(app: &tauri::AppHandle, settings: &AppSettings) -> Result<(), String> {
    let path = settings_path(app)?;
    let _guard = SETTINGS_WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    write_settings_to_path(&path, settings)
}

fn update_settings<T>(
    app: &tauri::AppHandle,
    update: impl FnOnce(&mut AppSettings) -> Result<T, String>,
) -> Result<T, String> {
    let _guard = SETTINGS_WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut settings = read_settings(app)?;
    let result = update(&mut settings)?;
    write_settings_to_path(&settings_path(app)?, &settings)?;
    Ok(result)
}

fn write_settings_to_path(path: &Path, settings: &AppSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to create the OOMU settings directory: {error}"))?;
    }

    let contents = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("Unable to serialize OOMU settings: {error}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| "OOMU settings path has no parent directory.".to_string())?;
    let temp_path = parent.join(format!(
        ".oomu_settings.{}.{}.tmp",
        std::process::id(),
        unix_time_ms()
    ));
    let write_result = (|| -> Result<(), String> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|error| format!("Unable to create temporary OOMU settings: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("Unable to protect temporary OOMU settings: {error}"))?;
        }
        file.write_all(contents.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("Unable to save temporary OOMU settings: {error}"))?;
        fs::rename(&temp_path, path)
            .map_err(|error| format!("Unable to publish OOMU settings atomically: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("Unable to protect OOMU settings: {error}"))?;
        }
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

fn reconcile_license_version(settings: &mut AppSettings) {
    if settings.license.state == LicenseAcceptanceState::Accepted && !license_is_accepted(settings)
    {
        settings.license.state = LicenseAcceptanceState::NotPresented;
        settings.license.accepted_version = None;
        settings.license.accepted_at_ms = None;
        settings.license.updated_at_ms = Some(unix_time_ms());
    } else if settings.license.state != LicenseAcceptanceState::Accepted {
        settings.license.accepted_version = None;
        settings.license.accepted_at_ms = None;
    }
}

fn license_is_accepted(settings: &AppSettings) -> bool {
    settings.license.state == LicenseAcceptanceState::Accepted
        && settings.license.accepted_version.as_deref() == Some(CURRENT_LICENSE_VERSION)
        && settings.license.accepted_at_ms.is_some()
}

fn accept_current_license(settings: &mut AppSettings, now_ms: i64) {
    settings.license.state = LicenseAcceptanceState::Accepted;
    settings.license.accepted_version = Some(CURRENT_LICENSE_VERSION.to_string());
    settings.license.accepted_at_ms = Some(now_ms);
    settings.license.updated_at_ms = Some(now_ms);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_local_and_cloud_sessions_default_to_12288_tokens() {
        assert_eq!(Settings::default().context_budget, DEFAULT_CONTEXT_BUDGET);
        assert_eq!(DEFAULT_CONTEXT_BUDGET, 12_288);
        assert_eq!(DEFAULT_CLOUD_CONTEXT_BUDGET, 12_288);
        assert_eq!(DEFAULT_AUTO_COMPACTION_THRESHOLD_PERCENT, 70);
    }

    #[cfg(unix)]
    #[test]
    fn failed_settings_write_preserves_the_last_persisted_model_choice() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "oomu-settings-write-failure-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        std::fs::create_dir_all(&root).expect("settings test root exists");
        let path = root.join("oomu_settings.json");
        let previous = AppSettings {
            default_prewarmed_model_id: Some(gemma::GEMMA_E2B_CANONICAL_ID.to_string()),
            ..AppSettings::default()
        };
        write_settings_to_path(&path, &previous).expect("previous preference saves");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o500))
            .expect("settings directory becomes read-only");
        let next = AppSettings {
            default_prewarmed_model_id: Some(gemma::GEMMA_E4B_CANONICAL_ID.to_string()),
            ..AppSettings::default()
        };

        let result = write_settings_to_path(&path, &next);

        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("settings directory becomes writable for cleanup");
        assert!(result.is_err());
        let persisted: AppSettings = serde_json::from_slice(
            &std::fs::read(&path).expect("previous preference remains readable"),
        )
        .expect("previous preference remains valid");
        assert_eq!(
            persisted.default_prewarmed_model_id.as_deref(),
            Some(gemma::GEMMA_E2B_CANONICAL_ID)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_explicit_context_budget_remains_authoritative() {
        let configured = Settings {
            context_budget: 32_768,
        };
        assert_eq!(configured.context_budget, 32_768);
    }

    #[test]
    fn honors_a_configured_model_directory() {
        let configured = AppSettings {
            local_model_directory: Some("/Users/example/OOMU/assets/models".to_string()),
            ..AppSettings::default()
        };
        assert_eq!(
            resolve_model_root_from_settings(Some(configured)),
            PathBuf::from("/Users/example/OOMU/assets/models"),
        );
    }

    #[test]
    fn falls_back_to_models_root_without_a_configured_directory() {
        assert_eq!(resolve_model_root_from_settings(None), models_root());
        assert_eq!(
            resolve_model_root_from_settings(Some(AppSettings::default())),
            models_root(),
        );
    }

    #[test]
    fn ignores_a_blank_configured_directory() {
        let blank = AppSettings {
            local_model_directory: Some("   ".to_string()),
            ..AppSettings::default()
        };
        assert_eq!(resolve_model_root_from_settings(Some(blank)), models_root());
    }

    #[test]
    fn default_startup_model_is_e2b() {
        assert_eq!(
            resolve_default_prewarmed_model_id_from_settings(None),
            "gemma-4-E2B-it-qat-q4_0-gguf",
        );
        assert_eq!(
            resolve_default_prewarmed_model_id_from_settings(Some(AppSettings::default())),
            "gemma-4-E2B-it-qat-q4_0-gguf",
        );
        assert_eq!(
            resolve_auto_route_classifier_model_id_from_settings(None),
            "gemma-4-E2B-it-qat-q4_0-gguf",
        );
        assert_eq!(
            resolve_auto_route_classifier_model_id_from_settings(Some(AppSettings::default())),
            "gemma-4-E2B-it-qat-q4_0-gguf",
        );
    }

    #[test]
    fn explicit_e4b_startup_selection_is_preserved() {
        let configured = AppSettings {
            default_prewarmed_model_id: Some("gemma-4-E4B-it-qat-q4_0-gguf".to_string()),
            ..AppSettings::default()
        };
        assert_eq!(
            resolve_default_prewarmed_model_id_from_settings(Some(configured)),
            "gemma-4-E4B-it-qat-q4_0-gguf",
        );
        let preference = resolve_startup_model_preference(AppSettings {
            default_prewarmed_model_id: Some("gemma-4-E4B-it-qat-q4_0-gguf".to_string()),
            ..AppSettings::default()
        })
        .unwrap();
        assert_eq!(
            preference.selection_source,
            gemma::StartupModelSelectionSource::ExplicitUserSelection
        );
    }

    #[test]
    fn workflow_model_resolution_preserves_the_configured_generation_model() {
        let configured = AppSettings {
            default_prewarmed_model_id: Some("gemma-4-E4B-it-qat-q4_0-gguf".to_string()),
            ..AppSettings::default()
        };
        assert_eq!(
            resolve_default_prewarmed_model_id_strict(configured).unwrap(),
            "gemma-4-E4B-it-qat-q4_0-gguf",
        );
    }

    #[test]
    fn workflow_model_resolution_rejects_an_invalid_configured_identity() {
        let configured = AppSettings {
            default_prewarmed_model_id: Some("nested/model".to_string()),
            ..AppSettings::default()
        };
        assert!(resolve_default_prewarmed_model_id_strict(configured).is_err());
    }

    #[test]
    fn hidden_classifier_identity_never_overrides_the_visible_startup_selection() {
        let configured = || {
            serde_json::from_str::<AppSettings>(
                r#"{
                    "default_prewarmed_model_id": "gemma-4-12B-it-qat-q4_0-gguf",
                    "auto_route_classifier_model_id": "gemma-4-E2B-it-qat-q4_0-gguf"
                }"#,
            )
            .expect("legacy settings remain readable while the hidden override is ignored")
        };
        assert_eq!(
            resolve_default_prewarmed_model_id_from_settings(Some(configured())),
            "gemma-4-12B-it-qat-q4_0-gguf",
        );
        assert_eq!(
            resolve_auto_route_classifier_model_id_from_settings(Some(configured())),
            "gemma-4-12B-it-qat-q4_0-gguf",
        );
    }

    #[test]
    fn default_prewarmed_model_ignores_invalid_model_ids() {
        for value in ["   ", ".", "..", "nested/model", "nested\\model"] {
            let configured = AppSettings {
                default_prewarmed_model_id: Some(value.to_string()),
                ..AppSettings::default()
            };
            assert_eq!(
                resolve_default_prewarmed_model_id_from_settings(Some(configured)),
                "gemma-4-E2B-it-qat-q4_0-gguf",
            );
        }
    }

    #[test]
    fn privacy_settings_default_automated_web_grounding_off() {
        let settings = AppSettings::default();
        assert!(!settings.privacy.automated_web_grounding_enabled);
        assert_eq!(settings.license.state, LicenseAcceptanceState::NotPresented);
    }

    #[test]
    fn accepting_current_license_unlocks_functionality_locally() {
        let mut settings = AppSettings::default();
        accept_current_license(&mut settings, 100);
        assert!(license_is_accepted(&settings));
        assert!(privacy_settings_from(&settings).license_accepted);
        assert_eq!(settings.license.accepted_at_ms, Some(100));
    }

    #[test]
    fn accepted_state_unlocks_only_current_license_and_version_change_resets_gate() {
        let mut settings = AppSettings::default();
        accept_current_license(&mut settings, 100);
        assert!(privacy_settings_from(&settings).license_accepted);

        settings.license.accepted_version = Some("obsolete".to_string());
        reconcile_license_version(&mut settings);
        assert_eq!(settings.license.state, LicenseAcceptanceState::NotPresented);
        assert!(settings.license.accepted_version.is_none());
    }

    #[test]
    fn incomplete_accepted_state_fails_closed() {
        let mut settings = AppSettings {
            license: LicenseSettings {
                state: LicenseAcceptanceState::Accepted,
                accepted_version: Some(CURRENT_LICENSE_VERSION.to_string()),
                accepted_at_ms: None,
                ..LicenseSettings::default()
            },
            ..AppSettings::default()
        };
        assert!(!license_is_accepted(&settings));
        reconcile_license_version(&mut settings);
        assert_eq!(settings.license.state, LicenseAcceptanceState::NotPresented);
    }

    #[test]
    fn settings_publish_is_atomic_private_and_leaves_no_temporary_file() {
        let root = temp_locale_dir("oomu_atomic_settings");
        let path = root.join(SETTINGS_FILE);
        write_settings_to_path(&path, &AppSettings::default()).unwrap();
        let decoded: AppSettings = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(decoded.license.state, LicenseAcceptanceState::NotPresented);
        assert!(fs::read_dir(&root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepted_state_survives_disk_restart_without_delivery_metadata() {
        let root = temp_locale_dir("oomu_accepted_license_restart");
        let path = root.join(SETTINGS_FILE);
        let mut settings = AppSettings::default();
        accept_current_license(&mut settings, 100);
        write_settings_to_path(&path, &settings).unwrap();

        let restarted: AppSettings = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(license_is_accepted(&restarted));
        assert_eq!(restarted.license.accepted_at_ms, Some(100));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn first_use_license_presented_state_has_no_acceptance_authority() {
        let settings = AppSettings {
            license: LicenseSettings {
                state: LicenseAcceptanceState::Presented,
                ..LicenseSettings::default()
            },
            ..AppSettings::default()
        };
        assert!(!license_is_accepted(&settings));
        assert!(settings.license.accepted_version.is_none());
        assert!(settings.license.accepted_at_ms.is_none());
    }

    #[test]
    fn locale_scanner_lists_only_verified_json_assets() {
        let temp_dir = temp_locale_dir("oomu_locale_scan");
        fs::write(temp_dir.join("en-US.json"), r#"{"common":{"save":"Save"}}"#).unwrap();
        fs::write(
            temp_dir.join("test-TEST.json"),
            r#"{"common":{"save":"Test Save"}}"#,
        )
        .unwrap();
        fs::write(temp_dir.join("bad-BAD.json"), "{not-json").unwrap();
        fs::write(temp_dir.join("notes.txt"), r#"{"ignored":true}"#).unwrap();

        let assets = scan_locale_assets_from_dir(&temp_dir).unwrap();
        let ids: Vec<&str> = assets.iter().map(|asset| asset.id.as_str()).collect();
        assert_eq!(ids, vec!["en-US", "test-TEST"]);
        assert!(assets.iter().all(|asset| asset.verified));

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn locale_state_uses_default_when_saved_locale_is_missing() {
        let temp_dir = temp_locale_dir("oomu_locale_state");
        fs::write(
            temp_dir.join("en-US.json"),
            r#"{"settings":{"title":"Settings"}}"#,
        )
        .unwrap();
        fs::write(
            temp_dir.join("test-TEST.json"),
            r#"{"settings":{"title":"Test Settings"}}"#,
        )
        .unwrap();
        let db_dir = temp_locale_dir("oomu_locale_state_db");
        let persistence = PersistenceEngine::initialize_at(db_dir.join("state.sqlite")).unwrap();
        persistence
            .upsert_app_preference(ACTIVE_LOCALE_SETTING_KEY, "missing-MISS")
            .unwrap();

        let assets = scan_locale_assets_from_dir(&temp_dir).unwrap();
        let state = locale_state_from_assets(&persistence, &temp_dir, assets, None).unwrap();
        assert_eq!(state.active_locale, "en-US");
        assert_eq!(state.translations["settings"]["title"], "Settings");

        let assets = scan_locale_assets_from_dir(&temp_dir).unwrap();
        let state =
            locale_state_from_assets(&persistence, &temp_dir, assets, Some("test-TEST".into()))
                .unwrap();
        assert_eq!(state.active_locale, "test-TEST");
        assert_eq!(state.translations["settings"]["title"], "Test Settings");

        let _ = fs::remove_dir_all(temp_dir);
        let _ = fs::remove_dir_all(db_dir);
    }

    #[test]
    fn locale_state_merges_active_dictionary_over_english_fallbacks() {
        let temp_dir = temp_locale_dir("oomu_locale_merge");
        fs::write(
            temp_dir.join("en-US.json"),
            r#"{"settings":{"title":"Settings","subtitle":"Configure OOMU"},"common":{"save":"Save"}}"#,
        )
        .unwrap();
        fs::write(
            temp_dir.join("test-TEST.json"),
            r#"{"settings":{"title":"Test Settings"},"extra":{"label":"Extra"}}"#,
        )
        .unwrap();
        let db_dir = temp_locale_dir("oomu_locale_merge_db");
        let persistence = PersistenceEngine::initialize_at(db_dir.join("state.sqlite")).unwrap();
        let assets = vec![
            LocaleAsset {
                id: "en-US".to_string(),
                label: "English (US)".to_string(),
                file_name: "en-US.json".to_string(),
                is_default: true,
                verified: true,
            },
            LocaleAsset {
                id: "test-TEST".to_string(),
                label: "Test Language".to_string(),
                file_name: "test-TEST.json".to_string(),
                is_default: false,
                verified: true,
            },
        ];

        let state =
            locale_state_from_assets(&persistence, &temp_dir, assets, Some("test-TEST".into()))
                .unwrap();
        assert_eq!(state.active_locale, "test-TEST");
        assert_eq!(state.translations["settings"]["title"], "Test Settings");
        assert_eq!(state.translations["settings"]["subtitle"], "Configure OOMU");
        assert_eq!(state.translations["common"]["save"], "Save");
        assert_eq!(state.translations["extra"]["label"], "Extra");

        let _ = fs::remove_dir_all(temp_dir);
        let _ = fs::remove_dir_all(db_dir);
    }

    #[test]
    fn locale_ids_reject_paths_and_normalize_case() {
        assert_eq!(normalize_locale_id("EN-us").unwrap(), "en-US");
        assert!(normalize_locale_id("../en-US").is_err());
        assert!(normalize_locale_id("en").is_err());
    }

    #[test]
    fn folder_picker_title_rejects_empty_control_and_unbounded_text() {
        assert_eq!(
            sanitize_folder_picker_title(" Choose a folder ").unwrap(),
            "Choose a folder"
        );
        assert!(sanitize_folder_picker_title("   ").is_err());
        assert!(sanitize_folder_picker_title("Choose\na folder").is_err());
        assert!(sanitize_folder_picker_title(&"x".repeat(161)).is_err());
    }

    #[test]
    fn folder_picker_initial_path_falls_back_to_existing_parent() {
        let root = temp_locale_dir("oomu_folder_picker_initial");
        let missing_child = root.join("not-created").join("child");
        assert_eq!(
            initial_folder_picker_directory(&missing_child.display().to_string()),
            Some(root.clone())
        );
        assert!(initial_folder_picker_directory("  ").is_none());
        let _ = fs::remove_dir_all(root);
    }

    fn temp_locale_dir(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "{prefix}_{}_{}",
            std::process::id(),
            unix_time_ms()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
