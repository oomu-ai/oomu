use crate::foundation::digest::sha256_hex;
use crate::pdf_protocol::{
    known_failure_code, PdfExtractionLimits, PdfExtractionRequestHeader, PdfExtractionResponse,
    PdfExtractionResult, MAX_REQUEST_HEADER_BYTES, MAX_RESPONSE_BYTES, PDF_HELPER_PROTOCOL_VERSION,
    PDF_PARSER_NAME, PDF_PARSER_VERSION,
};
use rand_core::{OsRng, RngCore};
use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainedPdfExtraction {
    pub page_count: usize,
    pub text: String,
    pub truncated: bool,
    pub wall_time_ms: u64,
    pub cpu_time_ms: u64,
    pub peak_memory_bytes: u64,
    pub object_count: usize,
    pub decompressed_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfContainmentError {
    pub code: &'static str,
    pub message: String,
    pub wall_time_ms: Option<u64>,
    pub cpu_time_ms: Option<u64>,
    pub peak_memory_bytes: Option<u64>,
    pub limit_triggered: Option<String>,
}

impl PdfContainmentError {
    fn scoped(code: &'static str) -> Self {
        Self {
            code,
            message: format!(
                "This document could not be safely processed ({code}). No document content was accepted."
            ),
            wall_time_ms: None,
            cpu_time_ms: None,
            peak_memory_bytes: None,
            limit_triggered: None,
        }
    }

    fn from_helper(code: &'static str, metrics: crate::pdf_protocol::PdfExtractionMetrics) -> Self {
        Self {
            code,
            message: format!(
                "This document could not be safely processed ({code}). No document content was accepted."
            ),
            wall_time_ms: Some(metrics.wall_time_ms),
            cpu_time_ms: Some(metrics.cpu_time_ms),
            peak_memory_bytes: Some(metrics.peak_memory_bytes),
            limit_triggered: metrics.limit_triggered,
        }
    }
}

impl std::fmt::Display for PdfContainmentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PdfContainmentError {}

pub fn extract_pdf_from_open_file(
    file: fs::File,
) -> Result<ContainedPdfExtraction, PdfContainmentError> {
    let limits = PdfExtractionLimits::secure_default();
    if !limits.is_exact_secure_profile() {
        return Err(PdfContainmentError::scoped("pdf_limit_profile_invalid"));
    }
    let mut input = Zeroizing::new(Vec::new());
    file.take(limits.input_bytes.saturating_add(1))
        .read_to_end(&mut input)
        .map_err(|_| PdfContainmentError::scoped("pdf_input_read_failed"))?;
    if input.len() as u64 > limits.input_bytes {
        return Err(PdfContainmentError::scoped("pdf_input_limit_exceeded"));
    }
    extract_pdf_bytes_contained(input.as_slice())
}

pub fn extract_pdf_bytes_contained(
    input: &[u8],
) -> Result<ContainedPdfExtraction, PdfContainmentError> {
    let helper = resolve_pdf_helper_path()
        .ok_or_else(|| PdfContainmentError::scoped("pdf_helper_unavailable"))?;
    extract_pdf_bytes_with_helper(input, &helper)
}

pub fn extract_pdf_bytes_with_helper(
    input: &[u8],
    helper_path: &Path,
) -> Result<ContainedPdfExtraction, PdfContainmentError> {
    extract_pdf_bytes_with_helper_inner(input, helper_path, None)
}

pub fn extract_pdf_bytes_with_helper_and_cancellation(
    input: &[u8],
    helper_path: &Path,
    cancelled: &AtomicBool,
) -> Result<ContainedPdfExtraction, PdfContainmentError> {
    extract_pdf_bytes_with_helper_inner(input, helper_path, Some(cancelled))
}

fn extract_pdf_bytes_with_helper_inner(
    input: &[u8],
    helper_path: &Path,
    cancelled: Option<&AtomicBool>,
) -> Result<ContainedPdfExtraction, PdfContainmentError> {
    let limits = PdfExtractionLimits::secure_default();
    if input.is_empty() {
        return Err(PdfContainmentError::scoped("pdf_input_empty"));
    }
    if input.len() as u64 > limits.input_bytes {
        return Err(PdfContainmentError::scoped("pdf_input_limit_exceeded"));
    }
    validate_helper_path(helper_path)?;

    let mut request_random = [0_u8; 32];
    OsRng.fill_bytes(&mut request_random);
    let request_id = hex::encode(request_random);
    let request = PdfExtractionRequestHeader {
        protocol_version: PDF_HELPER_PROTOCOL_VERSION,
        request_id: request_id.clone(),
        operation: "extract_text".to_string(),
        input_byte_count: input.len() as u64,
        input_sha256: sha256_hex(input),
        parent_pid: std::process::id(),
        limits,
    };
    let request_json = serde_json::to_vec(&request)
        .map_err(|_| PdfContainmentError::scoped("pdf_protocol_encode_failed"))?;
    if request_json.len() > MAX_REQUEST_HEADER_BYTES {
        return Err(PdfContainmentError::scoped("pdf_protocol_encode_failed"));
    }

    let mut command = Command::new(helper_path);
    command
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_child_resource_limits(&mut command, limits);
    let child = command
        .spawn()
        .map_err(|_| PdfContainmentError::scoped("pdf_helper_start_failed"))?;
    collect_helper_response(child, request_json, input, &request_id, limits, cancelled)
}

fn collect_helper_response(
    mut child: Child,
    request_json: Vec<u8>,
    input: &[u8],
    request_id: &str,
    limits: PdfExtractionLimits,
    cancelled: Option<&AtomicBool>,
) -> Result<ContainedPdfExtraction, PdfContainmentError> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| PdfContainmentError::scoped("pdf_helper_protocol_failed"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| PdfContainmentError::scoped("pdf_helper_protocol_failed"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| PdfContainmentError::scoped("pdf_helper_protocol_failed"))?;
    let mut guard = ChildGuard::new(child);

    let input = Zeroizing::new(input.to_vec());
    let input_writer = thread::spawn(move || write_framed_request(stdin, request_json, input));
    let stdout_reader = thread::spawn(move || read_bounded(stdout, MAX_RESPONSE_BYTES));
    let stderr_reader = thread::spawn(move || read_bounded(stderr, 8 * 1024));

    let started = Instant::now();
    let child_pid = guard.child.id();
    let status = loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            guard.kill_and_reap();
            let _ = input_writer.join();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(PdfContainmentError::scoped("pdf_helper_cancelled"));
        }
        if child_resident_bytes(child_pid)
            .is_some_and(|resident| resident > limits.address_space_bytes)
        {
            guard.kill_and_reap();
            let _ = input_writer.join();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(PdfContainmentError::scoped(
                "pdf_helper_memory_limit_exceeded",
            ));
        }
        match guard.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < Duration::from_millis(limits.wall_time_ms) => {
                thread::sleep(Duration::from_millis(5));
            }
            Ok(None) => {
                guard.kill_and_reap();
                let _ = input_writer.join();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(PdfContainmentError::scoped(
                    "pdf_helper_wall_limit_exceeded",
                ));
            }
            Err(_) => {
                guard.kill_and_reap();
                let _ = input_writer.join();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(PdfContainmentError::scoped("pdf_helper_wait_failed"));
            }
        }
    };
    guard.mark_reaped();

    let input_ok = input_writer.join().ok().and_then(Result::ok).is_some();
    let (stdout_bytes, stdout_overflow) =
        stdout_reader.join().unwrap_or_else(|_| (Vec::new(), true));
    let _ = stderr_reader.join();
    if !input_ok || stdout_overflow {
        return Err(PdfContainmentError::scoped("pdf_helper_protocol_failed"));
    }
    validate_response(status, &stdout_bytes, request_id, limits)
}

fn write_framed_request(
    mut stdin: impl Write,
    request_json: Vec<u8>,
    input: Zeroizing<Vec<u8>>,
) -> Result<(), std::io::Error> {
    stdin.write_all(&(request_json.len() as u32).to_be_bytes())?;
    stdin.write_all(&request_json)?;
    stdin.write_all(input.as_slice())?;
    stdin.flush()
}

fn read_bounded(mut reader: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut output = Vec::with_capacity(limit.min(16 * 1024));
    let mut overflow = false;
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let remaining = limit.saturating_sub(output.len());
                if count > remaining {
                    output.extend_from_slice(&buffer[..remaining]);
                    overflow = true;
                } else {
                    output.extend_from_slice(&buffer[..count]);
                }
            }
        }
    }
    (output, overflow)
}

fn validate_response(
    status: ExitStatus,
    output: &[u8],
    request_id: &str,
    limits: PdfExtractionLimits,
) -> Result<ContainedPdfExtraction, PdfContainmentError> {
    if !status.success() {
        return Err(PdfContainmentError::scoped("pdf_helper_terminated"));
    }
    let response: PdfExtractionResponse = serde_json::from_slice(output)
        .map_err(|_| PdfContainmentError::scoped("pdf_helper_protocol_failed"))?;
    if response.protocol_version != PDF_HELPER_PROTOCOL_VERSION
        || response.request_id != request_id
        || response.parser_name != PDF_PARSER_NAME
        || response.parser_version != PDF_PARSER_VERSION
    {
        return Err(PdfContainmentError::scoped("pdf_helper_protocol_failed"));
    }
    match (response.result, response.error) {
        (Some(result), None) => validate_success(result, response.metrics, limits),
        (None, Some(error)) => match known_failure_code(&error.code) {
            Some(code) => Err(PdfContainmentError::from_helper(code, response.metrics)),
            None => Err(PdfContainmentError::scoped("pdf_helper_protocol_failed")),
        },
        _ => Err(PdfContainmentError::scoped("pdf_helper_protocol_failed")),
    }
}

fn validate_success(
    result: PdfExtractionResult,
    metrics: crate::pdf_protocol::PdfExtractionMetrics,
    limits: PdfExtractionLimits,
) -> Result<ContainedPdfExtraction, PdfContainmentError> {
    if result.page_count > limits.page_count
        || result.page_count != metrics.page_count
        || result.text.len() > limits.output_text_bytes
        || metrics.wall_time_ms > limits.wall_time_ms
        || metrics.cpu_time_ms > limits.cpu_time_seconds.saturating_mul(1_000)
        || metrics.peak_memory_bytes > limits.address_space_bytes
        || metrics.input_bytes == 0
        || metrics.input_bytes > limits.input_bytes
        || metrics.object_count > limits.object_count
        || metrics.maximum_nesting_depth > limits.nesting_depth
        || metrics.decompressed_bytes > limits.decompressed_bytes
        || metrics.limit_triggered.is_some()
    {
        return Err(PdfContainmentError::scoped("pdf_helper_protocol_failed"));
    }
    Ok(ContainedPdfExtraction {
        page_count: result.page_count,
        text: result.text,
        truncated: result.truncated,
        wall_time_ms: metrics.wall_time_ms,
        cpu_time_ms: metrics.cpu_time_ms,
        peak_memory_bytes: metrics.peak_memory_bytes,
        object_count: metrics.object_count,
        decompressed_bytes: metrics.decompressed_bytes,
    })
}

fn validate_helper_path(path: &Path) -> Result<(), PdfContainmentError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| PdfContainmentError::scoped("pdf_helper_unavailable"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(PdfContainmentError::scoped("pdf_helper_unavailable"));
    }
    Ok(())
}

fn resolve_pdf_helper_path() -> Option<PathBuf> {
    let current_exe = env::current_exe().ok()?;
    let directory = current_exe.parent()?;
    let filename = if cfg!(target_os = "windows") {
        "pdf_extract_helper.exe"
    } else {
        "pdf_extract_helper"
    };
    let sibling = directory.join(filename);
    if sibling.is_file() {
        return Some(sibling);
    }
    #[cfg(debug_assertions)]
    {
        let debug_helper = Path::new(crate::runtime_profile::OOMU_MANIFEST_DIR)
            .join("target")
            .join("debug")
            .join(filename);
        if debug_helper.is_file() {
            return Some(debug_helper);
        }
    }
    None
}

#[cfg(unix)]
fn configure_child_resource_limits(command: &mut Command, limits: PdfExtractionLimits) {
    // SAFETY: this closure only invokes async-signal-safe setrlimit calls between
    // fork and exec; it does not allocate, lock, or access shared Rust state.
    unsafe {
        command.pre_exec(move || {
            set_limit(libc::RLIMIT_CORE, 0)?;
            set_limit(libc::RLIMIT_CPU, limits.cpu_time_seconds)?;
            #[cfg(not(target_os = "macos"))]
            set_limit(libc::RLIMIT_AS, limits.address_space_bytes)?;
            set_limit(libc::RLIMIT_FSIZE, MAX_RESPONSE_BYTES as u64)?;
            set_limit(libc::RLIMIT_NOFILE, 32)?;
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
    if unsafe { libc::setrlimit(resource as _, &limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn configure_child_resource_limits(_command: &mut Command, _limits: PdfExtractionLimits) {}

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

struct ChildGuard {
    child: Child,
    reaped: bool,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self {
            child,
            reaped: false,
        }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    fn kill_and_reap(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.reaped = true;
        }
    }

    fn mark_reaped(&mut self) {
        self.reaped = true;
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill_and_reap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_limits_are_finite_and_exact() {
        let limits = PdfExtractionLimits::secure_default();
        assert!(limits.is_exact_secure_profile());
        assert!(limits.wall_time_ms <= 5_000);
        assert!(limits.address_space_bytes <= 256 * 1024 * 1024);
        assert!(limits.output_text_bytes <= 128 * 1024);
    }

    #[test]
    fn oversized_input_is_rejected_before_process_launch() {
        let input = vec![0_u8; PdfExtractionLimits::secure_default().input_bytes as usize + 1];
        let error = extract_pdf_bytes_with_helper(&input, Path::new("/does/not/exist"))
            .expect_err("oversized input must fail before helper resolution");
        assert_eq!(error.code, "pdf_input_limit_exceeded");
    }
}
