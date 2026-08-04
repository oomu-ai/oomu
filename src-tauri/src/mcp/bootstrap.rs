use crate::mcp::client::McpServerConfig;
use crate::mcp::shield::McpTransportConfig;
use crate::settings;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::path::BaseDirectory;
use tauri::Manager;

const BUNDLED_MCP_RESOURCE_DIR: &str = "resources/mcp";
const BUNDLED_PYTHON_RESOURCE_DIR: &str = "resources/python";
const MCP_VENV_DIR: &str = "mcp_venv";
const MCP_SANDBOX_DIR: &str = "mcp_sandbox";
const MCP_SEARCH_PROFILE_DIR: &str = "mcp_search_profile";
const LOCAL_FILESYSTEM_SERVER: &str = "local_filesystem";
const TASKFLOW_NATIVE_SERVER: &str = "taskflow_native";
const LOCAL_SEARCH_SERVER: &str = "local_search";
const MACOS_APPLESCRIPT_SERVER: &str = "macos_applescript";
const LOCAL_SEARCH_SCRIPT: &str = "mcp_search.py";
const MACOS_APPLESCRIPT_SCRIPT: &str = "mcp_applescript.py";
static MCP_VENV_REBUILD_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct OptionalPythonRuntime {
    python_path: PathBuf,
    venv_root: PathBuf,
    created_venv: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBootstrapReport {
    pub python_path: Option<String>,
    pub resource_root: Option<String>,
    pub venv_root: Option<String>,
    pub created_venv: bool,
    pub optional_python_runtime_error: Option<String>,
    pub server_configs: Vec<McpServerConfig>,
}

pub(crate) fn record_mcp_runtime_health(
    degraded_mode: &crate::persistence_health::DegradedModeState,
    report: &McpBootstrapReport,
) {
    use crate::persistence_health::BackingStoreClass;

    if let Some(error) = report.optional_python_runtime_error.as_deref() {
        degraded_mode.activate(
            "mcpRuntime",
            format!("Optional MCP runtime bootstrap failed: {error}"),
            BackingStoreClass::NotApplicable,
            true,
            "Public search and Apple app automation are temporarily unavailable; native OOMU features remain usable.",
        );
    } else {
        degraded_mode.clear_after_verified_recovery(
            "mcpRuntime",
            BackingStoreClass::NotApplicable,
            "MCP resource discovery, runtime bootstrap, and trusted configuration registration succeeded.",
        );
    }
}

pub fn bootstrap_mcp_runtime(app: &tauri::AppHandle) -> Result<McpBootstrapReport, String> {
    super::client::install_connected_tool_catalog_port(app);
    let app_data_root = settings::app_data_root();
    fs::create_dir_all(&app_data_root)
        .map_err(|error| format!("Unable to create OOMU app data root: {error}"))?;

    let sandbox_root = mcp_sandbox_root();
    ensure_mcp_sandbox_dir(&sandbox_root)?;
    let mut server_configs = vec![
        native_filesystem_server_config(&sandbox_root),
        taskflow_native_server_config(&sandbox_root),
    ];
    let mut python_path = None;
    let mut venv_root = None;
    let mut created_venv = false;
    let mut resource_root_report = None;
    let mut optional_python_runtime_error = None;

    let bundled_python_root = resolve_bundled_python_root(app).ok();
    match prepare_optional_python_runtime(&app_data_root, bundled_python_root.as_deref()) {
        Ok(runtime) => match resolve_mcp_resource_root(app) {
            Ok(resource_root) => {
                let search_profile_root = mcp_search_profile_root();
                if let Err(error) = ensure_mcp_search_profile_dir(&search_profile_root) {
                    optional_python_runtime_error = Some(error);
                } else {
                    match optional_python_server_configs(
                        &runtime.python_path,
                        &resource_root,
                        &search_profile_root,
                    ) {
                        Ok(optional_configs) => {
                            server_configs.extend(optional_configs);
                            python_path = Some(runtime.python_path.display().to_string());
                            venv_root = Some(runtime.venv_root.display().to_string());
                            created_venv = runtime.created_venv;
                            resource_root_report = Some(resource_root.display().to_string());
                        }
                        Err(error) => optional_python_runtime_error = Some(error),
                    }
                }
            }
            Err(error) => optional_python_runtime_error = Some(error),
        },
        Err(error) => optional_python_runtime_error = Some(error),
    }

    Ok(McpBootstrapReport {
        python_path,
        resource_root: resource_root_report,
        venv_root,
        created_venv,
        optional_python_runtime_error,
        server_configs,
    })
}

#[tauri::command]
pub fn mcp_builtin_server_configs(app: tauri::AppHandle) -> Result<Vec<McpServerConfig>, String> {
    bootstrap_mcp_runtime(&app).map(|report| {
        report
            .server_configs
            .iter()
            .map(McpServerConfig::public_builtin_descriptor)
            .collect()
    })
}

pub fn mcp_builtin_server_configs_headless() -> Result<Vec<McpServerConfig>, String> {
    let app_data_root = settings::app_data_root();
    fs::create_dir_all(&app_data_root)
        .map_err(|error| format!("Unable to create OOMU app data root: {error}"))?;

    let sandbox_root = mcp_sandbox_root();
    ensure_mcp_sandbox_dir(&sandbox_root)?;
    let mut server_configs = vec![
        native_filesystem_server_config(&sandbox_root),
        taskflow_native_server_config(&sandbox_root),
    ];

    let bundled_python_root = resolve_bundled_python_root_headless().ok();
    if let Ok(runtime) =
        prepare_optional_python_runtime(&app_data_root, bundled_python_root.as_deref())
    {
        let Ok(resource_root) = resolve_mcp_resource_root_headless() else {
            return Ok(server_configs);
        };
        let search_profile_root = mcp_search_profile_root();
        if ensure_mcp_search_profile_dir(&search_profile_root).is_err() {
            return Ok(server_configs);
        }
        let Ok(optional_configs) = optional_python_server_configs(
            &runtime.python_path,
            &resource_root,
            &search_profile_root,
        ) else {
            return Ok(server_configs);
        };
        server_configs.extend(optional_configs);
    }

    Ok(server_configs)
}

/// Resolve an in-process MCP built-in without probing the optional Python runtime.
///
/// Native recovery paths use this before the complete headless catalog so a
/// filesystem or taskflow self-heal cannot be delayed by Python discovery.
pub fn headless_server_configs_for(server_name: &str) -> Result<Vec<McpServerConfig>, String> {
    let config = match server_name {
        LOCAL_FILESYSTEM_SERVER => {
            let sandbox_root = mcp_sandbox_root();
            ensure_mcp_sandbox_dir(&sandbox_root)?;
            native_filesystem_server_config(&sandbox_root)
        }
        TASKFLOW_NATIVE_SERVER => {
            let sandbox_root = mcp_sandbox_root();
            ensure_mcp_sandbox_dir(&sandbox_root)?;
            taskflow_native_server_config(&sandbox_root)
        }
        _ => return mcp_builtin_server_configs_headless(),
    };
    Ok(vec![config])
}

pub fn mcp_sandbox_root() -> PathBuf {
    settings::app_data_root().join(MCP_SANDBOX_DIR)
}

pub fn mcp_search_profile_root() -> PathBuf {
    settings::app_data_root().join(MCP_SEARCH_PROFILE_DIR)
}

pub fn ensure_default_mcp_sandbox_dir() -> Result<PathBuf, String> {
    let sandbox_root = mcp_sandbox_root();
    ensure_mcp_sandbox_dir(&sandbox_root)?;
    Ok(sandbox_root)
}

pub fn ensure_mcp_sandbox_dir(sandbox_root: &Path) -> Result<(), String> {
    fs::create_dir_all(sandbox_root).map_err(|error| {
        format!(
            "Unable to create MCP sandbox directory at {}: {error}",
            sandbox_root.display()
        )
    })
}

fn ensure_mcp_search_profile_dir(profile_root: &Path) -> Result<(), String> {
    for directory in [
        profile_root.to_path_buf(),
        profile_root.join("cache"),
        profile_root.join("config"),
        profile_root.join("tmp"),
    ] {
        fs::create_dir_all(&directory).map_err(|error| {
            format!(
                "Unable to create isolated MCP search profile directory at {}: {error}",
                directory.display()
            )
        })?;
    }
    Ok(())
}

fn verify_system_python3(preferred_bundled_root: Option<&Path>) -> Result<String, String> {
    let mut bundled_errors = Vec::new();
    if let Some(root) = preferred_bundled_root {
        let candidate = bundled_python_executable_path(root);
        match verify_python_candidate(&candidate, "bundled MCP Python") {
            Ok(command) => return Ok(command),
            Err(error) => bundled_errors.push(error),
        }
    }

    if preferred_bundled_root.is_none() {
        if let Ok(root) = resolve_bundled_python_root_headless() {
            let candidate = bundled_python_executable_path(&root);
            match verify_python_candidate(&candidate, "bundled MCP Python") {
                Ok(command) => return Ok(command),
                Err(error) => bundled_errors.push(error),
            }
        }
    }

    let command = std::env::var_os("OOMU_SYSTEM_PYTHON")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("python3"));
    verify_python_candidate(&command, "system python3").map_err(|error| {
        if bundled_errors.is_empty() {
            error
        } else {
            format!(
                "{}; bundled fallback attempts: {}",
                error,
                bundled_errors.join(" | ")
            )
        }
    })
}

pub(crate) fn resolve_system_python3_headless() -> Result<String, String> {
    verify_system_python3(None)
}

fn verify_python_candidate(command: &Path, label: &str) -> Result<String, String> {
    let output = isolated_python_command(command)
        .arg("--version")
        .output()
        .map_err(|error| {
            format!(
                "Unable to run {label} for MCP runtime bootstrap at {}: {error}",
                command.display()
            )
        })?;

    if output.status.success() {
        Ok(command.display().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let message = if stderr.is_empty() { stdout } else { stderr };
        Err(format!(
            "{label} failed version check for MCP runtime bootstrap at {}: {message}",
            command.display()
        ))
    }
}

fn ensure_python_runtime(system_python: &str, venv_root: &Path) -> Result<bool, String> {
    let python_path = venv_python_path(venv_root);
    if python_runtime_matches_base(Path::new(system_python), &python_path) {
        return Ok(false);
    }

    if let Some(parent) = venv_root.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to create MCP runtime parent directory: {error}"))?;
    }

    let staged_root = python_runtime_sidecar_path(venv_root, "replacement");
    let previous_root = python_runtime_sidecar_path(venv_root, "previous");
    create_python_runtime(system_python, &staged_root)?;
    if !python_runtime_matches_base(Path::new(system_python), &venv_python_path(&staged_root)) {
        let _ = remove_python_runtime_path(&staged_root);
        return Err("New isolated MCP Python runtime failed interpreter verification.".to_string());
    }

    let had_previous = fs::symlink_metadata(venv_root).is_ok();
    if had_previous {
        fs::rename(venv_root, &previous_root).map_err(|error| {
            let _ = remove_python_runtime_path(&staged_root);
            format!("Unable to preserve the stale MCP Python runtime for repair: {error}")
        })?;
    }
    if let Err(error) = fs::rename(&staged_root, venv_root) {
        let restored = !had_previous || fs::rename(&previous_root, venv_root).is_ok();
        let _ = remove_python_runtime_path(&staged_root);
        return Err(format!(
            "Unable to install the repaired MCP Python runtime: {error}; previous runtime restored={restored}"
        ));
    }
    if !python_runtime_matches_base(Path::new(system_python), &python_path) {
        let _ = remove_python_runtime_path(venv_root);
        let restored = had_previous && fs::rename(&previous_root, venv_root).is_ok();
        return Err(format!(
            "Repaired MCP Python runtime failed final verification; previous runtime restored={restored}"
        ));
    }
    if had_previous && remove_python_runtime_path(&previous_root).is_err() {
        eprintln!("MCP_PYTHON_STALE_RUNTIME_CLEANUP_FAILED");
    }

    Ok(true)
}

fn create_python_runtime(system_python: &str, venv_root: &Path) -> Result<(), String> {
    if fs::symlink_metadata(venv_root).is_ok() {
        remove_python_runtime_path(venv_root)
            .map_err(|error| format!("Unable to clear staged MCP Python runtime: {error}"))?;
    }

    let output = isolated_python_command(Path::new(system_python))
        .args(["-m", "venv", "--without-pip"])
        .arg(venv_root)
        .output()
        .map_err(|error| format!("Unable to create isolated MCP Python runtime: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(format!(
            "Unable to create isolated MCP Python runtime at {}: {detail}",
            venv_root.display()
        ));
    }
    Ok(())
}

fn python_runtime_matches_base(system_python: &Path, runtime_python: &Path) -> bool {
    python_base_identity(system_python)
        .ok()
        .zip(python_base_identity(runtime_python).ok())
        .is_some_and(|(system, runtime)| system == runtime)
}

fn python_base_identity(python_path: &Path) -> Result<PathBuf, String> {
    let output = isolated_python_command(python_path)
        .args([
            "-c",
            "import os, sys; print(os.path.realpath(sys._base_executable))",
        ])
        .output()
        .map_err(|error| format!("Unable to identify MCP Python interpreter: {error}"))?;
    if !output.status.success() {
        return Err("MCP Python interpreter identity check failed.".to_string());
    }
    let identity = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if identity.is_empty() {
        return Err("MCP Python interpreter identity was empty.".to_string());
    }
    let path = PathBuf::from(identity);
    Ok(fs::canonicalize(&path).unwrap_or(path))
}

fn python_runtime_sidecar_path(venv_root: &Path, label: &str) -> PathBuf {
    let sequence = MCP_VENV_REBUILD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = venv_root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(MCP_VENV_DIR);
    venv_root.with_file_name(format!(
        ".{name}.{label}.{}.{}",
        std::process::id(),
        sequence
    ))
}

fn remove_python_runtime_path(path: &Path) -> std::io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)
    } else {
        fs::remove_dir_all(path)
    }
}

fn verify_python_executable(python_path: &Path) -> Result<(), String> {
    let output = isolated_python_command(python_path)
        .arg("--version")
        .output()
        .map_err(|error| {
            format!(
                "Unable to run isolated MCP Python at {}: {error}",
                python_path.display()
            )
        })?;
    if output.status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!(
            "Isolated MCP Python failed version check at {}: {message}",
            python_path.display()
        ))
    }
}

fn isolated_python_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.env("PYTHONDONTWRITEBYTECODE", "1");
    command
}

fn bundled_python_executable_path(root: &Path) -> PathBuf {
    if cfg!(windows) {
        root.join("Scripts").join("python.exe")
    } else {
        root.join("bin").join("python3.10")
    }
}

fn resolve_bundled_python_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let candidates = [BUNDLED_PYTHON_RESOURCE_DIR, "python"];
    for candidate in candidates {
        if let Ok(path) = app.path().resolve(candidate, BaseDirectory::Resource) {
            if bundled_python_executable_path(&path).is_file() {
                return Ok(path);
            }
        }
    }

    let dev_path =
        PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join(BUNDLED_PYTHON_RESOURCE_DIR);
    if bundled_python_executable_path(&dev_path).is_file() {
        return Ok(dev_path);
    }

    Err(format!(
        "Unable to resolve bundled Python resources. Expected {} inside the Tauri resource scope.",
        BUNDLED_PYTHON_RESOURCE_DIR
    ))
}

fn resolve_bundled_python_root_headless() -> Result<PathBuf, String> {
    let manifest_path =
        PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join(BUNDLED_PYTHON_RESOURCE_DIR);
    if bundled_python_executable_path(&manifest_path).is_file() {
        return Ok(manifest_path);
    }

    let install_root = settings::install_root();
    let candidates = [
        install_root.join(BUNDLED_PYTHON_RESOURCE_DIR),
        install_root
            .join("Contents")
            .join("Resources")
            .join(BUNDLED_PYTHON_RESOURCE_DIR),
        install_root.join("python"),
        install_root
            .join("Contents")
            .join("Resources")
            .join("python"),
    ];
    for candidate in candidates {
        if bundled_python_executable_path(&candidate).is_file() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "Unable to resolve bundled Python resources without an app handle. Expected {} under the manifest or install resource root.",
        BUNDLED_PYTHON_RESOURCE_DIR
    ))
}

fn resolve_mcp_resource_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let candidates = [BUNDLED_MCP_RESOURCE_DIR, "mcp"];
    for candidate in candidates {
        if let Ok(path) = app.path().resolve(candidate, BaseDirectory::Resource) {
            if path.is_dir() {
                return Ok(path);
            }
        }
    }

    let dev_path =
        PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join(BUNDLED_MCP_RESOURCE_DIR);
    if dev_path.is_dir() {
        return Ok(dev_path);
    }

    Err(format!(
        "Unable to resolve bundled MCP resources. Expected {} inside the Tauri resource scope.",
        BUNDLED_MCP_RESOURCE_DIR
    ))
}

fn resolve_mcp_resource_root_headless() -> Result<PathBuf, String> {
    let manifest_path =
        PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join(BUNDLED_MCP_RESOURCE_DIR);
    if manifest_path.is_dir() {
        return Ok(manifest_path);
    }

    let install_root = settings::install_root();
    let candidates = [
        install_root.join(BUNDLED_MCP_RESOURCE_DIR),
        install_root
            .join("Contents")
            .join("Resources")
            .join(BUNDLED_MCP_RESOURCE_DIR),
        install_root.join("mcp"),
        install_root.join("Contents").join("Resources").join("mcp"),
    ];
    for candidate in candidates {
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "Unable to resolve bundled MCP resources without an app handle. Expected {} under the manifest or install resource root.",
        BUNDLED_MCP_RESOURCE_DIR
    ))
}

fn native_filesystem_server_config(sandbox_root: &Path) -> McpServerConfig {
    McpServerConfig {
        name: LOCAL_FILESYSTEM_SERVER.to_string(),
        command: "oomu-native".to_string(),
        args: Vec::new(),
        env: HashMap::from([(
            "OOMU_MCP_SANDBOX_DIR".to_string(),
            sandbox_root.display().to_string(),
        )]),
        transport: McpTransportConfig::Native,
    }
}

// taskflow_native is a real in-process native server (see mcp::taskflow). It is
// always available with no Python runtime, so it ships alongside the local
// filesystem server. Without this registration the workflow runtime cannot
// resolve the taskflow_native capabilities the compiler advertises and preflight
// fails with "is not registered and is not a built-in MCP server."
fn taskflow_native_server_config(sandbox_root: &Path) -> McpServerConfig {
    McpServerConfig {
        name: TASKFLOW_NATIVE_SERVER.to_string(),
        command: "oomu-native".to_string(),
        args: Vec::new(),
        env: HashMap::from([(
            "OOMU_MCP_SANDBOX_DIR".to_string(),
            sandbox_root.display().to_string(),
        )]),
        transport: McpTransportConfig::Native,
    }
}

fn optional_python_server_configs(
    python_path: &Path,
    resource_root: &Path,
    search_profile_root: &Path,
) -> Result<Vec<McpServerConfig>, String> {
    Ok(vec![
        bundled_server_config(
            LOCAL_SEARCH_SERVER,
            python_path,
            resource_root.join(LOCAL_SEARCH_SCRIPT),
            strict_search_env(search_profile_root),
        )?,
        bundled_server_config(
            MACOS_APPLESCRIPT_SERVER,
            python_path,
            resource_root.join(MACOS_APPLESCRIPT_SCRIPT),
            Vec::new(),
        )?,
    ])
}

fn prepare_optional_python_runtime(
    app_data_root: &Path,
    bundled_python_root: Option<&Path>,
) -> Result<OptionalPythonRuntime, String> {
    let system_python = verify_system_python3(bundled_python_root)?;
    let venv_root = app_data_root.join(MCP_VENV_DIR);
    let created_venv = ensure_python_runtime(&system_python, &venv_root)?;
    let python_path = venv_python_path(&venv_root);
    verify_python_executable(&python_path)?;

    Ok(OptionalPythonRuntime {
        python_path,
        venv_root,
        created_venv,
    })
}

fn strict_search_env(search_profile_root: &Path) -> Vec<(String, String)> {
    let profile = search_profile_root.display().to_string();
    vec![
        ("OOMU_MCP_ENV_ISOLATION".to_string(), "strict".to_string()),
        ("OOMU_MCP_SEARCH_PROFILE_DIR".to_string(), profile.clone()),
        ("HOME".to_string(), profile.clone()),
        ("USERPROFILE".to_string(), profile.clone()),
        (
            "XDG_CONFIG_HOME".to_string(),
            search_profile_root.join("config").display().to_string(),
        ),
        (
            "XDG_CACHE_HOME".to_string(),
            search_profile_root.join("cache").display().to_string(),
        ),
        (
            "TMPDIR".to_string(),
            search_profile_root.join("tmp").display().to_string(),
        ),
        ("LANG".to_string(), "C.UTF-8".to_string()),
        (
            "PATH".to_string(),
            "/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
        ),
    ]
}

fn bundled_server_config(
    name: &str,
    python_path: &Path,
    script_path: PathBuf,
    extra_env: Vec<(String, String)>,
) -> Result<McpServerConfig, String> {
    if !script_path.is_file() {
        return Err(format!(
            "Bundled MCP server '{}' is missing at {}.",
            name,
            script_path.display()
        ));
    }

    let mut env = HashMap::from([
        ("PYTHONDONTWRITEBYTECODE".to_string(), "1".to_string()),
        ("PYTHONNOUSERSITE".to_string(), "1".to_string()),
        ("PYTHONUTF8".to_string(), "1".to_string()),
    ]);
    for (key, value) in extra_env {
        env.insert(key, value);
    }

    Ok(McpServerConfig {
        name: name.to_string(),
        command: python_path.display().to_string(),
        args: vec![script_path.display().to_string()],
        env,
        transport: McpTransportConfig::Stdio,
    })
}

fn venv_python_path(venv_root: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_root.join("Scripts").join("python.exe")
    } else {
        venv_root.join("bin").join("python3")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::clock::unix_time_ms_u128 as unix_time_ms;

    #[test]
    fn bundled_configs_use_native_filesystem_and_optional_python_scripts() {
        let manifest_root = PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR);
        let resource_root = manifest_root.join(BUNDLED_MCP_RESOURCE_DIR);
        let python_path = PathBuf::from("/tmp/oomu-test-venv/bin/python3");
        let sandbox_root = PathBuf::from("/tmp/oomu-test-sandbox");
        let search_profile_root = PathBuf::from("/tmp/oomu-test-search-profile");

        let mut configs = vec![native_filesystem_server_config(&sandbox_root)];
        configs.extend(
            optional_python_server_configs(&python_path, &resource_root, &search_profile_root)
                .expect("bundled resource configs build"),
        );
        assert_eq!(configs.len(), 3);
        let filesystem = configs
            .iter()
            .find(|config| config.name == LOCAL_FILESYSTEM_SERVER)
            .expect("filesystem config exists");
        assert_eq!(filesystem.command, "oomu-native");
        assert!(filesystem.args.is_empty());
        assert_eq!(filesystem.transport, McpTransportConfig::Native);
        assert_eq!(
            filesystem
                .env
                .get("OOMU_MCP_SANDBOX_DIR")
                .map(String::as_str),
            Some("/tmp/oomu-test-sandbox")
        );
        let search = configs
            .iter()
            .find(|config| config.name == LOCAL_SEARCH_SERVER)
            .expect("local search config exists");
        assert_eq!(search.command, "/tmp/oomu-test-venv/bin/python3");
        assert_eq!(
            search.env.get("OOMU_MCP_ENV_ISOLATION").map(String::as_str),
            Some("strict")
        );
        assert_eq!(
            search
                .env
                .get("OOMU_MCP_SEARCH_PROFILE_DIR")
                .map(String::as_str),
            Some("/tmp/oomu-test-search-profile")
        );
        assert_eq!(
            search.env.get("HOME").map(String::as_str),
            Some("/tmp/oomu-test-search-profile")
        );
        assert!(search
            .args
            .first()
            .is_some_and(|arg| arg.ends_with("mcp_search.py")));
        let applescript = configs
            .iter()
            .find(|config| config.name == MACOS_APPLESCRIPT_SERVER)
            .expect("AppleScript config exists");
        assert_eq!(applescript.command, "/tmp/oomu-test-venv/bin/python3");
        assert!(applescript
            .args
            .first()
            .is_some_and(|arg| arg.ends_with("mcp_applescript.py")));
    }

    #[test]
    fn native_filesystem_config_does_not_require_python_runtime() {
        let sandbox_root = PathBuf::from("/tmp/oomu-test-sandbox");
        let config = native_filesystem_server_config(&sandbox_root);

        assert_eq!(config.name, LOCAL_FILESYSTEM_SERVER);
        assert_eq!(config.command, "oomu-native");
        assert!(config.args.is_empty());
        assert_eq!(config.transport, McpTransportConfig::Native);
        assert_eq!(
            config.env.get("OOMU_MCP_SANDBOX_DIR").map(String::as_str),
            Some("/tmp/oomu-test-sandbox")
        );
    }

    #[test]
    fn taskflow_native_config_is_a_native_builtin() {
        let sandbox_root = PathBuf::from("/tmp/oomu-test-sandbox");
        let config = taskflow_native_server_config(&sandbox_root);

        assert_eq!(config.name, TASKFLOW_NATIVE_SERVER);
        assert_eq!(config.command, "oomu-native");
        assert!(config.args.is_empty());
        assert_eq!(config.transport, McpTransportConfig::Native);
        assert_eq!(
            config.env.get("OOMU_MCP_SANDBOX_DIR").map(String::as_str),
            Some("/tmp/oomu-test-sandbox")
        );
    }

    #[test]
    fn creates_an_empty_mcp_sandbox_without_fixture_inputs() {
        let root = std::env::temp_dir().join(format!(
            "oomu-mcp-sandbox-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let sandbox_root = root.join(MCP_SANDBOX_DIR);

        ensure_mcp_sandbox_dir(&sandbox_root).expect("sandbox directory is created");
        assert!(sandbox_root.is_dir());
        assert_eq!(fs::read_dir(&sandbox_root).unwrap().count(), 0);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn creates_isolated_python_runtime_once() {
        let system_python = match verify_system_python3(None) {
            Ok(system_python) => system_python,
            Err(_) => return,
        };
        let root = std::env::temp_dir().join(format!(
            "oomu-mcp-bootstrap-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let venv_root = root.join(MCP_VENV_DIR);

        let created = ensure_python_runtime(&system_python, &venv_root)
            .expect("isolated Python runtime is created");
        assert!(created);
        assert!(venv_python_path(&venv_root).is_file());

        let created_again = ensure_python_runtime(&system_python, &venv_root)
            .expect("existing isolated Python runtime is reused");
        assert!(!created_again);

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn repairs_relocated_python_runtime_and_restores_full_optional_catalog() {
        use crate::mcp::client::McpClientRegistry;
        use std::os::unix::fs::symlink;
        use std::time::Duration;
        use tokio::time::timeout;

        let system_python = match verify_system_python3(None) {
            Ok(system_python) => system_python,
            Err(_) => return,
        };
        let root = std::env::temp_dir().join(format!(
            "oomu-mcp-relocation-{}-{}",
            std::process::id(),
            unix_time_ms()
        ));
        let venv_root = root.join(MCP_VENV_DIR);
        ensure_python_runtime(&system_python, &venv_root)
            .expect("initial isolated Python runtime is created");

        let python_path = venv_python_path(&venv_root);
        fs::remove_file(&python_path).expect("managed interpreter link is removable");
        symlink(root.join("removed-development-app/python3"), &python_path)
            .expect("stale relocated interpreter link is created");
        assert!(!python_runtime_matches_base(
            Path::new(&system_python),
            &python_path
        ));

        let runtime =
            prepare_optional_python_runtime(&root, None).expect("relocated runtime self-heals");
        assert!(runtime.created_venv);
        assert!(python_runtime_matches_base(
            Path::new(&system_python),
            &runtime.python_path
        ));

        let resource_root =
            PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join(BUNDLED_MCP_RESOURCE_DIR);
        let search_profile_root = root.join(MCP_SEARCH_PROFILE_DIR);
        ensure_mcp_search_profile_dir(&search_profile_root)
            .expect("isolated search profile is created");
        let configs = optional_python_server_configs(
            &runtime.python_path,
            &resource_root,
            &search_profile_root,
        )
        .expect("optional connector configurations are restored");
        assert!(configs
            .iter()
            .any(|config| config.name == LOCAL_SEARCH_SERVER));
        assert!(configs
            .iter()
            .any(|config| config.name == MACOS_APPLESCRIPT_SERVER));

        let registry = McpClientRegistry::default();
        assert_eq!(
            registry.register_trusted_server_configs(configs).await,
            2,
            "both Python-backed built-ins are registered"
        );
        for server_name in [LOCAL_SEARCH_SERVER, MACOS_APPLESCRIPT_SERVER] {
            timeout(
                Duration::from_secs(5),
                registry.ensure_server_connected(server_name),
            )
            .await
            .expect("repaired built-in connects before timeout")
            .expect("repaired built-in connects");
        }
        let mut search_tools = registry
            .list_tools(LOCAL_SEARCH_SERVER)
            .await
            .expect("public search catalog is available")
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        search_tools.sort();
        assert_eq!(search_tools, ["search_web"]);

        let mut apple_tools = registry
            .list_tools(MACOS_APPLESCRIPT_SERVER)
            .await
            .expect("Apple capability catalog is available")
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        apple_tools.sort();
        let mut expected_apple_tools = [
            "trigger_system_notification",
            "read_system_calendar",
            "add_system_reminder",
            "draft_system_email",
            "prepare_system_message",
            "capture_disposable_window",
            "preview_camera",
            "send_system_email",
            "create_system_note",
            "read_system_emails",
            "read_system_notes",
            "read_system_contacts",
            "read_system_music",
            "read_system_photos",
            "read_system_reminders",
            "read_apple_app_ui",
        ];
        expected_apple_tools.sort();
        assert_eq!(apple_tools, expected_apple_tools);

        registry
            .shutdown_all()
            .await
            .expect("repaired connector processes shut down cleanly");

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn every_bootstrap_python_command_disables_bundle_bytecode_writes() {
        let output = isolated_python_command(Path::new("/usr/bin/env"))
            .output()
            .expect("environment probe runs");
        let environment = String::from_utf8(output.stdout).expect("environment is UTF-8");
        assert!(environment
            .lines()
            .any(|line| line == "PYTHONDONTWRITEBYTECODE=1"));
    }
}
