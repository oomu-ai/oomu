use crate::{
    agent_manager::AgentManager,
    agentic_loop::AgenticLoopError,
    background_tasks::hooks::{refresh_active_mod_hook_registry_async, BackgroundHookRegistry},
    db::PersistenceEngine,
    foundation::{
        clock::{unix_time_ms_i64 as now_ms, unix_time_ns_from},
        digest::{sha256, sha256_reader_bounded},
    },
    gemma::{resolve_local_model, GemmaService},
    settings,
    sovereign_identity::SovereignIdentity,
    OomuLaunchOptions,
};
use chrono::{DateTime, Local, NaiveDate};
use rand_core::{OsRng, RngCore};
use regex::Regex;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};

use super::{
    mod_manifest_support::{
        ensure_no_case_colliding_mod_id, exact_package_is_revoked, inferred_category,
        manifest_permissions, storage_id, valid_mod_identifier,
    },
    mod_package::{
        extract_entries_to, normalize_archive_name, parse_mod_archive, relative_archive_path,
        ArchiveEntry, MAX_MOD_ARCHIVE_SIZE,
    },
    mod_trust::{self, ModTrust, INSTALLED_MODS_SCHEMA},
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

const MOD_PACKAGE_GRANT_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_PENDING_MOD_PACKAGE_GRANTS: usize = 32;
const MOD_PACKAGE_GRANT_TOKEN_BYTES: usize = 32;
const RETIRED_EMBEDDED_MOD_ENTRYPOINTS: [&str; 2] =
    ["builtin://alignment", "builtin://developer_bundle"];
const REMOVED_BUILT_IN_MODS_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS removed_built_in_mods (
    mod_id TEXT PRIMARY KEY,
    removed_at_ms INTEGER NOT NULL
);
";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModPermission {
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledMod {
    pub id: String,
    pub name: String,
    pub description: String,
    pub is_active: bool,
    pub version: String,
    pub author: String,
    pub category: String,
    pub package_size: String,
    pub last_updated: String,
    pub review_state: String,
    pub publisher_identity_verified: bool,
    pub integrity_state: String,
    pub is_built_in: bool,
    pub permissions: Vec<ModPermission>,
    pub endpoints: Vec<String>,
    pub agent_config_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<ModCommand>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirements: Option<ModRequirements>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthorizedNetworkModCommand {
    pub(crate) mod_id: String,
    pub(crate) search_query: String,
    pub(crate) allowed_hosts: Vec<String>,
    pub(crate) context_urls: Vec<String>,
    pub(crate) required_context_evidence_patterns: Vec<String>,
}

impl AuthorizedNetworkModCommand {
    pub(crate) fn allows_url(&self, endpoint: &str) -> bool {
        let Ok(requested_host) = normalize_endpoint_host(endpoint) else {
            return false;
        };
        self.allowed_hosts.iter().any(|allowed_host| {
            allowed_host_matches(&requested_host, allowed_host).unwrap_or(false)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstalledModTrustBinding {
    pub(crate) mod_id: String,
    pub(crate) version: String,
    pub(crate) review_state: String,
    pub(crate) publisher_identity_verified: bool,
    pub(crate) integrity_state: String,
    pub(crate) payload_sha256: String,
    pub(crate) is_built_in: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ModPermissions {
    pub allowed_paths: Option<Vec<String>>,
    pub allowed_hosts: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ModRequirements {
    #[serde(default)]
    pub min_cognitive_tier: Option<String>,
    #[serde(default)]
    pub supported_provider_classes: Option<Vec<String>>,
    #[serde(default)]
    pub supported_local_models: Option<Vec<String>>,
    #[serde(default)]
    pub error_notice_override: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ModCommand {
    pub trigger: String,
    #[serde(default)]
    pub description: HashMap<String, String>,
    #[serde(default)]
    pub public_network: bool,
    #[serde(default)]
    pub context_url_templates: Vec<String>,
    #[serde(default)]
    pub context_parameters: HashMap<String, ModContextParameter>,
    /// Every declared pattern must match sanitized page/result text before it can ground a turn.
    #[serde(default)]
    pub required_context_evidence_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ModContextParameter {
    pub pattern: String,
    #[serde(default = "default_mod_context_transform")]
    pub transform: String,
}

fn default_mod_context_transform() -> String {
    "url_encode".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ModManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub package_size: Option<String>,
    #[serde(default)]
    pub last_updated: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_mod_permissions"
    )]
    pub permissions: Option<ModPermissions>,
    #[serde(default)]
    pub endpoints: Vec<String>,
    #[serde(default)]
    pub hooks: Value,
    #[serde(default)]
    pub entrypoint: String,
    #[serde(default)]
    pub default_system_prompt: Option<String>,
    #[serde(default)]
    pub agent_config_schema: Option<Value>,
    #[serde(default)]
    pub commands: Option<Vec<ModCommand>>,
    #[serde(default)]
    pub requirements: Option<ModRequirements>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModCompatibilityCheckRequest {
    pub agent_id: String,
    pub provider_id: String,
    pub model_id: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default, alias = "explicitModId")]
    pub explicit_mod_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModCompatibilityCheckResponse {
    pub ok: bool,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SecurityError {
    #[error("Unable to normalize path {path}: {reason}")]
    PathNormalizationFailed { path: String, reason: String },
    #[error("Unable to normalize endpoint {endpoint}: {reason}")]
    EndpointNormalizationFailed { endpoint: String, reason: String },
    #[error("{0}")]
    UnauthorizedAccess(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModPackageGrantResponse {
    grant_id: String,
    expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModPackageFileIdentity {
    length: u64,
    modified_nanos: Option<u128>,
    created_nanos: Option<u128>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    change_seconds: i64,
    #[cfg(unix)]
    change_nanos: i64,
}

impl ModPackageFileIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            length: metadata.len(),
            modified_nanos: metadata.modified().ok().and_then(unix_time_ns_from),
            created_nanos: metadata.created().ok().and_then(unix_time_ns_from),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            change_seconds: metadata.ctime(),
            #[cfg(unix)]
            change_nanos: metadata.ctime_nsec(),
        }
    }
}

#[derive(Debug)]
struct ModPackageGrantRecord {
    canonical_path: PathBuf,
    selected_file: fs::File,
    identity: ModPackageFileIdentity,
    digest: [u8; 32],
    metadata: fs::Metadata,
    expires_at: Instant,
}

#[derive(Debug, Default)]
struct ModPackageGrantRegistry {
    grants: HashMap<String, ModPackageGrantRecord>,
}

#[derive(Debug)]
struct VerifiedModPackage {
    archive: Vec<u8>,
    metadata: fs::Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModPackageGrantError {
    InvalidSelection,
    RegistryUnavailable,
    InvalidOrExpired,
    FileChanged,
    ReadFailed,
}

static MOD_PACKAGE_GRANTS: OnceLock<Mutex<ModPackageGrantRegistry>> = OnceLock::new();

fn mod_package_grant_registry() -> &'static Mutex<ModPackageGrantRegistry> {
    MOD_PACKAGE_GRANTS.get_or_init(|| Mutex::new(ModPackageGrantRegistry::default()))
}

fn prune_expired_mod_package_grants(registry: &Mutex<ModPackageGrantRegistry>) {
    let now = Instant::now();
    if let Ok(mut registry) = registry.lock() {
        registry.grants.retain(|_, record| record.expires_at > now);
    }
}

fn is_valid_mod_package_grant_id(grant_id: &str) -> bool {
    grant_id.len() == MOD_PACKAGE_GRANT_TOKEN_BYTES * 2
        && grant_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn random_mod_package_grant_id() -> String {
    let mut token = [0_u8; MOD_PACKAGE_GRANT_TOKEN_BYTES];
    OsRng.fill_bytes(&mut token);
    hex::encode(token)
}

fn hash_mod_package_file(file: &fs::File) -> Result<[u8; 32], ModPackageGrantError> {
    let mut file = file
        .try_clone()
        .map_err(|_| ModPackageGrantError::ReadFailed)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| ModPackageGrantError::ReadFailed)?;
    sha256_reader_bounded(file, MAX_MOD_ARCHIVE_SIZE)
        .map_err(|_| ModPackageGrantError::ReadFailed)?
        .map(|digest| *digest.as_bytes())
        .ok_or(ModPackageGrantError::InvalidSelection)
}

fn read_mod_package_file(file: &fs::File) -> Result<(Vec<u8>, [u8; 32]), ModPackageGrantError> {
    let mut file = file
        .try_clone()
        .map_err(|_| ModPackageGrantError::ReadFailed)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| ModPackageGrantError::ReadFailed)?;
    let mut limited = file.take(MAX_MOD_ARCHIVE_SIZE + 1);
    let mut archive = Vec::new();
    limited
        .read_to_end(&mut archive)
        .map_err(|_| ModPackageGrantError::ReadFailed)?;
    if archive.len() as u64 > MAX_MOD_ARCHIVE_SIZE {
        return Err(ModPackageGrantError::FileChanged);
    }
    let digest = *sha256(&archive).as_bytes();
    Ok((archive, digest))
}

fn issue_mod_package_grant(
    registry: &Mutex<ModPackageGrantRegistry>,
    selected_path: &Path,
    ttl: Duration,
) -> Result<ModPackageGrantResponse, ModPackageGrantError> {
    let extension = selected_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("oomu") {
        return Err(ModPackageGrantError::InvalidSelection);
    }

    let selected_metadata =
        fs::symlink_metadata(selected_path).map_err(|_| ModPackageGrantError::ReadFailed)?;
    if selected_metadata.file_type().is_symlink() || !selected_metadata.is_file() {
        return Err(ModPackageGrantError::InvalidSelection);
    }
    if selected_metadata.len() > MAX_MOD_ARCHIVE_SIZE {
        return Err(ModPackageGrantError::InvalidSelection);
    }

    let canonical_path =
        fs::canonicalize(selected_path).map_err(|_| ModPackageGrantError::ReadFailed)?;
    let selected_file =
        fs::File::open(&canonical_path).map_err(|_| ModPackageGrantError::ReadFailed)?;
    let metadata = selected_file
        .metadata()
        .map_err(|_| ModPackageGrantError::ReadFailed)?;
    let identity = ModPackageFileIdentity::from_metadata(&metadata);
    if !metadata.is_file()
        || metadata.len() > MAX_MOD_ARCHIVE_SIZE
        || identity != ModPackageFileIdentity::from_metadata(&selected_metadata)
    {
        return Err(ModPackageGrantError::FileChanged);
    }

    let digest = hash_mod_package_file(&selected_file)?;
    let final_metadata =
        fs::symlink_metadata(&canonical_path).map_err(|_| ModPackageGrantError::FileChanged)?;
    if final_metadata.file_type().is_symlink()
        || identity != ModPackageFileIdentity::from_metadata(&final_metadata)
        || identity
            != ModPackageFileIdentity::from_metadata(
                &selected_file
                    .metadata()
                    .map_err(|_| ModPackageGrantError::ReadFailed)?,
            )
    {
        return Err(ModPackageGrantError::FileChanged);
    }

    let issued_at = Instant::now();
    let expires_at = issued_at + ttl;
    let expires_at_ms = now_ms().saturating_add(ttl.as_millis().min(i64::MAX as u128) as i64);
    let mut registry = registry
        .lock()
        .map_err(|_| ModPackageGrantError::RegistryUnavailable)?;
    registry
        .grants
        .retain(|_, record| record.expires_at > issued_at);
    while registry.grants.len() >= MAX_PENDING_MOD_PACKAGE_GRANTS {
        let Some(oldest_id) = registry
            .grants
            .iter()
            .min_by_key(|(_, record)| record.expires_at)
            .map(|(grant_id, _)| grant_id.clone())
        else {
            break;
        };
        registry.grants.remove(&oldest_id);
    }

    let grant_id = loop {
        let candidate = random_mod_package_grant_id();
        if !registry.grants.contains_key(&candidate) {
            break candidate;
        }
    };
    registry.grants.insert(
        grant_id.clone(),
        ModPackageGrantRecord {
            canonical_path,
            selected_file,
            identity,
            digest,
            metadata,
            expires_at,
        },
    );

    Ok(ModPackageGrantResponse {
        grant_id,
        expires_at_ms,
    })
}

fn consume_mod_package_grant(
    registry: &Mutex<ModPackageGrantRegistry>,
    grant_id: &str,
) -> Result<VerifiedModPackage, ModPackageGrantError> {
    if !is_valid_mod_package_grant_id(grant_id) {
        return Err(ModPackageGrantError::InvalidOrExpired);
    }

    let now = Instant::now();
    let record = {
        let mut registry = registry
            .lock()
            .map_err(|_| ModPackageGrantError::RegistryUnavailable)?;
        registry.grants.retain(|_, record| record.expires_at > now);
        registry
            .grants
            .remove(grant_id)
            .ok_or(ModPackageGrantError::InvalidOrExpired)?
    };

    let current_canonical =
        fs::canonicalize(&record.canonical_path).map_err(|_| ModPackageGrantError::FileChanged)?;
    if current_canonical != record.canonical_path {
        return Err(ModPackageGrantError::FileChanged);
    }
    let path_metadata = fs::symlink_metadata(&record.canonical_path)
        .map_err(|_| ModPackageGrantError::FileChanged)?;
    if path_metadata.file_type().is_symlink()
        || ModPackageFileIdentity::from_metadata(&path_metadata) != record.identity
    {
        return Err(ModPackageGrantError::FileChanged);
    }

    let current_file =
        fs::File::open(&record.canonical_path).map_err(|_| ModPackageGrantError::FileChanged)?;
    if ModPackageFileIdentity::from_metadata(
        &current_file
            .metadata()
            .map_err(|_| ModPackageGrantError::FileChanged)?,
    ) != record.identity
    {
        return Err(ModPackageGrantError::FileChanged);
    }
    if ModPackageFileIdentity::from_metadata(
        &record
            .selected_file
            .metadata()
            .map_err(|_| ModPackageGrantError::FileChanged)?,
    ) != record.identity
    {
        return Err(ModPackageGrantError::FileChanged);
    }

    let (archive, digest) = read_mod_package_file(&record.selected_file)?;
    if digest != record.digest
        || ModPackageFileIdentity::from_metadata(
            &record
                .selected_file
                .metadata()
                .map_err(|_| ModPackageGrantError::FileChanged)?,
        ) != record.identity
        || ModPackageFileIdentity::from_metadata(
            &fs::symlink_metadata(&record.canonical_path)
                .map_err(|_| ModPackageGrantError::FileChanged)?,
        ) != record.identity
    {
        return Err(ModPackageGrantError::FileChanged);
    }

    Ok(VerifiedModPackage {
        archive,
        metadata: record.metadata,
    })
}

fn mod_package_picker_error(error: ModPackageGrantError) -> AgenticLoopError {
    match error {
        ModPackageGrantError::InvalidSelection => mod_error(
            "mod_picker_invalid_file",
            "Only a regular .oomu package within the supported size limit can be selected."
                .to_string(),
        ),
        ModPackageGrantError::RegistryUnavailable => mod_error(
            "mod_package_grant_unavailable",
            "Mod package authorization is temporarily unavailable.".to_string(),
        ),
        ModPackageGrantError::InvalidOrExpired
        | ModPackageGrantError::FileChanged
        | ModPackageGrantError::ReadFailed => mod_error(
            "mod_picker_failed",
            "The selected mod package could not be authorized.".to_string(),
        ),
    }
}

fn mod_package_consume_error(error: ModPackageGrantError) -> AgenticLoopError {
    match error {
        ModPackageGrantError::InvalidOrExpired => mod_error(
            "mod_package_grant_invalid",
            "The mod package authorization is invalid or expired. Select the package again."
                .to_string(),
        ),
        ModPackageGrantError::RegistryUnavailable => mod_error(
            "mod_package_grant_unavailable",
            "Mod package authorization is temporarily unavailable.".to_string(),
        ),
        ModPackageGrantError::InvalidSelection
        | ModPackageGrantError::FileChanged
        | ModPackageGrantError::ReadFailed => mod_error(
            "mod_package_grant_stale",
            "The selected mod package changed or became unavailable. Select it again.".to_string(),
        ),
    }
}

#[tauri::command]
pub async fn choose_mod_package_path() -> Result<Option<ModPackageGrantResponse>, AgenticLoopError>
{
    let Some(selected_file) = rfd::AsyncFileDialog::new()
        .set_title("Install OOMU Mod Package")
        .add_filter("OOMU Mod Package", &["oomu"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };

    let path = selected_file.path().to_path_buf();
    let grant = tauri::async_runtime::spawn_blocking(move || {
        issue_mod_package_grant(mod_package_grant_registry(), &path, MOD_PACKAGE_GRANT_TTL)
    })
    .await
    .map_err(|_| {
        mod_error(
            "mod_picker_failed",
            "The selected mod package could not be authorized.".to_string(),
        )
    })?
    .map_err(mod_package_picker_error)?;
    let _ = tauri::async_runtime::spawn(async move {
        tokio::time::sleep(MOD_PACKAGE_GRANT_TTL).await;
        prune_expired_mod_package_grants(mod_package_grant_registry());
    });
    Ok(Some(grant))
}

#[tauri::command]
pub async fn list_installed_mods(
    persistence: tauri::State<'_, PersistenceEngine>,
    launch_options: tauri::State<'_, OomuLaunchOptions>,
) -> Result<Vec<InstalledMod>, AgenticLoopError> {
    let engine = persistence.inner().clone();
    let safe_mode = launch_options.inner().safe_mode;
    tauri::async_runtime::spawn_blocking(move || {
        let mods = select_installed_mods(&engine)?;
        Ok(filter_installed_mods_for_safe_mode(mods, safe_mode))
    })
    .await
    .map_err(|error| mod_error("mod_task_failed", error.to_string()))?
    .map_err(|error| mod_error("mod_list_failed", error))
}

#[tauri::command]
pub async fn validate_mod_compatibility_for_turn(
    request: ModCompatibilityCheckRequest,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
    agents: tauri::State<'_, AgentManager>,
    launch_options: tauri::State<'_, OomuLaunchOptions>,
) -> Result<ModCompatibilityCheckResponse, AgenticLoopError> {
    if launch_options.inner().safe_mode {
        return Ok(ModCompatibilityCheckResponse { ok: true });
    }

    let agent_id = clean_binding_id("agent_id", &request.agent_id)
        .map_err(|error| mod_error("mod_requirement_invalid_agent", error))?;
    let provider_id = clean_route_text("provider_id", &request.provider_id)
        .map_err(|error| mod_error("mod_requirement_invalid_route", error))?;
    let model_id = clean_route_text("model_id", &request.model_id)
        .map_err(|error| mod_error("mod_requirement_invalid_route", error))?;
    let locale = normalize_locale_id(request.locale.as_deref());
    let message = request.message.unwrap_or_default();
    let explicit_mod_id = request
        .explicit_mod_id
        .as_deref()
        .map(|mod_id| clean_binding_id("mod_id", mod_id))
        .transpose()
        .map_err(|error| mod_error("mod_requirement_invalid_mod", error))?;

    let bound_mod_ids = agents
        .inner()
        .clone()
        .get_agent_mods(agent_id)
        .await
        .map_err(|error| mod_error("mod_requirement_binding_lookup_failed", error))?;
    let provider_class_id = provider_class_for_route(agents.inner(), &provider_id)
        .map_err(|error| mod_error("mod_requirement_route_lookup_failed", error))?;
    let model_id = effective_model_id_for_validation(&app, &provider_class_id, &model_id);
    let engine = persistence.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        validate_active_mods_for_turn(
            &engine,
            &bound_mod_ids,
            explicit_mod_id.as_deref(),
            &message,
            &provider_class_id,
            &model_id,
            &locale,
        )
    })
    .await
    .map_err(|error| mod_error("mod_task_failed", error.to_string()))?
    .map_err(|error| mod_error("mod_requirement_blocked", error))?;

    Ok(ModCompatibilityCheckResponse { ok: true })
}

#[tauri::command]
pub async fn set_mod_active_state(
    mod_id: String,
    active: bool,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
    background_hooks: tauri::State<'_, BackgroundHookRegistry>,
    gemma: tauri::State<'_, GemmaService>,
    identity: tauri::State<'_, SovereignIdentity>,
    launch_options: tauri::State<'_, OomuLaunchOptions>,
) -> Result<(), AgenticLoopError> {
    let safe_mode = launch_options.inner().safe_mode;
    if safe_mode && active {
        return Err(mod_error(
            "mod_safe_mode_active",
            "Safe Mode is active. Capability mods cannot be activated.".to_string(),
        ));
    }

    let engine = persistence.inner().clone();
    let hook_engine = engine.clone();
    let hook_registry = background_hooks.inner().clone();
    let hook_gemma = gemma.inner().clone();
    let hook_identity = identity.inner().clone();
    tauri::async_runtime::spawn_blocking(move || update_mod_active_state(&engine, &mod_id, active))
        .await
        .map_err(|error| mod_error("mod_task_failed", error.to_string()))?
        .map_err(|error| mod_error("mod_toggle_failed", error))?;

    refresh_active_mod_hook_registry_async(
        app,
        hook_registry,
        hook_engine,
        hook_gemma,
        hook_identity,
        safe_mode,
    );
    Ok(())
}

#[tauri::command]
pub async fn bind_mod_to_agent(
    agent_id: String,
    mod_id: String,
    persistence: tauri::State<'_, PersistenceEngine>,
    agents: tauri::State<'_, AgentManager>,
    launch_options: tauri::State<'_, OomuLaunchOptions>,
) -> Result<(), AgenticLoopError> {
    let agent_id = clean_binding_id("agent_id", &agent_id)
        .map_err(|error| mod_error("mod_binding_invalid_agent", error))?;
    let mod_id = clean_binding_id("mod_id", &mod_id)
        .map_err(|error| mod_error("mod_binding_invalid_mod", error))?;
    if launch_options.inner().safe_mode {
        return Err(mod_error(
            "mod_safe_mode_active",
            "Safe Mode is active. Capability mods cannot be bound to agents.".to_string(),
        ));
    }
    ensure_agent_exists(&agents, &agent_id).await?;
    let engine = persistence.inner().clone();
    let lookup_mod_id = mod_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        ensure_installed_mod_exists(&engine, &lookup_mod_id)
    })
    .await
    .map_err(|error| mod_error("mod_task_failed", error.to_string()))?
    .map_err(|error| mod_error("mod_binding_failed", error))?;

    agents
        .inner()
        .clone()
        .bind_mod_to_agent(agent_id, mod_id)
        .await
        .map_err(|error| mod_error("mod_binding_failed", error))
}

#[tauri::command]
pub async fn unbind_mod_to_agent(
    agent_id: String,
    mod_id: String,
    agents: tauri::State<'_, AgentManager>,
) -> Result<(), AgenticLoopError> {
    let agent_id = clean_binding_id("agent_id", &agent_id)
        .map_err(|error| mod_error("mod_binding_invalid_agent", error))?;
    let mod_id = clean_binding_id("mod_id", &mod_id)
        .map_err(|error| mod_error("mod_binding_invalid_mod", error))?;
    ensure_agent_exists(&agents, &agent_id).await?;

    agents
        .inner()
        .clone()
        .unbind_mod_to_agent(agent_id, mod_id)
        .await
        .map_err(|error| mod_error("mod_unbinding_failed", error))
}

#[tauri::command]
pub async fn get_agent_mods(
    agent_id: String,
    agents: tauri::State<'_, AgentManager>,
    launch_options: tauri::State<'_, OomuLaunchOptions>,
) -> Result<Vec<String>, AgenticLoopError> {
    let agent_id = clean_binding_id("agent_id", &agent_id)
        .map_err(|error| mod_error("mod_binding_invalid_agent", error))?;
    ensure_agent_exists(&agents, &agent_id).await?;

    let safe_mode = launch_options.inner().safe_mode;
    agents
        .inner()
        .clone()
        .get_agent_mods(agent_id)
        .await
        .map(|mod_ids| filter_mod_ids_for_safe_mode(mod_ids, safe_mode))
        .map_err(|error| mod_error("mod_binding_list_failed", error))
}

/// The legacy command name is retained for capability compatibility; it accepts only a
/// picker-issued grant and never renderer-supplied filesystem authority.
#[tauri::command]
pub async fn install_mod_from_path(
    grant_id: String,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
    background_hooks: tauri::State<'_, BackgroundHookRegistry>,
    gemma: tauri::State<'_, GemmaService>,
    identity: tauri::State<'_, SovereignIdentity>,
    launch_options: tauri::State<'_, OomuLaunchOptions>,
) -> Result<InstalledMod, AgenticLoopError> {
    let safe_mode = launch_options.inner().safe_mode;
    let verified_package = tauri::async_runtime::spawn_blocking(move || {
        consume_mod_package_grant(mod_package_grant_registry(), &grant_id)
    })
    .await
    .map_err(|_| {
        mod_error(
            "mod_task_failed",
            "The mod package authorization task failed.".to_string(),
        )
    })?
    .map_err(mod_package_consume_error)?;

    if safe_mode {
        return Err(mod_error(
            "mod_safe_mode_active",
            "Safe Mode is active. Capability mods cannot be installed.".to_string(),
        ));
    }

    let engine = persistence.inner().clone();
    let hook_engine = engine.clone();
    let hook_registry = background_hooks.inner().clone();
    let hook_gemma = gemma.inner().clone();
    let hook_identity = identity.inner().clone();
    let installed =
        tauri::async_runtime::spawn_blocking(move || install_mod(&engine, verified_package))
            .await
            .map_err(|_| {
                mod_error(
                    "mod_task_failed",
                    "The mod package installation task failed.".to_string(),
                )
            })?
            .map_err(|_| {
                mod_error(
                    "mod_install_failed",
                    "The selected mod package could not be installed.".to_string(),
                )
            })?;
    refresh_active_mod_hook_registry_async(
        app,
        hook_registry,
        hook_engine,
        hook_gemma,
        hook_identity,
        safe_mode,
    );
    Ok(installed)
}

#[tauri::command]
pub async fn uninstall_mod(
    mod_id: String,
    app: tauri::AppHandle,
    persistence: tauri::State<'_, PersistenceEngine>,
    agents: tauri::State<'_, AgentManager>,
    background_hooks: tauri::State<'_, BackgroundHookRegistry>,
    gemma: tauri::State<'_, GemmaService>,
    identity: tauri::State<'_, SovereignIdentity>,
    launch_options: tauri::State<'_, OomuLaunchOptions>,
) -> Result<(), AgenticLoopError> {
    let binding_mod_id = mod_id.clone();
    agents
        .inner()
        .clone()
        .unbind_mod_from_all_agents(binding_mod_id)
        .await
        .map_err(|error| mod_error("mod_uninstall_failed", error))?;
    let engine = persistence.inner().clone();
    let hook_engine = engine.clone();
    let hook_registry = background_hooks.inner().clone();
    let hook_gemma = gemma.inner().clone();
    let hook_identity = identity.inner().clone();
    let safe_mode = launch_options.inner().safe_mode;
    tauri::async_runtime::spawn_blocking(move || delete_installed_mod(&engine, &mod_id))
        .await
        .map_err(|error| mod_error("mod_task_failed", error.to_string()))?
        .map_err(|error| mod_error("mod_uninstall_failed", error))?;
    refresh_active_mod_hook_registry_async(
        app,
        hook_registry,
        hook_engine,
        hook_gemma,
        hook_identity,
        safe_mode,
    );
    Ok(())
}

pub(super) fn select_installed_mods(
    engine: &PersistenceEngine,
) -> Result<Vec<InstalledMod>, String> {
    reverify_external_mods(engine, false)?;
    let _guard = engine.lock_writes();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    ensure_schema(&connection)?;
    let mut statement = connection
        .prepare(
            "
            SELECT id, name, description, is_active, version, author, category,
                   package_size, last_updated, permissions_json, endpoints_json,
                   manifest_json, review_state, publisher_identity_verified,
                   integrity_state, is_built_in
            FROM installed_mods
            ORDER BY name COLLATE NOCASE
            ",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], installed_mod_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

pub(crate) fn authorize_active_network_mod_command(
    engine: &PersistenceEngine,
    requested_mod_id: &str,
    originating_utterance: &str,
) -> Result<AuthorizedNetworkModCommand, String> {
    let requested_mod_id = requested_mod_id.trim();
    let utterance = originating_utterance.trim();
    let (trigger, arguments) = utterance
        .split_once(char::is_whitespace)
        .map(|(trigger, arguments)| (trigger.trim(), arguments.trim()))
        .unwrap_or((utterance, ""));
    if requested_mod_id.is_empty()
        || !trigger.starts_with('/')
        || arguments.is_empty()
        || crate::local_app_intent::has_private_app_data_intent(utterance)
    {
        return Err("The mod command did not authorize a public headless search.".to_string());
    }

    let installed = select_installed_mods(engine)?
        .into_iter()
        .find(|installed_mod| installed_mod.id.eq_ignore_ascii_case(requested_mod_id))
        .ok_or_else(|| "The requested mod is not installed.".to_string())?;
    if !installed.is_active
        || installed.review_state == "revoked"
        || installed.integrity_state == "modified"
    {
        return Err("The requested mod is not active and trusted.".to_string());
    }
    let command = installed
        .commands
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|command| command.trigger.trim().eq_ignore_ascii_case(trigger))
        .ok_or_else(|| "The active mod does not declare this slash command.".to_string())?;
    if !mod_command_requests_public_network(command) {
        return Err("This mod command is local-only and did not authorize web access.".to_string());
    }
    let context_url_templates = command.context_url_templates.clone();
    let required_context_evidence_patterns = command
        .required_context_evidence_patterns
        .iter()
        .map(|pattern| pattern.trim())
        .filter(|pattern| !pattern.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let installed_id = installed.id.clone();
    let allowed_hosts = installed
        .endpoints
        .into_iter()
        .filter(|endpoint| {
            let endpoint = endpoint.trim();
            !endpoint.is_empty() && !endpoint.eq_ignore_ascii_case("none declared")
        })
        .collect::<Vec<_>>();
    if allowed_hosts.is_empty() {
        return Err("The active mod does not declare any network hosts.".to_string());
    }
    let search_topic = format!(
        "{} {}",
        trigger.trim_start_matches('/').replace(['-', '_'], " "),
        arguments
    )
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ");
    if search_topic.is_empty() {
        return Err("The mod command requires a public search topic.".to_string());
    }

    let declared_host_authority = AuthorizedNetworkModCommand {
        mod_id: installed_id.clone(),
        search_query: search_topic.clone(),
        allowed_hosts: allowed_hosts.clone(),
        context_urls: Vec::new(),
        required_context_evidence_patterns: Vec::new(),
    };
    let context_urls = render_mod_context_urls(
        &context_url_templates,
        &command.context_parameters,
        arguments,
        &declared_host_authority,
    )?;

    Ok(AuthorizedNetworkModCommand {
        mod_id: installed_id,
        search_query: search_topic,
        allowed_hosts,
        context_urls,
        required_context_evidence_patterns,
    })
}

fn mod_command_requests_public_network(command: &ModCommand) -> bool {
    // Network authority must be an explicit, reviewable manifest capability. A localized
    // description is presentation copy, not permission: inferring authority from words such as
    // "search" let older/local-only mod versions silently fall through to a generic web search.
    command.public_network || !command.context_url_templates.is_empty()
}

fn render_mod_context_urls(
    templates: &[String],
    context_parameters: &HashMap<String, ModContextParameter>,
    arguments: &str,
    authority: &AuthorizedNetworkModCommand,
) -> Result<Vec<String>, String> {
    let encoded_arguments =
        url::form_urlencoded::byte_serialize(arguments.as_bytes()).collect::<String>();
    let mut replacements = HashMap::from([("query".to_string(), encoded_arguments)]);
    for (name, parameter) in context_parameters {
        let pattern = Regex::new(&parameter.pattern)
            .map_err(|_| "A network mod context parameter pattern is invalid.".to_string())?;
        let Some(value) = pattern
            .captures(arguments)
            .and_then(|captures| captures.get(1))
            .map(|capture| capture.as_str().trim())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let transformed = match transform_mod_context_parameter(value, &parameter.transform) {
            Ok(transformed) => transformed,
            Err(_) if parameter.transform.trim() == "date_iso" => continue,
            Err(error) => return Err(error),
        };
        replacements.insert(name.clone(), transformed);
    }

    templates
        .iter()
        .take(3)
        .map(|template| template.trim())
        .filter(|template| !template.is_empty())
        .filter_map(|template| {
            let rendered = render_mod_context_template(template, &replacements)?;
            Some((|| {
                let parsed = url::Url::parse(&rendered)
                    .map_err(|_| "The network mod context URL template is invalid.".to_string())?;
                if parsed.scheme() != "https"
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                {
                    return Err(
                    "Network mod context URL templates must resolve to credential-free HTTPS URLs."
                        .to_string(),
                );
                }
                if !authority.allows_url(parsed.as_str()) {
                    return Err(
                        "The network mod context URL is outside its declared hosts.".to_string()
                    );
                }
                Ok(parsed.to_string())
            })())
        })
        .collect()
}

fn render_mod_context_template(
    template: &str,
    replacements: &HashMap<String, String>,
) -> Option<String> {
    let placeholder_pattern = Regex::new(r"\{([a-z][a-z0-9_]{0,31})\}").ok()?;
    let mut rendered = String::with_capacity(template.len());
    let mut copied_until = 0;
    for captures in placeholder_pattern.captures_iter(template) {
        let placeholder = captures.get(0)?;
        let name = captures.get(1)?.as_str();
        let value = replacements.get(name)?;
        rendered.push_str(&template[copied_until..placeholder.start()]);
        rendered.push_str(value);
        copied_until = placeholder.end();
    }
    rendered.push_str(&template[copied_until..]);
    Some(rendered)
}

fn transform_mod_context_parameter(value: &str, transform: &str) -> Result<String, String> {
    let transformed = match transform.trim() {
        "url_encode" | "" => value.to_string(),
        "uppercase" => value.to_ascii_uppercase(),
        "lowercase" => value.to_ascii_lowercase(),
        "date_iso" => parse_mod_context_date(value)
            .ok_or_else(|| "A network mod context date parameter could not be parsed.".to_string())?
            .format("%Y-%m-%d")
            .to_string(),
        _ => return Err("A network mod context parameter transform is invalid.".to_string()),
    };
    Ok(url::form_urlencoded::byte_serialize(transformed.as_bytes()).collect())
}

fn parse_mod_context_date(value: &str) -> Option<NaiveDate> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    ["%Y-%m-%d", "%B %e, %Y", "%B %e %Y", "%b %e, %Y", "%b %e %Y"]
        .iter()
        .find_map(|format| NaiveDate::parse_from_str(&normalized, format).ok())
}

fn reverify_external_mods(engine: &PersistenceEngine, active_only: bool) -> Result<(), String> {
    let _operation_guard = mod_trust::lock_mod_package_operation()?;
    let ids = {
        let _guard = engine.lock_writes();
        let connection = engine
            .open_connection()
            .map_err(|error| error.to_string())?;
        ensure_schema(&connection)?;
        let sql = if active_only {
            "SELECT id FROM installed_mods WHERE is_built_in=0 AND is_active=1"
        } else {
            "SELECT id FROM installed_mods WHERE is_built_in=0"
        };
        let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
        let collected = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        collected
    };
    for id in ids {
        if let Err(error) = reverify_installed_mod_trust(engine, &id) {
            if active_only {
                return Err(format!("Active mod verification failed: {error}"));
            }
        }
    }
    Ok(())
}

fn filter_installed_mods_for_safe_mode(
    mods: Vec<InstalledMod>,
    safe_mode: bool,
) -> Vec<InstalledMod> {
    if safe_mode {
        Vec::new()
    } else {
        mods
    }
}

fn filter_mod_ids_for_safe_mode(mod_ids: Vec<String>, safe_mode: bool) -> Vec<String> {
    if safe_mode {
        Vec::new()
    } else {
        mod_ids
    }
}

fn update_mod_active_state(
    engine: &PersistenceEngine,
    mod_id: &str,
    active: bool,
) -> Result<(), String> {
    if update_mod_active_state_row(engine, mod_id, active)? {
        return Ok(());
    }

    let installed_dir = installed_mod_directory(mod_id)?;
    update_mod_active_state_from_directory(engine, mod_id, active, &installed_dir)
}

fn update_mod_active_state_from_directory(
    engine: &PersistenceEngine,
    mod_id: &str,
    active: bool,
    installed_dir: &Path,
) -> Result<(), String> {
    if update_mod_active_state_row(engine, mod_id, active)? {
        return Ok(());
    }

    if recover_installed_mod_from_directory(engine, mod_id, installed_dir)? {
        if update_mod_active_state_row(engine, mod_id, active)? {
            return Ok(());
        }
    }

    Err(format!("Installed mod {mod_id} was not found."))
}

fn update_mod_active_state_row(
    engine: &PersistenceEngine,
    mod_id: &str,
    active: bool,
) -> Result<bool, String> {
    let _operation_guard = mod_trust::lock_mod_package_operation()?;
    let exists = {
        let _guard = engine.lock_writes();
        let connection = engine
            .open_connection()
            .map_err(|error| error.to_string())?;
        ensure_schema(&connection)?;
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM installed_mods WHERE id=?1)",
                params![mod_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| error.to_string())?
    };
    if !exists {
        return Ok(false);
    }
    let activation_binding = if active {
        let trust = reverify_installed_mod_trust(engine, mod_id)?;
        if trust.review_state == "revoked" || trust.integrity_state == "modified" {
            return Err("This mod is no longer available for activation.".to_string());
        }
        if !trust.is_built_in {
            let connection = engine
                .open_connection()
                .map_err(|error| error.to_string())?;
            let reviewed_activation: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM capability_bundle_records WHERE mod_id=?1 AND package_version=?2 AND payload_sha256=?3 AND install_state IN ('active','disabled') AND review_state<>'revoked')",
                params![trust.mod_id, trust.version, trust.payload_sha256],
                |row| row.get(0),
            ).map_err(|error| error.to_string())?;
            if !reviewed_activation {
                return Err("Review what this mod can do before turning it on.".to_string());
            }
        }
        Some(trust)
    } else {
        None
    };
    let _guard = engine.lock_writes();
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    ensure_schema(&connection)?;
    if !active {
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let changed = transaction
            .execute(
                "UPDATE installed_mods SET is_active=0,updated_at_ms=?2 WHERE id=?1",
                params![mod_id, now_ms()],
            )
            .map_err(|error| error.to_string())?;
        transaction.execute(
            "UPDATE capability_bundle_records SET install_state='disabled',updated_at_ms=?2 WHERE mod_id=?1 AND install_state='active'",
            params![mod_id, now_ms()],
        ).map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        return Ok(changed > 0);
    }
    let trust = activation_binding.expect("active transitions have a verified binding");
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    if !trust.is_built_in {
        let approved = transaction.execute(
            "UPDATE capability_bundle_records SET install_state='active',updated_at_ms=?4 WHERE mod_id=?1 AND package_version=?2 AND payload_sha256=?3 AND install_state IN ('active','disabled') AND review_state<>'revoked'",
            params![trust.mod_id, trust.version, trust.payload_sha256, now_ms()],
        ).map_err(|error| error.to_string())?;
        if approved != 1 {
            return Err("Review what this mod can do before turning it on.".to_string());
        }
    }
    let changed = transaction
        .execute(
            "UPDATE installed_mods SET is_active=1,updated_at_ms=?2 WHERE id=?1",
            params![mod_id, now_ms()],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(changed > 0)
}

pub(crate) fn reverify_installed_mod_trust(
    engine: &PersistenceEngine,
    mod_id: &str,
) -> Result<InstalledModTrustBinding, String> {
    let _guard = engine.lock_writes();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    ensure_schema(&connection)?;
    let stored: Option<(String, String, String, bool, String)> = connection
        .query_row(
            "SELECT installed_path,version,review_state,is_built_in,payload_sha256 FROM installed_mods WHERE id=?1",
            params![mod_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get::<_, i64>(3)? != 0, row.get(4)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((installed_path, version, stored_review, is_built_in, stored_payload)) = stored else {
        return Err("That mod is not installed.".to_string());
    };
    if is_built_in {
        return Ok(InstalledModTrustBinding {
            mod_id: mod_id.to_string(),
            version,
            review_state: stored_review,
            publisher_identity_verified: false,
            integrity_state: "verified".to_string(),
            payload_sha256: stored_payload,
            is_built_in: true,
        });
    }
    let evaluation = match mod_trust::evaluate_installed_directory(Path::new(&installed_path)) {
        Ok(evaluation) => evaluation,
        Err(error) => {
            connection
                .execute(
                    "UPDATE installed_mods SET is_active=0,review_state=CASE WHEN review_state='revoked' THEN 'revoked' ELSE 'unreviewed' END,publisher_identity_verified=0,integrity_state='modified',updated_at_ms=?2 WHERE id=?1",
                    params![mod_id, now_ms()],
                )
                .map_err(|database_error| database_error.to_string())?;
            return Err(error);
        }
    };
    if evaluation.mod_id != mod_id || evaluation.version != version {
        connection.execute(
            "UPDATE installed_mods SET is_active=0,review_state=CASE WHEN review_state='revoked' THEN 'revoked' ELSE 'unreviewed' END,publisher_identity_verified=0,integrity_state='modified',updated_at_ms=?2 WHERE id=?1",
            params![mod_id, now_ms()],
        ).map_err(|error| error.to_string())?;
        return Err("The installed mod no longer matches its original identity.".to_string());
    }
    let manifest_json =
        serde_json::to_string(&evaluation.manifest).map_err(|error| error.to_string())?;
    let default_system_prompt = evaluation
        .manifest
        .get("default_system_prompt")
        .and_then(Value::as_str);
    let exact_revocation = exact_package_is_revoked(
        &connection,
        mod_id,
        &version,
        &evaluation.trust.payload_sha256,
        &evaluation.manifest,
        &manifest_json,
    )?;
    let trust = evaluation.trust;
    let changed_since_inspection =
        !stored_payload.is_empty() && stored_payload != trust.payload_sha256;
    let review_state = if stored_review == "revoked" || exact_revocation {
        "revoked"
    } else if changed_since_inspection {
        "unreviewed"
    } else {
        trust.review_state
    };
    let integrity_state = if changed_since_inspection {
        "modified"
    } else {
        trust.integrity_state
    };
    let persisted_payload = if changed_since_inspection {
        stored_payload.as_str()
    } else {
        trust.payload_sha256.as_str()
    };
    if integrity_state == "modified" {
        connection.execute(
            "UPDATE capability_bundle_records SET install_state='quarantined',review_state=CASE WHEN review_state='revoked' THEN 'revoked' ELSE 'unreviewed' END,updated_at_ms=?4 WHERE mod_id=?1 AND package_version=?2 AND payload_sha256=?3 AND install_state='active'",
            params![mod_id, version, stored_payload, now_ms()],
        ).map_err(|error| error.to_string())?;
    }
    connection
        .execute(
            "UPDATE installed_mods SET review_state=?2,publisher_identity_verified=?3,integrity_state=?4,payload_sha256=?5,manifest_json=?6,default_system_prompt=?7,is_active=CASE WHEN ?2='revoked' OR ?4='modified' OR ?8 THEN 0 ELSE is_active END,updated_at_ms=?9 WHERE id=?1",
            params![mod_id, review_state, bool_to_db(trust.publisher_identity_verified), integrity_state, persisted_payload, manifest_json, default_system_prompt, changed_since_inspection, now_ms()],
        )
        .map_err(|error| error.to_string())?;
    if !changed_since_inspection {
        connection
            .execute(
                "UPDATE capability_bundle_records SET review_state=CASE WHEN review_state='revoked' THEN 'revoked' ELSE ?4 END,updated_at_ms=?5 WHERE mod_id=?1 AND package_version=?2 AND payload_sha256=?3",
                params![mod_id, version, trust.payload_sha256, trust.review_state, now_ms()],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(InstalledModTrustBinding {
        mod_id: mod_id.to_string(),
        version,
        review_state: review_state.to_string(),
        publisher_identity_verified: trust.publisher_identity_verified,
        integrity_state: integrity_state.to_string(),
        payload_sha256: trust.payload_sha256,
        is_built_in: false,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveModPromptContext {
    pub prompt: String,
    pub applied_mod_ids: Vec<String>,
    pub selection_mode: &'static str,
}

pub(crate) fn active_mod_prompt_context_details(
    engine: &PersistenceEngine,
    bound_mod_ids: &[String],
) -> Result<Option<ActiveModPromptContext>, String> {
    reverify_external_mods(engine, true)?;
    let bound_mod_ids = bound_mod_ids
        .iter()
        .map(|mod_id| clean_binding_id("mod_id", mod_id))
        .collect::<Result<HashSet<_>, _>>()?;
    let _guard = engine.lock_writes();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    ensure_schema(&connection)?;
    let active_prompt_mods = {
        let mut statement = connection
            .prepare(
                "
                SELECT installed_mods.id, installed_mods.name, installed_mods.default_system_prompt
                FROM installed_mods
                WHERE installed_mods.is_active=1
                  AND installed_mods.default_system_prompt IS NOT NULL
                  AND TRIM(installed_mods.default_system_prompt) <> ''
                ORDER BY installed_mods.name COLLATE NOCASE
                ",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?
    };
    if active_prompt_mods.is_empty() {
        return Ok(None);
    }

    let selected_mods = if bound_mod_ids.is_empty() {
        Vec::new()
    } else {
        active_prompt_mods
            .iter()
            .filter(|(mod_id, _, _)| bound_mod_ids.contains(mod_id))
            .cloned()
            .collect::<Vec<_>>()
    };
    if selected_mods.is_empty() {
        return Ok(None);
    }
    let configured_mods = selected_mods;
    let selection_mode = "agent_binding";
    let source_detail =
        "verified installed mod manifests with global active state enabled and explicit active-agent binding";
    let applied_mod_ids = configured_mods
        .iter()
        .map(|(mod_id, _, _)| mod_id.clone())
        .collect::<Vec<_>>();
    let prompt_blocks = configured_mods
        .into_iter()
        .filter_map(|(_, name, prompt)| {
            let prompt = prompt.trim();
            if prompt.is_empty() {
                None
            } else {
                Some(format!(
                    "Mod: {}\nRequired behavior:\n{}",
                    name.trim(),
                    prompt
                ))
            }
        })
        .collect::<Vec<_>>();

    if prompt_blocks.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ActiveModPromptContext {
            prompt: format!(
                "Active OOMU Mod Runtime Contract\nSource: {source_detail}.\nStatus: mandatory for this turn.\nApply every listed mod instruction to the next assistant response as active runtime behavior, not descriptive metadata. If a mod defines a visible response behavior, perform it naturally while preserving safety and the active agent persona.\n\nActive OOMU Mod Prompt Hooks\n{}",
                prompt_blocks.join("\n\n")
            ),
            applied_mod_ids,
            selection_mode,
        }))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveModManifestRecord {
    pub id: String,
    pub manifest_json: Value,
}

pub(crate) fn active_mod_manifest_records(
    engine: &PersistenceEngine,
) -> Result<Vec<ActiveModManifestRecord>, String> {
    reverify_external_mods(engine, true)?;
    let _guard = engine.lock_writes();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    ensure_schema(&connection)?;
    let mut statement = connection
        .prepare(
            "
            SELECT id, manifest_json
            FROM installed_mods
            WHERE is_active=1
            ORDER BY name COLLATE NOCASE
            ",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|(id, manifest_json)| {
            serde_json::from_str(&manifest_json)
                .map(|manifest_json| ActiveModManifestRecord { id, manifest_json })
                .map_err(|error| error.to_string())
        })
        .collect()
}

pub(crate) fn validate_active_mods_for_turn(
    engine: &PersistenceEngine,
    bound_mod_ids: &[String],
    explicit_mod_id: Option<&str>,
    message: &str,
    provider_class_id: &str,
    model_id: &str,
    user_locale: &str,
) -> Result<(), String> {
    let records = active_mod_manifest_records(engine)?;
    validate_mod_manifest_records_for_turn(
        &records,
        bound_mod_ids,
        explicit_mod_id,
        message,
        provider_class_id,
        model_id,
        user_locale,
    )
}

fn validate_mod_manifest_records_for_turn(
    records: &[ActiveModManifestRecord],
    bound_mod_ids: &[String],
    explicit_mod_id: Option<&str>,
    message: &str,
    provider_class_id: &str,
    model_id: &str,
    user_locale: &str,
) -> Result<(), String> {
    let mut target_mod_ids = bound_mod_ids
        .iter()
        .map(|mod_id| mod_id.trim().to_string())
        .filter(|mod_id| !mod_id.is_empty())
        .collect::<HashSet<_>>();
    if let Some(explicit_mod_id) = explicit_mod_id
        .map(str::trim)
        .filter(|mod_id| !mod_id.is_empty())
    {
        target_mod_ids.insert(explicit_mod_id.to_string());
    }
    if let Some(command_mod_id) = mod_id_for_slash_command(records, message)? {
        target_mod_ids.insert(command_mod_id);
    }
    if target_mod_ids.is_empty() {
        return Ok(());
    }

    for record in records {
        if !target_mod_ids.contains(&record.id) {
            continue;
        }
        let manifest = installed_mod_summary_from_manifest_record(record)?;
        validate_active_mod_compatibility(&manifest, provider_class_id, model_id, user_locale)?;
    }
    Ok(())
}

fn mod_id_for_slash_command(
    records: &[ActiveModManifestRecord],
    message: &str,
) -> Result<Option<String>, String> {
    let Some(trigger) = first_slash_trigger(message) else {
        return Ok(None);
    };
    for record in records {
        let commands = strict_manifest_commands(&record.manifest_json, &record.id)?;
        if commands.as_ref().is_some_and(|commands| {
            commands
                .iter()
                .any(|command| command_trigger_matches(&command.trigger, trigger))
        }) {
            return Ok(Some(record.id.clone()));
        }
    }
    Ok(None)
}

fn first_slash_trigger(message: &str) -> Option<&str> {
    let trimmed = message.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    trimmed.split_whitespace().next()
}

fn command_trigger_matches(manifest_trigger: &str, user_trigger: &str) -> bool {
    let manifest_trigger = manifest_trigger.trim();
    !manifest_trigger.is_empty() && manifest_trigger.eq_ignore_ascii_case(user_trigger.trim())
}

fn installed_mod_summary_from_manifest_record(
    record: &ActiveModManifestRecord,
) -> Result<InstalledMod, String> {
    let manifest = &record.manifest_json;
    let id = manifest
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&record.id);
    let name = manifest
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(id);
    Ok(InstalledMod {
        id: id.to_string(),
        name: name.to_string(),
        description: manifest
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        is_active: true,
        version: manifest
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        author: manifest
            .get("author")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        category: manifest
            .get("category")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        package_size: String::new(),
        last_updated: String::new(),
        review_state: "unreviewed".to_string(),
        publisher_identity_verified: false,
        integrity_state: "unsigned".to_string(),
        is_built_in: false,
        permissions: Vec::new(),
        endpoints: Vec::new(),
        agent_config_schema: manifest
            .get("agent_config_schema")
            .cloned()
            .filter(|schema| !schema.is_null()),
        commands: strict_manifest_commands(manifest, &record.id)?,
        requirements: strict_manifest_requirements(manifest, &record.id)?,
    })
}

fn strict_manifest_commands(
    manifest: &Value,
    mod_id: &str,
) -> Result<Option<Vec<ModCommand>>, String> {
    match manifest.get("commands") {
        Some(Value::Null) | None => Ok(None),
        Some(value) => serde_json::from_value::<Vec<ModCommand>>(value.clone())
            .map(|commands| (!commands.is_empty()).then_some(commands))
            .map_err(|error| format!("Active mod '{mod_id}' has invalid commands: {error}")),
    }
}

fn strict_manifest_requirements(
    manifest: &Value,
    mod_id: &str,
) -> Result<Option<ModRequirements>, String> {
    match manifest.get("requirements") {
        Some(Value::Null) | None => Ok(None),
        Some(value) => serde_json::from_value::<ModRequirements>(value.clone())
            .map(Some)
            .map_err(|error| format!("Active mod '{mod_id}' has invalid requirements: {error}")),
    }
}

pub fn validate_active_mod_compatibility(
    manifest: &InstalledMod,
    provider_class_id: &str,
    model_id: &str,
    user_locale: &str,
) -> Result<(), String> {
    let Some(reqs) = &manifest.requirements else {
        return Ok(());
    };
    let localized_error = |default_msg: String| -> String {
        localized_mod_requirement_error(&reqs.error_notice_override, user_locale, default_msg)
    };
    let provider_class = normalized_provider_class(provider_class_id);
    let is_local_provider = provider_class == "local_model";

    if is_local_provider {
        if let Some(allowed_local_models) = &reqs.supported_local_models {
            let is_allowed_local = allowed_local_models
                .iter()
                .any(|allowed| allowed.trim().eq_ignore_ascii_case(model_id.trim()));
            if !is_allowed_local {
                return Err(localized_error(format!(
                    "Mod {} is incompatible with local model {}.",
                    manifest.name, model_id
                )));
            }
            return Ok(());
        }
        if reqs.supported_provider_classes.is_some() {
            return Err(localized_error(format!(
                "Mod {} requires a cloud provider.",
                manifest.name
            )));
        }
    }

    if let Some(allowed_classes) = &reqs.supported_provider_classes {
        let is_allowed_provider = allowed_classes
            .iter()
            .any(|class| normalized_provider_class(class) == provider_class);
        if !is_allowed_provider {
            return Err(localized_error(format!(
                "Mod {} is incompatible with provider class {}.",
                manifest.name, provider_class_id
            )));
        }
    }

    Ok(())
}

fn localized_mod_requirement_error(
    error_map: &Option<HashMap<String, String>>,
    user_locale: &str,
    default_msg: String,
) -> String {
    error_map
        .as_ref()
        .and_then(|map| {
            map.get(user_locale)
                .or_else(|| user_locale.split_once('-').and_then(|(lang, _)| map.get(lang)))
                .or_else(|| map.get("en-US"))
        })
        .cloned()
        .unwrap_or_else(|| {
            if default_msg.trim().is_empty() {
                "This mod requires Google Gemini or Gemma 4 12B. Please switch models in the Tuning panel."
                    .to_string()
            } else {
                default_msg
            }
        })
}

fn normalized_provider_class(provider_class_id: &str) -> String {
    let normalized = provider_class_id
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_");
    match normalized.as_str() {
        "gemini" | "google_gemini" | "gemini_pro" | "gemini_flash" => "google".to_string(),
        "chatgpt" | "chat_gpt" => "openai".to_string(),
        "claude" => "anthropic".to_string(),
        "local" | "local_gemma" | "gemma" => "local_model".to_string(),
        _ => normalized,
    }
}

fn provider_class_for_route(
    agent_manager: &AgentManager,
    provider_id: &str,
) -> Result<String, String> {
    let route_provider_id = clean_route_text("provider_id", provider_id)?;
    if !route_provider_id.starts_with("prov-") {
        return Ok(route_provider_id);
    }

    let config = agent_manager
        .select_provider_config(&route_provider_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Provider configuration '{route_provider_id}' was not found."))?;
    clean_route_text("provider_id", &config.provider_id)
}

fn effective_model_id_for_validation(
    app: &tauri::AppHandle,
    provider_class_id: &str,
    model_id: &str,
) -> String {
    if normalized_provider_class(provider_class_id) != "local_model" {
        return model_id.trim().to_string();
    }
    settings::resolved_local_model_directory(app)
        .ok()
        .and_then(|model_root| resolve_local_model(&model_root, model_id).ok())
        .map(|resolved| resolved.id)
        .unwrap_or_else(|| model_id.trim().to_string())
}

fn install_mod(
    engine: &PersistenceEngine,
    package: VerifiedModPackage,
) -> Result<InstalledMod, String> {
    let _operation_guard = mod_trust::lock_mod_package_operation()?;
    let VerifiedModPackage { archive, metadata } = package;
    if archive.len() as u64 > MAX_MOD_ARCHIVE_SIZE {
        return Err(format!(
            "Mod package is too large. Maximum supported size is {}.",
            format_package_size(MAX_MOD_ARCHIVE_SIZE)
        ));
    }

    let entries = parse_mod_archive(&archive)?;
    let manifest_entry = entries
        .iter()
        .find(|entry| entry.name == "manifest.json")
        .ok_or_else(|| "Mod package is missing manifest.json.".to_string())?;
    let manifest_value: Value = serde_json::from_slice(&manifest_entry.bytes)
        .map_err(|error| format!("manifest.json is not valid JSON: {error}"))?;
    let manifest: ModManifest = serde_json::from_value(manifest_value.clone())
        .map_err(|error| format!("manifest.json does not match the OOMU mod schema: {error}"))?;
    validate_manifest(&manifest, &entries)?;
    let expected_trust = mod_trust::evaluate_package(&manifest_value, &entries)?;

    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    ensure_schema(&connection)?;
    ensure_no_case_colliding_mod_id(&connection, &manifest.id)?;

    let mods_root = mods_root()?;
    fs::create_dir_all(&mods_root).map_err(|error| {
        format!(
            "Unable to create OOMU mods directory at {}: {error}",
            mods_root.display()
        )
    })?;

    let storage_id = storage_id(&manifest.id);
    let final_dir = mods_root.join(&storage_id);
    let staging_dir = mods_root.join(format!(".installing-{storage_id}-{}", now_ms()));
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).map_err(|error| {
            format!(
                "Unable to reset staging directory {}: {error}",
                staging_dir.display()
            )
        })?;
    }
    fs::create_dir_all(&staging_dir).map_err(|error| {
        format!(
            "Unable to create staging directory {}: {error}",
            staging_dir.display()
        )
    })?;
    if let Err(error) = extract_entries_to(&entries, &staging_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }
    if final_dir.exists() {
        fs::remove_dir_all(&final_dir).map_err(|error| {
            format!(
                "Unable to replace installed mod directory {}: {error}",
                final_dir.display()
            )
        })?;
    }
    fs::rename(&staging_dir, &final_dir).map_err(|error| {
        let _ = fs::remove_dir_all(&staging_dir);
        format!(
            "Unable to activate installed mod directory {}: {error}",
            final_dir.display()
        )
    })?;
    let installed_evaluation = mod_trust::evaluate_installed_directory(&final_dir)?;
    if installed_evaluation.mod_id != manifest.id.trim()
        || installed_evaluation.version != manifest.version.trim()
        || installed_evaluation.trust.payload_sha256 != expected_trust.payload_sha256
    {
        let _ = fs::remove_dir_all(&final_dir);
        return Err("The installed mod did not match the reviewed package.".to_string());
    }
    let trust = installed_evaluation.trust;

    let default_permissions_value = Value::Null;
    let manifest_permissions_value = manifest_value
        .get("permissions")
        .unwrap_or(&default_permissions_value);
    let installed_mod = installed_mod_from_manifest(
        &manifest,
        &metadata,
        manifest_permissions_value,
        &trust,
        false,
    );
    upsert_installed_mod(
        engine,
        &installed_mod,
        &final_dir,
        &manifest_value,
        manifest.default_system_prompt.as_deref(),
        &manifest.entrypoint,
        &trust.payload_sha256,
    )?;
    Ok(installed_mod)
}

fn delete_installed_mod(engine: &PersistenceEngine, mod_id: &str) -> Result<(), String> {
    let _operation_guard = mod_trust::lock_mod_package_operation()?;
    let _guard = engine.lock_writes();
    let mut connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    ensure_schema(&connection)?;
    let installed: Option<(String, bool)> = connection
        .query_row(
            "SELECT installed_path,is_built_in FROM installed_mods WHERE id=?1",
            params![mod_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((installed_path, is_built_in)) = installed else {
        return Ok(());
    };
    if !is_built_in {
        remove_installed_directory(&installed_path)?;
    }
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    if is_built_in {
        transaction
            .execute(
                "
                INSERT INTO removed_built_in_mods (mod_id, removed_at_ms)
                VALUES (?1, ?2)
                ON CONFLICT(mod_id) DO UPDATE SET removed_at_ms=excluded.removed_at_ms
                ",
                params![mod_id, now_ms()],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction
        .execute(
            "DELETE FROM capability_bundle_records WHERE mod_id=?1",
            params![mod_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute("DELETE FROM installed_mods WHERE id=?1", params![mod_id])
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

pub(crate) fn ensure_installed_mod_exists(
    engine: &PersistenceEngine,
    mod_id: &str,
) -> Result<(), String> {
    let _guard = engine.lock_writes();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    ensure_schema(&connection)?;
    let mod_exists = connection
        .query_row(
            "SELECT 1 FROM installed_mods WHERE id=?1",
            params![mod_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_some();
    if !mod_exists {
        return Err(format!("Installed mod {mod_id} was not found."));
    }
    Ok(())
}

pub(super) fn upsert_installed_mod(
    engine: &PersistenceEngine,
    installed_mod: &InstalledMod,
    final_dir: &Path,
    manifest_value: &Value,
    default_system_prompt: Option<&str>,
    entrypoint: &str,
    payload_sha256: &str,
) -> Result<(), String> {
    let _guard = engine.lock_writes();
    let connection = engine
        .open_connection()
        .map_err(|error| error.to_string())?;
    ensure_schema(&connection)?;
    let permissions_json =
        serde_json::to_string(&installed_mod.permissions).map_err(|error| error.to_string())?;
    let endpoints_json =
        serde_json::to_string(&installed_mod.endpoints).map_err(|error| error.to_string())?;
    let manifest_json = serde_json::to_string(manifest_value).map_err(|error| error.to_string())?;
    let review_state = if exact_package_is_revoked(
        &connection,
        &installed_mod.id,
        &installed_mod.version,
        payload_sha256,
        manifest_value,
        &manifest_json,
    )? {
        "revoked"
    } else {
        installed_mod.review_state.as_str()
    };
    let timestamp = now_ms();
    connection
        .execute(
            "
            INSERT INTO installed_mods (
                id, name, description, is_active, version, author, category,
                package_size, last_updated, permissions_json, endpoints_json,
                installed_path, manifest_json, default_system_prompt, entrypoint,
                review_state, publisher_identity_verified, integrity_state,
                payload_sha256, is_built_in, installed_at_ms, updated_at_ms
            )
            VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                    ?15, ?16, ?17, ?18, ?19, ?20, ?20)
            ON CONFLICT(id) DO UPDATE SET
                name=excluded.name,
                description=excluded.description,
                is_active=0,
                version=excluded.version,
                author=excluded.author,
                category=excluded.category,
                package_size=excluded.package_size,
                last_updated=excluded.last_updated,
                permissions_json=excluded.permissions_json,
                endpoints_json=excluded.endpoints_json,
                installed_path=excluded.installed_path,
                manifest_json=excluded.manifest_json,
                default_system_prompt=excluded.default_system_prompt,
                entrypoint=excluded.entrypoint,
                review_state=excluded.review_state,
                publisher_identity_verified=excluded.publisher_identity_verified,
                integrity_state=excluded.integrity_state,
                payload_sha256=excluded.payload_sha256,
                is_built_in=excluded.is_built_in,
                updated_at_ms=excluded.updated_at_ms
            ",
            params![
                installed_mod.id,
                installed_mod.name,
                installed_mod.description,
                installed_mod.version,
                installed_mod.author,
                installed_mod.category,
                installed_mod.package_size,
                installed_mod.last_updated,
                permissions_json,
                endpoints_json,
                final_dir.to_string_lossy(),
                manifest_json,
                default_system_prompt,
                entrypoint,
                review_state,
                bool_to_db(installed_mod.publisher_identity_verified),
                installed_mod.integrity_state,
                payload_sha256,
                bool_to_db(installed_mod.is_built_in),
                timestamp,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn recover_installed_mod_from_directory(
    engine: &PersistenceEngine,
    mod_id: &str,
    final_dir: &Path,
) -> Result<bool, String> {
    if !final_dir.exists() {
        return Ok(false);
    }

    let manifest_path = final_dir.join("manifest.json");
    if !manifest_path.is_file() {
        return Ok(false);
    }

    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        format!(
            "Unable to read installed mod manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    let manifest_value: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("installed manifest.json is not valid JSON: {error}"))?;
    let manifest: ModManifest =
        serde_json::from_value(manifest_value.clone()).map_err(|error| {
            format!("installed manifest.json does not match the OOMU mod schema: {error}")
        })?;
    let manifest_id = manifest.id.trim();
    if manifest_id != mod_id {
        return Err(format!(
            "Installed mod directory for {mod_id} contains manifest id {manifest_id}."
        ));
    }

    let entrypoint = normalize_archive_name(&manifest.entrypoint)?;
    let entrypoint_path = final_dir.join(relative_archive_path(&entrypoint)?);
    if !entrypoint_path.is_file() {
        return Err(format!(
            "Installed mod {mod_id} is missing entrypoint {}.",
            manifest.entrypoint
        ));
    }

    let metadata = manifest_path.metadata().map_err(|error| {
        format!(
            "Unable to read installed mod manifest metadata {}: {error}",
            manifest_path.display()
        )
    })?;
    let default_permissions_value = Value::Null;
    let manifest_permissions_value = manifest_value
        .get("permissions")
        .unwrap_or(&default_permissions_value);
    let evaluation = mod_trust::evaluate_installed_directory(final_dir)?;
    if evaluation.mod_id != mod_id || evaluation.version != manifest.version.trim() {
        return Err("Installed mod identity changed during recovery.".to_string());
    }
    let trust = evaluation.trust;
    let installed_mod = installed_mod_from_manifest(
        &manifest,
        &metadata,
        manifest_permissions_value,
        &trust,
        false,
    );
    upsert_installed_mod(
        engine,
        &installed_mod,
        final_dir,
        &manifest_value,
        manifest.default_system_prompt.as_deref(),
        &manifest.entrypoint,
        &trust.payload_sha256,
    )?;
    Ok(true)
}

fn ensure_schema(connection: &rusqlite::Connection) -> Result<(), String> {
    connection
        .execute_batch(INSTALLED_MODS_SCHEMA)
        .map_err(|error| error.to_string())?;
    mod_trust::ensure_installed_mod_trust_columns(connection)?;
    connection
        .execute_batch(REMOVED_BUILT_IN_MODS_SCHEMA)
        .map_err(|error| error.to_string())?;
    retire_embedded_mods(connection)?;
    reconcile_installed_mod_branding(connection)
}

fn retire_embedded_mods(connection: &rusqlite::Connection) -> Result<(), String> {
    let [alignment_entrypoint, developer_entrypoint] = RETIRED_EMBEDDED_MOD_ENTRYPOINTS;
    connection
        .execute(
            "
            DELETE FROM capability_bundle_records
            WHERE mod_id IN (
                SELECT id
                FROM installed_mods
                WHERE is_built_in=1
                  AND (installed_path IN (?1, ?2) OR entrypoint IN (?1, ?2))
            )
            ",
            params![alignment_entrypoint, developer_entrypoint],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "
            DELETE FROM installed_mods
            WHERE is_built_in=1
              AND (installed_path IN (?1, ?2) OR entrypoint IN (?1, ?2))
            ",
            params![alignment_entrypoint, developer_entrypoint],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn reconcile_installed_mod_branding(connection: &rusqlite::Connection) -> Result<(), String> {
    let rows = {
        let mut statement = connection
            .prepare(
                "
                SELECT id, name, description, manifest_json, default_system_prompt
                FROM installed_mods WHERE is_built_in=1
                ",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        rows
    };

    let timestamp = now_ms();
    for (id, name, description, manifest_json, default_system_prompt) in rows {
        let aligned_name = aligned_mod_branding_text(&name);
        let aligned_description = aligned_mod_branding_text(&description);
        let aligned_manifest_json = aligned_mod_branding_text(&manifest_json);
        let aligned_default_system_prompt = default_system_prompt
            .as_deref()
            .map(aligned_mod_branding_text);

        if aligned_name == name
            && aligned_description == description
            && aligned_manifest_json == manifest_json
            && aligned_default_system_prompt == default_system_prompt
        {
            continue;
        }

        connection
            .execute(
                "
                UPDATE installed_mods
                SET name=?1,
                    description=?2,
                    manifest_json=?3,
                    default_system_prompt=?4,
                    updated_at_ms=?5
                WHERE id=?6
                ",
                params![
                    aligned_name,
                    aligned_description,
                    aligned_manifest_json,
                    aligned_default_system_prompt,
                    timestamp,
                    id
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn aligned_mod_branding_text(value: &str) -> String {
    let mut aligned = value.to_string();
    for (legacy, replacement) in [
        ("Sovereign Boost (DSpark)", "Performance Boost (DSpark)"),
        ("Sovereign Boost", "Performance Boost"),
        ("Sovereign Web Browser", "Web Browser Mod"),
        ("Sovereign intelligence", "integrated capability"),
        ("sovereign intelligence", "integrated capability"),
        ("Sovereign agent", "OOMU mod"),
        ("sovereign agent", "OOMU mod"),
        ("Sovereignty", "Capability Control"),
        ("sovereignty", "capability control"),
    ] {
        aligned = aligned.replace(legacy, replacement);
    }
    aligned
}

async fn ensure_agent_exists(
    agents: &tauri::State<'_, AgentManager>,
    agent_id: &str,
) -> Result<(), AgenticLoopError> {
    agents
        .inner()
        .clone()
        .get_agent_config(agent_id.to_string())
        .await
        .map_err(|error| mod_error("mod_agent_lookup_failed", error))?
        .map(|_| ())
        .ok_or_else(|| {
            mod_error(
                "mod_agent_not_found",
                format!("Agent {agent_id} was not found."),
            )
        })
}

fn clean_binding_id(label: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err(format!("Mod binding field `{label}` cannot be empty."))
    } else {
        Ok(value.to_string())
    }
}

fn clean_route_text(label: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err(format!("Runtime route field `{label}` cannot be empty."))
    } else {
        Ok(value.to_string())
    }
}

fn normalize_locale_id(locale: Option<&str>) -> String {
    locale
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("en-US")
        .to_string()
}

fn installed_mod_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledMod> {
    let permissions_json: String = row.get(9)?;
    let endpoints_json: String = row.get(10)?;
    let manifest_json: String = row.get(11)?;
    let permissions = parse_mod_json_column::<Vec<ModPermission>>(&permissions_json, 9)?;
    let endpoints = parse_mod_json_column::<Vec<String>>(&endpoints_json, 10)?;
    let manifest_value = parse_mod_json_column::<Value>(&manifest_json, 11)?;
    if !manifest_value.is_object() {
        return Err(mod_json_column_validation_error(
            11,
            "Stored mod manifest must be a JSON object.",
        ));
    }
    let agent_config_schema = manifest_value
        .get("agent_config_schema")
        .cloned()
        .filter(|schema| !schema.is_null());
    let commands = match manifest_value.get("commands") {
        Some(Value::Null) | None => None,
        Some(value) => Some(
            serde_json::from_value::<Vec<ModCommand>>(value.clone())
                .map_err(|error| mod_json_column_error(11, error))?,
        )
        .filter(|commands| !commands.is_empty()),
    };
    let requirements = match manifest_value.get("requirements") {
        Some(Value::Null) | None => None,
        Some(value) => Some(
            serde_json::from_value::<ModRequirements>(value.clone())
                .map_err(|error| mod_json_column_error(11, error))?,
        ),
    };

    Ok(InstalledMod {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        is_active: row.get::<_, i64>(3)? != 0,
        version: row.get(4)?,
        author: row.get(5)?,
        category: row.get(6)?,
        package_size: row.get(7)?,
        last_updated: row.get(8)?,
        review_state: row.get(12)?,
        publisher_identity_verified: row.get::<_, i64>(13)? != 0,
        integrity_state: row.get(14)?,
        is_built_in: row.get::<_, i64>(15)? != 0,
        permissions,
        endpoints,
        agent_config_schema,
        commands,
        requirements,
    })
}

fn parse_mod_json_column<T: serde::de::DeserializeOwned>(
    value: &str,
    index: usize,
) -> rusqlite::Result<T> {
    serde_json::from_str(value).map_err(|error| mod_json_column_error(index, error))
}

fn mod_json_column_error(index: usize, error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))
}

fn mod_json_column_validation_error(index: usize, message: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message.to_string(),
        )),
    )
}

fn installed_mod_from_manifest(
    manifest: &ModManifest,
    metadata: &fs::Metadata,
    permissions_value: &Value,
    trust: &ModTrust,
    is_built_in: bool,
) -> InstalledMod {
    InstalledMod {
        id: manifest.id.trim().to_string(),
        name: manifest.name.trim().to_string(),
        description: manifest.description.trim().to_string(),
        is_active: false,
        version: manifest.version.trim().to_string(),
        author: manifest.author.trim().to_string(),
        category: manifest
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| inferred_category(&manifest.hooks)),
        package_size: manifest
            .package_size
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format_package_size(metadata.len())),
        last_updated: manifest
            .last_updated
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format_last_updated(metadata.modified().ok())),
        review_state: trust.review_state.to_string(),
        publisher_identity_verified: trust.publisher_identity_verified,
        integrity_state: trust.integrity_state.to_string(),
        is_built_in,
        permissions: manifest_permissions(permissions_value),
        endpoints: manifest_endpoints(manifest),
        agent_config_schema: manifest.agent_config_schema.clone(),
        commands: manifest
            .commands
            .clone()
            .filter(|commands| !commands.is_empty()),
        requirements: manifest.requirements.clone(),
    }
}

fn deserialize_optional_mod_permissions<'de, D>(
    deserializer: D,
) -> Result<Option<ModPermissions>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Null => Ok(None),
        Value::Array(_) => Ok(None),
        value => serde_json::from_value(value)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

pub fn validate_mod_filesystem_access(
    mod_id: &str,
    target_path: &Path,
    permissions: &ModPermissions,
) -> Result<(), SecurityError> {
    let canonical_target = canonicalize_permission_path(target_path)?;

    if let Some(allowed_prefixes) = &permissions.allowed_paths {
        for prefix in allowed_prefixes {
            let prefix = prefix.trim();
            if prefix.is_empty() {
                continue;
            }
            let prefix_path = Path::new(prefix);
            if !prefix_path.is_absolute() {
                return Err(SecurityError::PathNormalizationFailed {
                    path: prefix.to_string(),
                    reason: "allowed path prefixes must be absolute".to_string(),
                });
            }
            let canonical_prefix = canonicalize_permission_path(prefix_path)?;
            if canonical_target.starts_with(canonical_prefix) {
                return Ok(());
            }
        }
    }

    Err(SecurityError::UnauthorizedAccess(format!(
        "Mod {mod_id} attempted unauthorized filesystem access to {}",
        target_path.display()
    )))
}

pub fn validate_mod_network_access(
    mod_id: &str,
    endpoint: &str,
    permissions: &ModPermissions,
) -> Result<(), SecurityError> {
    let requested_host = normalize_endpoint_host(endpoint)?;

    if let Some(allowed_hosts) = &permissions.allowed_hosts {
        for allowed_host in allowed_hosts {
            if allowed_host_matches(&requested_host, allowed_host)? {
                return Ok(());
            }
        }
    }

    Err(SecurityError::UnauthorizedAccess(format!(
        "Mod {mod_id} attempted unauthorized network access to {endpoint}"
    )))
}

fn allowed_host_matches(
    requested_host: &str,
    raw_allowed_host: &str,
) -> Result<bool, SecurityError> {
    let raw_allowed_host = raw_allowed_host.trim();
    if raw_allowed_host.is_empty() {
        return Ok(false);
    }
    if raw_allowed_host == "*" {
        return Ok(true);
    }
    if let Some(suffix) = raw_allowed_host.strip_prefix("*.") {
        let Some(suffix) = normalize_allowed_host(suffix)? else {
            return Ok(false);
        };
        return Ok(requested_host != suffix && requested_host.ends_with(&format!(".{suffix}")));
    }
    Ok(normalize_allowed_host(raw_allowed_host)?
        .is_some_and(|allowed_host| requested_host == allowed_host))
}

fn canonicalize_permission_path(path: &Path) -> Result<PathBuf, SecurityError> {
    fs::canonicalize(path).map_err(|error| SecurityError::PathNormalizationFailed {
        path: path.display().to_string(),
        reason: error.to_string(),
    })
}

fn normalize_endpoint_host(endpoint: &str) -> Result<String, SecurityError> {
    let endpoint = endpoint.trim();
    let parsed = reqwest::Url::parse(endpoint).map_err(|error| {
        SecurityError::EndpointNormalizationFailed {
            endpoint: endpoint.to_string(),
            reason: error.to_string(),
        }
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(SecurityError::EndpointNormalizationFailed {
            endpoint: endpoint.to_string(),
            reason: "network endpoints must use http or https".to_string(),
        });
    }
    parsed.host_str().map(normalize_host).ok_or_else(|| {
        SecurityError::EndpointNormalizationFailed {
            endpoint: endpoint.to_string(),
            reason: "network endpoint does not include a host".to_string(),
        }
    })
}

fn normalize_allowed_host(raw_host: &str) -> Result<Option<String>, SecurityError> {
    let raw_host = raw_host.trim();
    if raw_host.is_empty() {
        return Ok(None);
    }
    let parse_target = if raw_host.contains("://") {
        raw_host.to_string()
    } else {
        format!("https://{raw_host}")
    };
    let parsed = reqwest::Url::parse(&parse_target).map_err(|error| {
        SecurityError::EndpointNormalizationFailed {
            endpoint: raw_host.to_string(),
            reason: error.to_string(),
        }
    })?;
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(SecurityError::EndpointNormalizationFailed {
            endpoint: raw_host.to_string(),
            reason: "allowed_hosts entries must be hostnames or absolute URL origins".to_string(),
        });
    }
    parsed
        .host_str()
        .map(|host| Some(normalize_host(host)))
        .ok_or_else(|| SecurityError::EndpointNormalizationFailed {
            endpoint: raw_host.to_string(),
            reason: "allowed host entry does not include a host".to_string(),
        })
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn validate_manifest(manifest: &ModManifest, entries: &[ArchiveEntry]) -> Result<(), String> {
    for (label, value) in [
        ("id", manifest.id.as_str()),
        ("name", manifest.name.as_str()),
        ("version", manifest.version.as_str()),
        ("author", manifest.author.as_str()),
        ("description", manifest.description.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("manifest.json field `{label}` cannot be empty."));
        }
    }
    if !valid_mod_identifier(&manifest.id) {
        return Err("manifest.json field `id` must be a simple package identifier.".to_string());
    }
    if manifest.entrypoint.trim().is_empty() {
        return Err("manifest.json field `entrypoint` cannot be empty.".to_string());
    }
    let entrypoint = normalize_archive_name(&manifest.entrypoint)?;
    if !entries.iter().any(|entry| entry.name == entrypoint) {
        return Err(format!(
            "Mod entrypoint `{}` is missing from the package.",
            manifest.entrypoint
        ));
    }
    let declared_hosts = manifest_endpoints(manifest)
        .into_iter()
        .filter(|host| !host.eq_ignore_ascii_case("none declared"))
        .collect::<Vec<_>>();
    for command in manifest.commands.as_deref().unwrap_or_default() {
        if command.context_url_templates.len() > 3 {
            return Err(
                "Mod commands can declare at most three public context URL templates.".to_string(),
            );
        }
        if command.context_parameters.len() > 8 {
            return Err("Mod commands can declare at most eight context parameters.".to_string());
        }
        validate_required_context_evidence_patterns(&command.required_context_evidence_patterns)?;
        let parameter_name_pattern = Regex::new(r"^[a-z][a-z0-9_]{0,31}$")
            .expect("static context parameter name pattern is valid");
        for (name, parameter) in &command.context_parameters {
            if !parameter_name_pattern.is_match(name) {
                return Err(
                    "Mod context parameter names must use lowercase letters, numbers, and underscores."
                        .to_string(),
                );
            }
            if parameter.pattern.len() > 512 {
                return Err("Mod context parameter patterns are limited to 512 bytes.".to_string());
            }
            let pattern = Regex::new(&parameter.pattern)
                .map_err(|_| "Mod context parameter patterns must be valid regexes.".to_string())?;
            if pattern.captures_len() != 2 {
                return Err(
                    "Mod context parameter patterns must contain exactly one capture group."
                        .to_string(),
                );
            }
            if !matches!(
                parameter.transform.trim(),
                "" | "url_encode" | "uppercase" | "lowercase" | "date_iso"
            ) {
                return Err("Mod context parameter transforms are invalid.".to_string());
            }
        }
        let mut verification_replacements =
            HashMap::from([("query".to_string(), "verification".to_string())]);
        verification_replacements.extend(
            command
                .context_parameters
                .keys()
                .map(|name| (name.clone(), "verification".to_string())),
        );
        for template in &command.context_url_templates {
            let template = template.trim();
            if template.len() > 4_096 {
                return Err("Mod context URL templates are limited to 4096 bytes.".to_string());
            }
            let rendered = render_mod_context_template(template, &verification_replacements)
                .filter(|rendered| !rendered.contains('{') && !rendered.contains('}'))
                .ok_or_else(|| {
                    "Mod context URL templates may use only {query} or declared context parameters."
                        .to_string()
                })?;
            if rendered == template {
                return Err(
                    "Mod context URL templates must contain at least one placeholder.".to_string(),
                );
            }
            let parsed = url::Url::parse(&rendered)
                .map_err(|_| "Mod context URL templates must be absolute URLs.".to_string())?;
            if parsed.scheme() != "https"
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err(
                    "Mod context URL templates must resolve to credential-free HTTPS URLs."
                        .to_string(),
                );
            }
            let authority = AuthorizedNetworkModCommand {
                mod_id: manifest.id.clone(),
                search_query: String::new(),
                allowed_hosts: declared_hosts.clone(),
                context_urls: Vec::new(),
                required_context_evidence_patterns: Vec::new(),
            };
            if !authority.allows_url(parsed.as_str()) {
                return Err(
                    "Mod context URL templates must stay inside permissions.allowed_hosts."
                        .to_string(),
                );
            }
        }
    }
    Ok(())
}

fn validate_required_context_evidence_patterns(patterns: &[String]) -> Result<(), String> {
    if patterns.len() > 4 {
        return Err(
            "Mod commands can declare at most four required context evidence patterns.".to_string(),
        );
    }
    for pattern in patterns {
        let pattern = pattern.trim();
        if pattern.is_empty() || pattern.len() > 256 {
            return Err(
                "Required context evidence patterns must contain 1 to 256 bytes.".to_string(),
            );
        }
        let compiled = Regex::new(pattern).map_err(|_| {
            "Required context evidence patterns must be valid Rust regexes.".to_string()
        })?;
        if compiled.is_match("") {
            return Err(
                "Required context evidence patterns must not match empty content.".to_string(),
            );
        }
    }
    Ok(())
}

fn remove_installed_directory(raw_path: &str) -> Result<(), String> {
    let path = PathBuf::from(raw_path);
    if !path.exists() {
        return Ok(());
    }
    let root = fs::canonicalize(mods_root()?).map_err(|error| {
        format!("Unable to resolve OOMU mods directory before uninstall: {error}")
    })?;
    let real_path = fs::canonicalize(&path)
        .map_err(|error| format!("Unable to resolve installed mod directory: {error}"))?;
    if !real_path.starts_with(&root) {
        return Err("Installed mod path is outside the OOMU mods directory.".to_string());
    }
    fs::remove_dir_all(&real_path).map_err(|error| {
        format!(
            "Unable to remove installed mod directory {}: {error}",
            real_path.display()
        )
    })
}

fn manifest_endpoints(manifest: &ModManifest) -> Vec<String> {
    let endpoints = manifest
        .endpoints
        .iter()
        .map(|endpoint| endpoint.trim())
        .filter(|endpoint| !endpoint.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if !endpoints.is_empty() {
        return endpoints;
    }

    let Some(allowed_hosts) = manifest
        .permissions
        .as_ref()
        .and_then(|permissions| permissions.allowed_hosts.as_ref())
    else {
        return vec!["None declared".to_string()];
    };
    let endpoints = allowed_hosts
        .iter()
        .map(|host| host.trim())
        .filter(|host| !host.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if endpoints.is_empty() {
        vec!["None declared".to_string()]
    } else {
        endpoints
    }
}

fn installed_mod_directory(mod_id: &str) -> Result<PathBuf, String> {
    Ok(mods_root()?.join(storage_id(mod_id)))
}

fn mods_root() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|home| home.join(".oomu").join("mods"))
        .ok_or_else(|| "Unable to resolve the current user's home directory.".to_string())
}

fn format_package_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;
    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }
    if unit_index == 0 {
        format!("{} {}", bytes, UNITS[unit_index])
    } else {
        format!("{size:.1} {}", UNITS[unit_index])
    }
}

fn format_last_updated(modified: Option<SystemTime>) -> String {
    match modified {
        Some(modified) => {
            let updated_at: DateTime<Local> = DateTime::from(modified);
            updated_at.format("%B %-d, %Y").to_string()
        }
        None => "Unavailable".to_string(),
    }
}

fn bool_to_db(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn mod_error(code: &'static str, message: String) -> AgenticLoopError {
    AgenticLoopError {
        code,
        boundary: "ModsSubsystem",
        message,
        mlc_path: None,
    }
}

#[cfg(test)]
mod tests;
