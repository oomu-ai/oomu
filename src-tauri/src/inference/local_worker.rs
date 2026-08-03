use super::*;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct LocalInferCliError {
    pub(super) code: String,
    pub(super) message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct LocalInferToken {
    pub(super) sequence: usize,
    pub(super) token: String,
}

pub(super) enum LocalInferStderrRecord {
    Ready,
    Progress,
    Token(LocalInferToken),
    Error(LocalInferCliError),
    Log,
}

pub(super) struct LocalInferWorker {
    pub(super) model_id: String,
    pub(super) model_root: PathBuf,
    pub(super) child: Option<Child>,
    pub(super) stdin: Option<ChildStdin>,
    pub(super) stdout_receiver: mpsc::Receiver<String>,
    pub(super) stderr_receiver: mpsc::Receiver<String>,
    pub(super) stdout_reader: Option<JoinHandle<Result<String, String>>>,
    pub(super) stderr_reader: Option<JoinHandle<String>>,
    pub(super) last_used_at: Instant,
}

pub(super) static LOCAL_INFER_WORKER: OnceLock<Mutex<Option<LocalInferWorker>>> = OnceLock::new();
pub(super) static LOCAL_INFER_REAPER: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();
pub(super) static LOCAL_INFER_IDLE_REAPER: OnceLock<JoinHandle<()>> = OnceLock::new();
static CANCELLED_LOCAL_STREAMS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static LOCAL_GENERATION_HEALTH: OnceLock<Mutex<LocalGenerationHealth>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalGenerationStatus {
    Cold,
    Loading,
    Ready,
    Degraded,
    Shutdown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalGenerationHealth {
    pub model_id: Option<String>,
    pub status: LocalGenerationStatus,
    pub last_verified_at_ms: Option<i64>,
    pub last_error_code: Option<String>,
}

impl Default for LocalGenerationHealth {
    fn default() -> Self {
        Self {
            model_id: None,
            status: LocalGenerationStatus::Cold,
            last_verified_at_ms: None,
            last_error_code: None,
        }
    }
}

pub(super) fn update_local_generation_health(
    model_id: Option<&str>,
    status: LocalGenerationStatus,
    error_code: Option<&str>,
) {
    let health =
        LOCAL_GENERATION_HEALTH.get_or_init(|| Mutex::new(LocalGenerationHealth::default()));
    if let Ok(mut health) = health.lock() {
        health.model_id = model_id.map(str::to_string);
        health.last_verified_at_ms = (status == LocalGenerationStatus::Ready)
            .then(crate::foundation::clock::unix_time_ms_i64);
        health.last_error_code = error_code.map(str::to_string);
        health.status = status;
    }
}

pub(super) fn get_local_generation_health(model_id: Option<String>) -> LocalGenerationHealth {
    let requested_model_id = model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let health = LOCAL_GENERATION_HEALTH
        .get_or_init(|| Mutex::new(LocalGenerationHealth::default()))
        .lock()
        .map(|health| health.clone())
        .unwrap_or_default();
    if requested_model_id.is_some()
        && requested_model_id != health.model_id.as_deref()
        && health.status != LocalGenerationStatus::Shutdown
    {
        return LocalGenerationHealth {
            model_id: requested_model_id.map(str::to_string),
            ..LocalGenerationHealth::default()
        };
    }
    health
}

fn cancelled_local_streams() -> &'static Mutex<HashSet<String>> {
    CANCELLED_LOCAL_STREAMS.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) fn is_local_stream_cancelled(stream_id: Option<&str>) -> bool {
    stream_id.is_some_and(|stream_id| {
        cancelled_local_streams()
            .lock()
            .is_ok_and(|streams| streams.contains(stream_id))
    })
}

pub(super) fn clear_local_stream_cancellation(stream_id: &str) {
    if let Ok(mut streams) = cancelled_local_streams().lock() {
        streams.remove(stream_id);
    }
}

pub(super) fn cancel_chat_stream(stream_id: String) -> bool {
    let stream_id = stream_id.trim();
    if stream_id.is_empty() {
        return false;
    }
    cancelled_local_streams()
        .lock()
        .map(|mut streams| streams.insert(stream_id.to_string()))
        .unwrap_or(false)
}

pub(super) fn reap_local_infer_child(
    mut child: Child,
    stdout_reader: Option<JoinHandle<Result<String, String>>>,
    stderr_reader: Option<JoinHandle<String>>,
) {
    let shutdown_started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if shutdown_started.elapsed() < LOCAL_INFER_SHUTDOWN_GRACE => {
                thread::sleep(Duration::from_millis(25));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    let _ = child.wait();
    if let Some(reader) = stdout_reader {
        let _ = reader.join();
    }
    if let Some(reader) = stderr_reader {
        let _ = reader.join();
    }
}

pub(super) fn wait_for_local_infer_cleanup() -> Result<(), InferenceError> {
    let reaper = LOCAL_INFER_REAPER.get_or_init(|| Mutex::new(None));
    let pending = reaper
        .lock()
        .map_err(|_| InferenceError::worker("Local inference cleanup lock was poisoned."))?
        .take();
    if let Some(pending) = pending {
        pending
            .join()
            .map_err(|_| InferenceError::worker("Local inference cleanup thread panicked."))?;
    }
    Ok(())
}

pub fn shutdown_local_inference_worker() {
    local_prewarm::shutdown();
    if let Some(worker) = LOCAL_INFER_WORKER.get() {
        if let Ok(mut worker) = worker.lock() {
            worker.take();
        }
    }
    if let Err(error) = wait_for_local_infer_cleanup() {
        eprintln!(
            "LOCAL_INFER_SHUTDOWN_CLEANUP_FAILED code={} message={}",
            error.code, error.message
        );
    }
    update_local_generation_health(None, LocalGenerationStatus::Shutdown, None);
}

pub fn prewarm_local_inference_worker(
    model_id: &str,
    local_model_directory: &Path,
) -> Result<(), InferenceError> {
    update_local_generation_health(Some(model_id), LocalGenerationStatus::Loading, None);
    let result = with_local_infer_worker(model_id, local_model_directory, None, |_| Ok(()));
    if let Err(error) = result.as_ref() {
        update_local_generation_health(
            Some(model_id),
            LocalGenerationStatus::Degraded,
            Some(&error.code),
        );
    }
    result
}

fn read_local_infer_stdout<R>(mut stream: R) -> JoinHandle<Result<String, String>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut output = String::new();
        stream
            .read_to_string(&mut output)
            .map(|_| output)
            .map_err(|error| format!("Failed to read local inference stdout: {error}"))
    })
}

pub(super) fn monitor_local_infer_stdout<R>(
    stream: R,
) -> (mpsc::Receiver<String>, JoinHandle<Result<String, String>>)
where
    R: Read + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut captured = Vec::new();
        for line in BufReader::new(stream).lines() {
            let line =
                line.map_err(|error| format!("Failed to read local inference stdout: {error}"))?;
            let _ = sender.send(line.clone());
            captured.push(line);
        }
        Ok(captured.join("\n"))
    });
    (receiver, reader)
}

fn join_local_infer_stdout(
    reader: JoinHandle<Result<String, String>>,
) -> Result<String, InferenceError> {
    reader
        .join()
        .map_err(|_| InferenceError::worker("Local inference stdout reader panicked."))?
        .map_err(InferenceError::worker)
}

pub(super) fn monitor_local_infer_stderr<R>(
    stream: R,
) -> (mpsc::Receiver<String>, JoinHandle<String>)
where
    R: Read + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut captured = Vec::new();
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else {
                break;
            };
            let _ = sender.send(line.clone());
            captured.push(line);
        }
        captured.join("\n")
    });
    (receiver, reader)
}

pub(super) fn parse_local_infer_stderr_record(line: &str) -> LocalInferStderrRecord {
    let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
        return LocalInferStderrRecord::Log;
    };
    if value.get("event").and_then(Value::as_str) == Some("ready") {
        return LocalInferStderrRecord::Ready;
    }
    if value.get("event").and_then(Value::as_str) == Some("progress") {
        return LocalInferStderrRecord::Progress;
    }
    if value.get("event").and_then(Value::as_str) == Some("token") {
        if let Ok(token) = serde_json::from_value::<LocalInferToken>(value.clone()) {
            if token.sequence > 0 {
                return LocalInferStderrRecord::Token(token);
            }
        }
    }
    match (
        value.get("code").and_then(Value::as_str),
        value.get("message").and_then(Value::as_str),
    ) {
        (Some(code), Some(message)) => LocalInferStderrRecord::Error(LocalInferCliError {
            code: code.to_string(),
            message: message.to_string(),
        }),
        _ => LocalInferStderrRecord::Log,
    }
}

pub(super) fn local_infer_error_payload(stderr: &str) -> Option<LocalInferCliError> {
    stderr
        .lines()
        .rev()
        .find_map(|line| match parse_local_infer_stderr_record(line) {
            LocalInferStderrRecord::Error(error) => Some(error),
            LocalInferStderrRecord::Ready
            | LocalInferStderrRecord::Progress
            | LocalInferStderrRecord::Token(_)
            | LocalInferStderrRecord::Log => None,
        })
}

pub(super) fn local_infer_error(
    stderr: &str,
    observed_error: Option<LocalInferCliError>,
) -> InferenceError {
    if let Some(error) = observed_error.or_else(|| local_infer_error_payload(stderr)) {
        return InferenceError::local_infer(error.code, error.message);
    }
    let message = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or("Local inference helper failed without an error message.");
    InferenceError::local_infer("local_infer_failed", message)
}

pub(super) fn verify_local_infer_protocol(
    helper_path: &Path,
    startup_cancellation: Option<&AtomicBool>,
    stream_id: Option<&str>,
) -> Result<(), InferenceError> {
    let mut child = Command::new(helper_path)
        .arg("--protocol-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            InferenceError::worker(format!(
                "Failed to query local inference helper version at {}: {error}",
                helper_path.display()
            ))
        })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        InferenceError::worker("Local inference protocol stdout was unavailable.")
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        InferenceError::worker("Local inference protocol stderr was unavailable.")
    })?;
    let stdout_reader = read_local_infer_stdout(stdout);
    let (_, stderr_reader) = monitor_local_infer_stderr(stderr);
    let started = Instant::now();

    loop {
        if let Err(error) =
            local_cancellation::ensure_startup_active(startup_cancellation, stream_id)
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_local_infer_stdout(stdout_reader);
            let _ = stderr_reader.join();
            return Err(error);
        }
        match child.try_wait().map_err(|error| {
            InferenceError::worker(format!("Failed to poll local inference protocol: {error}"))
        })? {
            Some(status) => {
                let actual = join_local_infer_stdout(stdout_reader)?.trim().to_string();
                let stderr = stderr_reader.join().unwrap_or_default();
                if !status.success() {
                    return Err(local_infer_error(&stderr, None));
                }
                validate_local_infer_protocol_version(&actual)?;
                return Ok(());
            }
            None if started.elapsed() >= Duration::from_secs(5) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_local_infer_stdout(stdout_reader);
                let _ = stderr_reader.join();
                return Err(InferenceError::local_infer(
                    "local_infer_protocol_timeout",
                    "Local inference helper did not answer its health check within 5 seconds. Rebuild or reinstall OOMU.",
                ));
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    }
}

pub(super) fn validate_local_infer_protocol_version(actual: &str) -> Result<(), InferenceError> {
    let expected = LOCAL_INFER_PROTOCOL_VERSION.to_string();
    if actual.trim() != expected {
        return Err(InferenceError::worker(format!(
            "Local inference helper protocol mismatch: app requires {expected}, helper reports {}. Rebuild or reinstall OOMU.",
            actual.trim()
        )));
    }
    Ok(())
}

pub(super) fn local_inference_timeout() -> Duration {
    let seconds = env::var(LOCAL_INFERENCE_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_LOCAL_INFERENCE_TIMEOUT_SECONDS)
        .clamp(1, 60 * 60);
    Duration::from_secs(seconds)
}

pub(super) fn local_model_idle_timeout() -> Duration {
    parse_local_model_idle_timeout(env::var(LOCAL_MODEL_IDLE_TIMEOUT_ENV).ok().as_deref())
}

pub(super) fn parse_local_model_idle_timeout(value: Option<&str>) -> Duration {
    let seconds = value
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_LOCAL_MODEL_IDLE_SECONDS)
        .clamp(5, 24 * 60 * 60);
    Duration::from_secs(seconds)
}

pub(super) fn local_infer_helper_path() -> Result<PathBuf, InferenceError> {
    let current_exe = env::current_exe().map_err(|error| {
        InferenceError::worker(format!("Unable to locate OOMU executable: {error}"))
    })?;
    let helper_name = if cfg!(windows) {
        "local_infer.exe"
    } else {
        "local_infer"
    };
    let helper_path = current_exe
        .parent()
        .map(|directory| directory.join(helper_name))
        .ok_or_else(|| InferenceError::worker("Unable to resolve local inference helper path."))?;
    if !helper_path.exists() {
        return Err(InferenceError::worker(format!(
            "Local inference helper is missing at {}. Build the local_infer binary before using local model chat.",
            helper_path.display()
        )));
    }
    Ok(helper_path)
}
