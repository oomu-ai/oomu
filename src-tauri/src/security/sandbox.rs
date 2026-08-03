use crate::foundation::clock::unix_time_ns_u128;
use serde::Serialize;
use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const DOCKER_SANDBOX_IMAGE: &str = "oomu-sandbox:latest";
const DOCKER_WORKSPACE_ROOT: &str = "/sandbox";
const DRIVER_PROBE_TIMEOUT_MS: u64 = 800;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxEngineKind {
    Docker,
    MacosSandboxExec,
}

impl SandboxEngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::MacosSandboxExec => "macos_sandbox_exec",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxCommandKind {
    Shell,
    Python,
    Binary,
}

#[derive(Debug, Clone)]
pub struct SandboxCommandRequest {
    pub kind: SandboxCommandKind,
    pub command: String,
    pub args: Vec<String>,
    pub native_executable: Option<PathBuf>,
    pub workspace_root: PathBuf,
    pub working_directory: PathBuf,
    pub network_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxExecutionMetadata {
    pub engine: SandboxEngineKind,
    pub network_enabled: bool,
    pub workspace_root: String,
    pub working_directory: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_workspace_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_path: Option<String>,
}

pub struct SandboxCommandLaunch {
    pub process: Command,
    pub metadata: SandboxExecutionMetadata,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxDriverStatus {
    pub engine: SandboxEngineKind,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxStatus {
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_engine: Option<SandboxEngineKind>,
    pub docker: SandboxDriverStatus,
    pub macos_sandbox_exec: SandboxDriverStatus,
    pub docker_image: String,
    pub default_network_enabled: bool,
}

pub trait SandboxEngine {
    fn kind(&self) -> SandboxEngineKind;
    fn status(&self) -> SandboxDriverStatus;
    fn build_command(
        &self,
        request: &SandboxCommandRequest,
    ) -> Result<SandboxCommandLaunch, String>;
}

#[tauri::command]
pub fn get_sandbox_status() -> SandboxStatus {
    sandbox_status()
}

pub fn sandbox_status() -> SandboxStatus {
    let docker = DockerSandboxEngine.status();
    let macos_sandbox_exec = MacosSandboxExecEngine.status();
    let active_engine = if docker.available {
        Some(SandboxEngineKind::Docker)
    } else if macos_sandbox_exec.available {
        Some(SandboxEngineKind::MacosSandboxExec)
    } else {
        None
    };

    SandboxStatus {
        supported: active_engine.is_some(),
        active_engine,
        docker,
        macos_sandbox_exec,
        docker_image: DOCKER_SANDBOX_IMAGE.to_string(),
        default_network_enabled: false,
    }
}

pub fn build_sandboxed_command(
    request: SandboxCommandRequest,
) -> Result<SandboxCommandLaunch, String> {
    let docker = DockerSandboxEngine;
    let docker_status = docker.status();
    if docker_status.available {
        return docker.build_command(&request);
    }

    let macos = MacosSandboxExecEngine;
    let macos_status = macos.status();
    if macos_status.available {
        return macos.build_command(&request);
    }

    Err(format!(
        "No local code sandbox engine is available. Docker: {}; macOS sandbox-exec: {}.",
        docker_status
            .reason
            .unwrap_or_else(|| "unavailable".to_string()),
        macos_status
            .reason
            .unwrap_or_else(|| "unavailable".to_string())
    ))
}

struct DockerSandboxEngine;

impl SandboxEngine for DockerSandboxEngine {
    fn kind(&self) -> SandboxEngineKind {
        SandboxEngineKind::Docker
    }

    fn status(&self) -> SandboxDriverStatus {
        if executable_in_path("docker").is_none() {
            return SandboxDriverStatus {
                engine: self.kind(),
                available: false,
                reason: Some("docker executable was not found on PATH".to_string()),
            };
        }

        if !command_succeeds_with_timeout(
            "docker",
            &["image", "inspect", DOCKER_SANDBOX_IMAGE],
            DRIVER_PROBE_TIMEOUT_MS,
        ) {
            return SandboxDriverStatus {
                engine: self.kind(),
                available: false,
                reason: Some(format!(
                    "docker is installed, but image {DOCKER_SANDBOX_IMAGE} is not available"
                )),
            };
        }

        SandboxDriverStatus {
            engine: self.kind(),
            available: true,
            reason: None,
        }
    }

    fn build_command(
        &self,
        request: &SandboxCommandRequest,
    ) -> Result<SandboxCommandLaunch, String> {
        let workspace_root = canonicalize_existing_dir(&request.workspace_root)?;
        let working_directory = canonicalize_existing_dir(&request.working_directory)?;
        ensure_path_inside(&working_directory, &workspace_root)?;
        let container_workdir = container_path_for_host_path(&working_directory, &workspace_root)?;

        let mut process = Command::new("docker");
        process
            .arg("run")
            .arg("--rm")
            .arg("--network")
            .arg(if request.network_enabled {
                "bridge"
            } else {
                "none"
            })
            .arg("--cap-drop")
            .arg("ALL")
            .arg("--security-opt")
            .arg("no-new-privileges")
            .arg("--read-only")
            .arg("--pids-limit")
            .arg("256")
            .arg("--tmpfs")
            .arg("/tmp:rw,nosuid,nodev")
            .arg("-e")
            .arg("HOME=/tmp")
            .arg("-e")
            .arg("TMPDIR=/tmp")
            .arg("-v")
            .arg(format!(
                "{}:{DOCKER_WORKSPACE_ROOT}:rw",
                workspace_root.display()
            ))
            .arg("-w")
            .arg(&container_workdir)
            .arg(DOCKER_SANDBOX_IMAGE);

        append_container_action(&mut process, request, &workspace_root);

        Ok(SandboxCommandLaunch {
            process,
            metadata: SandboxExecutionMetadata {
                engine: self.kind(),
                network_enabled: request.network_enabled,
                workspace_root: display_path(&workspace_root),
                working_directory: display_path(&working_directory),
                container_workspace_root: Some(DOCKER_WORKSPACE_ROOT.to_string()),
                profile_path: None,
            },
        })
    }
}

struct MacosSandboxExecEngine;

impl SandboxEngine for MacosSandboxExecEngine {
    fn kind(&self) -> SandboxEngineKind {
        SandboxEngineKind::MacosSandboxExec
    }

    fn status(&self) -> SandboxDriverStatus {
        if !cfg!(target_os = "macos") {
            return SandboxDriverStatus {
                engine: self.kind(),
                available: false,
                reason: Some("sandbox-exec is only supported on macOS".to_string()),
            };
        }

        let executable = executable_in_path("sandbox-exec").or_else(|| {
            Path::new("/usr/bin/sandbox-exec")
                .is_file()
                .then(|| PathBuf::from("/usr/bin/sandbox-exec"))
        });
        let Some(executable) = executable else {
            return SandboxDriverStatus {
                engine: self.kind(),
                available: false,
                reason: Some("sandbox-exec executable was not found".to_string()),
            };
        };

        let executable = executable.to_string_lossy().to_string();
        if !command_succeeds_with_timeout(
            &executable,
            &["-p", "(version 1)\n(allow default)", "/usr/bin/true"],
            DRIVER_PROBE_TIMEOUT_MS,
        ) {
            return SandboxDriverStatus {
                engine: self.kind(),
                available: false,
                reason: Some(
                    "sandbox-exec is installed, but sandbox_apply is not permitted in this process context"
                        .to_string(),
                ),
            };
        }

        SandboxDriverStatus {
            engine: self.kind(),
            available: true,
            reason: None,
        }
    }

    fn build_command(
        &self,
        request: &SandboxCommandRequest,
    ) -> Result<SandboxCommandLaunch, String> {
        let workspace_root = canonicalize_existing_dir(&request.workspace_root)?;
        let working_directory = canonicalize_existing_dir(&request.working_directory)?;
        ensure_path_inside(&working_directory, &workspace_root)?;
        let sandbox_home = workspace_root.join(".oomu_sandbox_home");
        fs::create_dir_all(&sandbox_home).map_err(|error| {
            format!(
                "Unable to create sandbox home directory {}: {error}",
                sandbox_home.display()
            )
        })?;
        let profile_path =
            write_macos_sandbox_profile(&workspace_root, request.native_executable.as_deref())?;

        let executable = executable_in_path("sandbox-exec")
            .unwrap_or_else(|| PathBuf::from("/usr/bin/sandbox-exec"));
        let mut process = Command::new(executable);
        process
            .arg("-f")
            .arg(&profile_path)
            .env("HOME", &sandbox_home)
            .env("CARGO_HOME", sandbox_home.join(".cargo"))
            .env("RUSTUP_HOME", sandbox_home.join(".rustup"))
            .env("NPM_CONFIG_CACHE", sandbox_home.join(".npm"))
            .env("npm_config_cache", sandbox_home.join(".npm"))
            .env("TMPDIR", std::env::temp_dir());
        append_native_action(&mut process, request);

        Ok(SandboxCommandLaunch {
            process,
            metadata: SandboxExecutionMetadata {
                engine: self.kind(),
                network_enabled: request.network_enabled,
                workspace_root: display_path(&workspace_root),
                working_directory: display_path(&working_directory),
                container_workspace_root: None,
                profile_path: Some(display_path(&profile_path)),
            },
        })
    }
}

#[derive(Debug, Clone)]
pub struct SandboxRoot {
    root: PathBuf,
    real_root: PathBuf,
}

impl SandboxRoot {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|error| {
            format!(
                "Unable to create sandbox directory at {}: {error}",
                root.display()
            )
        })?;
        let root = absolute_path(root)?;
        let real_root = fs::canonicalize(&root).map_err(|error| {
            format!(
                "Unable to resolve sandbox directory at {}: {error}",
                root.display()
            )
        })?;
        Ok(Self { root, real_root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn real_root(&self) -> &Path {
        &self.real_root
    }

    pub fn resolve(&self, raw_path: impl AsRef<Path>) -> Result<PathBuf, String> {
        resolve_sandbox_path(&self.root, &self.real_root, raw_path.as_ref())
    }

    pub fn relative_path(&self, path: &Path) -> String {
        sandbox_relative_path(&self.real_root, path)
    }
}

fn append_container_action(
    process: &mut Command,
    request: &SandboxCommandRequest,
    workspace_root: &Path,
) {
    match request.kind {
        SandboxCommandKind::Shell => {
            process
                .arg("/bin/bash")
                .arg("-lc")
                .arg(map_shell_command_to_container(
                    &request.command,
                    workspace_root,
                ));
        }
        SandboxCommandKind::Python => {
            process
                .arg("python3")
                .arg(map_arg_to_container(&request.command, workspace_root));
            process.args(
                request
                    .args
                    .iter()
                    .map(|arg| map_arg_to_container(arg, workspace_root)),
            );
        }
        SandboxCommandKind::Binary => {
            process.arg(map_arg_to_container(&request.command, workspace_root));
            process.args(
                request
                    .args
                    .iter()
                    .map(|arg| map_arg_to_container(arg, workspace_root)),
            );
        }
    }
}

fn append_native_action(process: &mut Command, request: &SandboxCommandRequest) {
    match request.kind {
        SandboxCommandKind::Shell => {
            process.arg("/bin/bash").arg("-c").arg(&request.command);
        }
        SandboxCommandKind::Python => {
            process
                .arg(
                    request
                        .native_executable
                        .as_deref()
                        .unwrap_or_else(|| Path::new("python3")),
                )
                .arg(&request.command)
                .args(&request.args);
        }
        SandboxCommandKind::Binary => {
            process.arg(&request.command).args(&request.args);
        }
    }
}

fn map_shell_command_to_container(command: &str, workspace_root: &Path) -> String {
    let workspace = workspace_root.to_string_lossy();
    command.replace(workspace.as_ref(), DOCKER_WORKSPACE_ROOT)
}

fn map_arg_to_container(value: &str, workspace_root: &Path) -> String {
    let path = Path::new(value);
    if !path.is_absolute() {
        return value.to_string();
    }

    let candidate = resolve_existing_path_prefix(path).unwrap_or_else(|_| path.to_path_buf());
    if !path_has_case_aware_prefix(&candidate, workspace_root) {
        return value.to_string();
    }

    candidate
        .strip_prefix(workspace_root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(|relative| format!("{DOCKER_WORKSPACE_ROOT}/{}", display_path(relative)))
        .unwrap_or_else(|| DOCKER_WORKSPACE_ROOT.to_string())
}

fn container_path_for_host_path(path: &Path, workspace_root: &Path) -> Result<String, String> {
    ensure_path_inside(path, workspace_root)?;
    Ok(path
        .strip_prefix(workspace_root)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(|relative| format!("{DOCKER_WORKSPACE_ROOT}/{}", display_path(relative)))
        .unwrap_or_else(|| DOCKER_WORKSPACE_ROOT.to_string()))
}

fn canonicalize_existing_dir(path: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(path).map_err(|error| {
        format!(
            "Unable to create sandbox directory {}: {error}",
            path.display()
        )
    })?;
    fs::canonicalize(path).map_err(|error| {
        format!(
            "Unable to resolve sandbox directory {}: {error}",
            path.display()
        )
    })
}

fn ensure_path_inside(path: &Path, workspace_root: &Path) -> Result<(), String> {
    if path_has_case_aware_prefix(path, workspace_root) {
        Ok(())
    } else {
        Err(format!(
            "Sandbox working directory {} escapes workspace {}.",
            path.display(),
            workspace_root.display()
        ))
    }
}

fn executable_in_path(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.is_absolute() && candidate.is_file() {
        return Some(candidate.to_path_buf());
    }

    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
}

fn command_succeeds_with_timeout(command: &str, args: &[&str], timeout_ms: u64) -> bool {
    let mut child = match Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {}
            Err(_) => return false,
        }

        if started.elapsed() >= Duration::from_millis(timeout_ms) {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }

        thread::sleep(Duration::from_millis(25));
    }
}

fn write_macos_sandbox_profile(
    workspace_root: &Path,
    native_executable: Option<&Path>,
) -> Result<PathBuf, String> {
    let profile_dir = std::env::temp_dir().join("oomu_sandbox_profiles");
    fs::create_dir_all(&profile_dir).map_err(|error| {
        format!(
            "Unable to create macOS sandbox profile directory {}: {error}",
            profile_dir.display()
        )
    })?;
    let suffix = unix_time_ns_u128();
    let profile_path =
        profile_dir.join(format!("oomu_sandbox_{}_{}.sb", std::process::id(), suffix));
    let profile = macos_sandbox_profile(workspace_root, native_executable);
    fs::write(&profile_path, profile).map_err(|error| {
        format!(
            "Unable to write macOS sandbox profile {}: {error}",
            profile_path.display()
        )
    })?;
    Ok(profile_path)
}

fn macos_sandbox_profile(workspace_root: &Path, native_executable: Option<&Path>) -> String {
    let workspace = sbpl_string(&display_path(workspace_root));
    let temp = sbpl_string(&display_path(&std::env::temp_dir()));
    let native_runtime = native_executable
        .and_then(|path| fs::canonicalize(path).ok())
        .and_then(|path| path.parent()?.parent().map(Path::to_path_buf))
        .map(|path| format!("  (subpath {})\n", sbpl_string(&display_path(&path))))
        .unwrap_or_default();
    format!(
        r#"(version 1)
(deny default)
(allow process*)
(allow signal (target self))
(allow sysctl-read)
(deny network*)
(allow file-read-metadata)
(allow file-read*
  (literal "/")
  (literal "/dev/null")
  (literal "/dev/random")
  (literal "/dev/urandom")
  (subpath "/bin")
  (subpath "/sbin")
  (subpath "/usr")
  (subpath "/System")
  (subpath "/System/Library/Frameworks")
  (subpath "/Library/Apple")
  (subpath "/Library/Developer/CommandLineTools")
  (subpath "/Library/Frameworks")
  (subpath "/Applications/Xcode.app")
  (subpath "/opt/homebrew")
  (subpath "/usr/local")
  (subpath "/private/var/db/dyld")
  (subpath "/tmp")
  (subpath "/private/tmp")
  (subpath {temp})
{native_runtime}
  (subpath {workspace}))
(allow file-map-executable
  (subpath "/bin")
  (subpath "/sbin")
  (subpath "/usr")
  (subpath "/System")
  (subpath "/Library/Apple")
  (subpath "/Library/Developer/CommandLineTools")
  (subpath "/Applications/Xcode.app")
  (subpath "/opt/homebrew")
  (subpath "/usr/local")
{native_runtime}
  (subpath {workspace}))
(allow file-write*
  (subpath "/tmp")
  (subpath "/private/tmp")
  (subpath {temp})
  (subpath {workspace}))
"#
    )
}

fn sbpl_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

pub fn resolve_sandbox_path(
    sandbox_root: &Path,
    sandbox_real: &Path,
    raw_path: &Path,
) -> Result<PathBuf, String> {
    let candidate = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        sandbox_root.join(raw_path)
    };
    let candidate = absolute_path(candidate)?;
    let candidate_real = resolve_existing_path_prefix(&candidate)?;

    if !path_has_case_aware_prefix(&candidate_real, sandbox_real) {
        return Err("Path escapes the local sandbox.".to_string());
    }

    Ok(candidate_real)
}

pub fn sandbox_relative_path(sandbox_real: &Path, path: &Path) -> String {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    path.strip_prefix(sandbox_real)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(display_path)
        .unwrap_or_default()
}

pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn absolute_path(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|error| format!("Unable to resolve current directory: {error}"))
    }
}

fn resolve_existing_path_prefix(candidate: &Path) -> Result<PathBuf, String> {
    if let Ok(real) = fs::canonicalize(candidate) {
        return Ok(real);
    }

    let mut ancestor = candidate;
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or_else(|| {
            format!(
                "Unable to resolve sandbox path {} because no parent exists.",
                candidate.display()
            )
        })?;
    }

    let mut resolved = fs::canonicalize(ancestor).map_err(|error| {
        format!(
            "Unable to resolve existing sandbox path prefix {}: {error}",
            ancestor.display()
        )
    })?;
    let remainder = candidate
        .strip_prefix(ancestor)
        .unwrap_or_else(|_| Path::new(""));
    for component in remainder.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(part) => resolved.push(part),
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Ok(resolved)
}

fn path_has_case_aware_prefix(path: &Path, prefix: &Path) -> bool {
    let path = comparable_components(path);
    let prefix = comparable_components(prefix);
    path.len() >= prefix.len()
        && path
            .iter()
            .zip(prefix.iter())
            .all(|(left, right)| left == right)
}

fn comparable_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| comparison_key(&component.as_os_str().to_string_lossy()))
        .collect()
}

fn comparison_key(value: &str) -> String {
    if cfg!(any(target_os = "macos", windows)) {
        value.to_lowercase()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resolves_existing_and_future_paths_inside_sandbox() {
        let root = temp_root("oomu-sandbox-resolve");
        let sandbox = SandboxRoot::new(root.clone()).expect("sandbox initializes");
        let target = sandbox
            .resolve(Path::new("reports/out.txt"))
            .expect("future child path resolves");

        assert!(target.ends_with("reports/out.txt"));
        assert!(target.starts_with(sandbox.real_root()));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_absolute_escape() {
        let root = temp_root("oomu-sandbox-escape");
        let sandbox = SandboxRoot::new(root.clone()).expect("sandbox initializes");

        let escaped = sandbox
            .resolve(Path::new("/private/etc/hosts"))
            .expect_err("outside path is rejected");
        assert_eq!(escaped, "Path escapes the local sandbox.");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_symlink_escape() {
        let root = temp_root("oomu-sandbox-link");
        let outside = temp_root("oomu-sandbox-outside");
        fs::create_dir_all(&outside).expect("outside directory creates");
        fs::write(outside.join("hosts"), "outside").expect("outside file writes");
        let sandbox = SandboxRoot::new(root.clone()).expect("sandbox initializes");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("etc-link")).expect("symlink creates");
            let escaped = sandbox
                .resolve(Path::new("etc-link/hosts"))
                .expect_err("symlink escape is rejected");
            assert_eq!(escaped, "Path escapes the local sandbox.");
        }

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn accepts_case_only_sandbox_path_differences() {
        let root = temp_root("OOMU-Sandbox-Case");
        fs::create_dir_all(&root).expect("root creates");
        fs::write(root.join("Instruction_Input.txt"), "case-safe").expect("file writes");
        let server_root = PathBuf::from(root.to_string_lossy().to_lowercase());
        let sandbox = SandboxRoot::new(server_root).expect("sandbox initializes");
        let case_variant = root.join("instruction_input.txt");

        let resolved = sandbox
            .resolve(&case_variant)
            .expect("case-only absolute path is accepted");
        assert!(resolved.starts_with(sandbox.real_root()));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn docker_driver_builds_networkless_workspace_mount() {
        let root = temp_root("oomu-docker-sandbox");
        let working_directory = root.join("work");
        fs::create_dir_all(&working_directory).expect("working directory creates");
        let request = SandboxCommandRequest {
            kind: SandboxCommandKind::Shell,
            command: "echo ok".to_string(),
            args: Vec::new(),
            native_executable: None,
            workspace_root: root.clone(),
            working_directory: working_directory.clone(),
            network_enabled: false,
        };

        let launch = DockerSandboxEngine
            .build_command(&request)
            .expect("docker command is constructed");
        let args = launch
            .process
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--network" && pair[1] == "none"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-v" && pair[1].ends_with(":/sandbox:rw")));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-w" && pair[1] == "/sandbox/work"));
        assert_eq!(launch.metadata.engine, SandboxEngineKind::Docker);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn macos_profile_denies_network_and_allows_workspace() {
        let root = PathBuf::from("/tmp/oomu-sandbox-profile-test");
        let profile = macos_sandbox_profile(&root, None);

        assert!(profile.contains("(deny network*)"));
        assert!(profile.contains(r#"(subpath "/tmp/oomu-sandbox-profile-test")"#));
        assert!(profile.contains(r#"(subpath "/private/tmp")"#));
    }

    fn temp_root(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or_default()
        ))
    }
}
