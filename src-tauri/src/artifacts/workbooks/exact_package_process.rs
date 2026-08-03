use std::{
    ffi::OsString,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};

const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_RESIDENT_BYTES: u64 = 1024 * 1024 * 1024;

pub(super) struct BoundedOutput {
    pub(super) status: ExitStatus,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

#[derive(Clone)]
struct DetachedProcessScope {
    executable: PathBuf,
    required_argument: OsString,
}

pub(super) fn run_qualified_conversion(
    executable: &Path,
    application_brand: &str,
    executable_name: &str,
    profile_argument: &OsString,
    converter_args: &[OsString],
    envs: &[(OsString, OsString)],
    current_dir: Option<&Path>,
    timeout: Duration,
    output_limit: u64,
) -> Result<BoundedOutput, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = (application_brand, executable_name);
        let scope = DetachedProcessScope {
            executable: executable.to_path_buf(),
            required_argument: profile_argument.clone(),
        };
        return run_bounded_inner(
            executable,
            converter_args,
            envs,
            current_dir,
            timeout,
            output_limit,
            Some(scope),
            true,
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (application_brand, executable_name, profile_argument);
        run_bounded_inner(
            executable,
            converter_args,
            envs,
            current_dir,
            timeout,
            output_limit,
            None,
            false,
        )
    }
}

pub(super) fn run_bounded(
    program: &Path,
    args: &[OsString],
    envs: &[(OsString, OsString)],
    current_dir: Option<&Path>,
    timeout: Duration,
    output_limit: u64,
) -> Result<BoundedOutput, String> {
    run_bounded_inner(
        program,
        args,
        envs,
        current_dir,
        timeout,
        output_limit,
        None,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_bounded_inner(
    program: &Path,
    args: &[OsString],
    envs: &[(OsString, OsString)],
    current_dir: Option<&Path>,
    timeout: Duration,
    output_limit: u64,
    scope: Option<DetachedProcessScope>,
    scope_observed: bool,
) -> Result<BoundedOutput, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(envs.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(directory) = current_dir {
        command.current_dir(directory);
    }
    configure_limits(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Qualified workbook rendering failed to start: {error}"))?;
    let child_pid = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Workbook rendering stdout is unavailable.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Workbook rendering stderr is unavailable.".to_string())?;
    let out_thread = bounded_reader(stdout, output_limit);
    let err_thread = bounded_reader(stderr, output_limit);
    let mut scope_monitor = ScopedProcessMonitor::new(scope, scope_observed);
    let started = Instant::now();
    let status = loop {
        let detached_resident = match scope_monitor.poll_resident_bytes() {
            Ok(value) => value,
            Err(error) => {
                let reason = format!("Qualified workbook process containment failed: {error}");
                return Err(stop_after_limit(&mut child, &scope_monitor, &reason));
            }
        };
        if child_resident_bytes(child_pid).is_some_and(|bytes| bytes > MAX_RESIDENT_BYTES)
            || detached_resident.is_some_and(|bytes| bytes > MAX_RESIDENT_BYTES)
        {
            return Err(stop_after_limit(
                &mut child,
                &scope_monitor,
                "Qualified workbook rendering exceeded its memory limit.",
            ));
        }
        let child_status = match child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                let reason = format!("Qualified workbook process monitoring failed: {error}");
                return Err(stop_after_limit(&mut child, &scope_monitor, &reason));
            }
        };
        if let Some(status) = child_status {
            break status;
        }
        if started.elapsed() >= timeout {
            return Err(stop_after_limit(
                &mut child,
                &scope_monitor,
                "Qualified workbook rendering exceeded its time limit.",
            ));
        }
        thread::sleep(Duration::from_millis(20));
    };
    if !status.success() {
        scope_monitor.terminate()?;
    } else {
        scope_monitor.require_completed(timeout.saturating_sub(started.elapsed()))?;
    }
    let stdout = join_reader(out_thread, "stdout")?;
    let stderr = join_reader(err_thread, "stderr")?;
    if stdout.len() > output_limit as usize || stderr.len() > output_limit as usize {
        return Err("Qualified workbook rendering exceeded its output limit.".to_string());
    }
    Ok(BoundedOutput {
        status,
        stdout,
        stderr,
    })
}

fn bounded_reader(
    stream: impl Read + Send + 'static,
    output_limit: u64,
) -> thread::JoinHandle<std::io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut data = Vec::new();
        stream
            .take(output_limit + 1)
            .read_to_end(&mut data)
            .map(|_| data)
    })
}

fn join_reader(
    reader: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    name: &str,
) -> Result<Vec<u8>, String> {
    reader
        .join()
        .map_err(|_| format!("Workbook rendering {name} reader failed."))?
        .map_err(|error| error.to_string())
}

fn stop_after_limit(
    child: &mut std::process::Child,
    monitor: &ScopedProcessMonitor,
    reason: &str,
) -> String {
    let cleanup = monitor.terminate();
    terminate_process(child);
    cleanup.map_or_else(
        |error| format!("{reason} Exact scoped cleanup failed: {error}"),
        |_| reason.to_string(),
    )
}

struct ScopedProcessMonitor {
    scope: Option<DetachedProcessScope>,
    observed: bool,
    last_poll: Option<Instant>,
}

impl ScopedProcessMonitor {
    fn new(scope: Option<DetachedProcessScope>, scope_observed: bool) -> Self {
        Self {
            observed: scope.is_none() || scope_observed,
            scope,
            last_poll: None,
        }
    }

    fn poll_resident_bytes(&mut self) -> Result<Option<u64>, String> {
        let Some(scope) = &self.scope else {
            return Ok(None);
        };
        if self
            .last_poll
            .is_some_and(|instant| instant.elapsed() < Duration::from_millis(100))
        {
            return Ok(None);
        }
        self.last_poll = Some(Instant::now());
        let processes = scoped_processes(scope)?;
        self.observed |= !processes.is_empty();
        Ok(processes
            .into_iter()
            .map(|process| process.resident_bytes)
            .max())
    }

    fn require_completed(&mut self, timeout: Duration) -> Result<(), String> {
        let Some(scope) = &self.scope else {
            return Ok(());
        };
        let started = Instant::now();
        loop {
            let remaining = scoped_processes(scope)?;
            self.observed |= !remaining.is_empty();
            if !self.observed {
                return Err(
                    "The qualified workbook converter process could not be contained.".into(),
                );
            }
            if remaining.is_empty() {
                return Ok(());
            }
            if remaining
                .iter()
                .any(|process| process.resident_bytes > MAX_RESIDENT_BYTES)
            {
                self.terminate()?;
                return Err("Qualified workbook rendering exceeded its memory limit.".to_string());
            }
            if started.elapsed() >= timeout {
                self.terminate()?;
                return Err(
                    "The qualified workbook converter exceeded its time limit and was stopped."
                        .to_string(),
                );
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn terminate(&self) -> Result<(), String> {
        self.scope
            .as_ref()
            .map_or(Ok(()), terminate_scoped_processes)
    }
}

struct ScopedProcess {
    resident_bytes: u64,
}

#[cfg(target_os = "macos")]
fn scoped_processes(scope: &DetachedProcessScope) -> Result<Vec<ScopedProcess>, String> {
    let system = refreshed_process_system();
    Ok(system
        .processes()
        .iter()
        .filter(|(_, process)| process_matches_scope(process, scope))
        .map(|(_, process)| ScopedProcess {
            resident_bytes: process.memory(),
        })
        .collect())
}

#[cfg(not(target_os = "macos"))]
fn scoped_processes(_scope: &DetachedProcessScope) -> Result<Vec<ScopedProcess>, String> {
    Ok(Vec::new())
}

#[cfg(target_os = "macos")]
fn process_matches_scope(process: &sysinfo::Process, scope: &DetachedProcessScope) -> bool {
    process.exe().is_some_and(|path| {
        let executable = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        scope_identity_matches(&executable, process.cmd(), scope)
    })
}

#[cfg(target_os = "macos")]
fn refreshed_process_system() -> System {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        ProcessRefreshKind::new()
            .with_memory()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    system
}

#[cfg(target_os = "macos")]
fn terminate_scoped_processes(scope: &DetachedProcessScope) -> Result<(), String> {
    signal_scoped_processes(scope, Signal::Term)?;
    if wait_for_scoped_exit(scope, Duration::from_secs(2))? {
        return Ok(());
    }
    signal_scoped_processes(scope, Signal::Kill)?;
    if wait_for_scoped_exit(scope, Duration::from_secs(1))? {
        Ok(())
    } else {
        Err("the private-profile converter process remained alive".to_string())
    }
}

#[cfg(not(target_os = "macos"))]
fn terminate_scoped_processes(_scope: &DetachedProcessScope) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn signal_scoped_processes(scope: &DetachedProcessScope, signal: Signal) -> Result<(), String> {
    let system = refreshed_process_system();
    for process in system
        .processes()
        .values()
        .filter(|process| process_matches_scope(process, scope))
    {
        if process.kill_with(signal) != Some(true) {
            return Err("the private-profile converter process could not be signaled".to_string());
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn wait_for_scoped_exit(scope: &DetachedProcessScope, timeout: Duration) -> Result<bool, String> {
    let started = Instant::now();
    loop {
        if scoped_processes(scope)?.is_empty() {
            return Ok(true);
        }
        if started.elapsed() >= timeout {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

pub(super) fn bounded_message(bytes: &[u8]) -> String {
    let value = String::from_utf8_lossy(bytes)
        .chars()
        .take(500)
        .collect::<String>();
    if value.trim().is_empty() {
        "no diagnostic output".to_string()
    } else {
        value
    }
}

pub(super) fn exit_status_diagnostic(status: ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exit code {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }
    "an unknown process status".to_string()
}

#[cfg(unix)]
fn configure_limits(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            set_limit(libc::RLIMIT_CORE, 0)?;
            set_limit(libc::RLIMIT_CPU, 65)?;
            set_limit(libc::RLIMIT_FSIZE, MAX_FILE_BYTES)?;
            set_limit(libc::RLIMIT_NOFILE, 256)?;
            Ok(())
        });
    }
}

#[cfg(unix)]
fn set_limit(resource: libc::c_int, value: u64) -> std::io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value as libc::rlim_t,
        rlim_max: value as libc::rlim_t,
    };
    if unsafe { libc::setrlimit(resource as _, &limit) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn configure_limits(_command: &mut Command) {}

fn terminate_process(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(target_os = "macos")]
fn child_resident_bytes(pid: u32) -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    let status = unsafe {
        libc::proc_pid_rusage(
            pid as libc::c_int,
            libc::RUSAGE_INFO_V2,
            usage.as_mut_ptr() as *mut libc::rusage_info_t,
        )
    };
    (status == 0).then(|| unsafe { usage.assume_init() }.ri_phys_footprint)
}

#[cfg(not(target_os = "macos"))]
fn child_resident_bytes(_pid: u32) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn scope_requires_both_exact_executable_and_private_profile_argument() {
        let executable = PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS/soffice");
        let argument = OsString::from(
            "-env:UserInstallation=file:///private/tmp/oomu-private-profile/profile",
        );
        let scope = DetachedProcessScope {
            executable: executable.clone(),
            required_argument: argument.clone(),
        };
        assert!(scope_identity_matches(
            &executable,
            &[OsString::from("soffice"), argument.clone()],
            &scope,
        ));
        assert!(!scope_identity_matches(
            Path::new("/Applications/Other.app/Contents/MacOS/soffice"),
            &[OsString::from("soffice"), argument.clone()],
            &scope,
        ));
        assert!(!scope_identity_matches(
            &executable,
            &[
                OsString::from("soffice"),
                OsString::from("-env:UserInstallation=file:///another/profile"),
            ],
            &scope,
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn directly_spawned_scope_is_contained_from_process_start() {
        let monitor = ScopedProcessMonitor::new(
            Some(DetachedProcessScope {
                executable: PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS/soffice"),
                required_argument: OsString::from(
                    "-env:UserInstallation=file:///private/tmp/private-profile/",
                ),
            }),
            true,
        );
        assert!(monitor.observed);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn scoped_cleanup_terminates_only_the_exact_profile_process() {
        use std::os::unix::process::CommandExt;

        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let executable = fs::canonicalize("/bin/sleep").unwrap();
        let marker = OsString::from(format!(
            "-env:UserInstallation=file:///private/tmp/oomu-scope-cleanup-test-{}/profile",
            std::process::id()
        ));
        let mut target = ChildGuard(
            Command::new(&executable)
                .arg0(&marker)
                .arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let scope = DetachedProcessScope {
            executable: executable.clone(),
            required_argument: marker,
        };
        let found = (0..100).any(|_| {
            let found = !scoped_processes(&scope).unwrap().is_empty();
            if !found {
                thread::sleep(Duration::from_millis(20));
            }
            found
        });
        assert!(found);

        let unrelated_scope = DetachedProcessScope {
            executable,
            required_argument: OsString::from(
                "-env:UserInstallation=file:///private/tmp/a-different-profile",
            ),
        };
        terminate_scoped_processes(&unrelated_scope).unwrap();
        assert!(target.0.try_wait().unwrap().is_none());

        terminate_scoped_processes(&scope).unwrap();
        assert!(target.0.wait().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn failed_process_diagnostic_includes_terminating_signal() {
        use std::os::unix::process::ExitStatusExt;
        let status = ExitStatus::from_raw(libc::SIGABRT);
        assert_eq!(
            exit_status_diagnostic(status),
            format!("signal {}", libc::SIGABRT)
        );
    }
}

#[cfg(any(test, target_os = "macos"))]
fn scope_identity_matches(
    executable: &Path,
    arguments: &[OsString],
    scope: &DetachedProcessScope,
) -> bool {
    executable == scope.executable
        && arguments
            .iter()
            .any(|argument| argument == &scope.required_argument)
}
